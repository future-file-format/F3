#[cfg(test)]
pub mod tests {
    use arrow_data::ArrayData;
    use fff_encoding::{
        enc_unit::FlatEncUnit,
        schemes::{vortex::VortexDecoder, Decoder},
    };
    use fff_poc::decoder::encunit::EncUnitDecoder;
    use std::{
        collections::vec_deque,
        io::{BufWriter, Read, Seek, Write},
        sync::{Arc, Mutex},
    };
    use vortex_sampling_compressor::ALL_ENCODINGS_CONTEXT;

    use arrow_array::{ArrayRef, StringArray, UInt32Array};
    use arrow_buffer::{Buffer, MutableBuffer};
    use bytes::BytesMut;
    use fff_core::util::buffer_to_array::{
        primitive_array_from_arrow_buffers, primitive_array_from_arrow_buffers_iter,
    };
    use fff_encoding::schemes::vortex::VortexEncoder;
    use fff_encoding::schemes::{bp::BPEncoder, Encoder};
    use fff_poc::decoder::encunit::WASMEncUnitDecoder;
    use fff_ude_wasm::{Config, Runtime};

    fn encode(encoder: impl Encoder, arr: ArrayRef) -> BytesMut {
        let encunit = encoder.encode(arr).unwrap();
        let mut file = tempfile::tempfile().unwrap();
        {
            let mut writer = encunit.try_serialize(BufWriter::new(&file)).unwrap();
            writer.flush().unwrap();
        }
        file.rewind().unwrap();
        let mut buf = vec![];
        file.read_to_end(&mut buf).unwrap();
        let mut bytes = BytesMut::with_capacity(buf.len());
        bytes.extend_from_slice(&buf);
        bytes
    }

    /// Conclusion of this experiment:
    /// alloc, dealloc, and then alloc will reuse some of the freed memory but also increase a little.
    /// Reason to be investigated.
    #[test]
    fn test_wasm() {
        use fff_ude_wasm::Runtime;
        let vec_size = 64 * 1024;
        let vec: Vec<u32> = (1..=vec_size).map(|x| x % 128).collect();
        let arr = UInt32Array::from(vec);
        let arr = Arc::new(arr) as ArrayRef;
        {
            let enc = BPEncoder;
            let bytes = encode(enc, arr.clone()).freeze();
            let filename = fff_test_util::BP_WASM_PATH.as_path();
            let rt = Runtime::try_new(&std::fs::read(filename).unwrap()).unwrap();
            let mut outputs = vec_deque::VecDeque::new();
            let iterations = 10000;
            for _ in 0..iterations {
                // This closure contains the code to be measured
                let output = rt
                    .call_single_buf(
                        fff_test_util::BP_WASM_FUNC,
                        &FlatEncUnit::read_first_buffer(bytes.clone()).unwrap(),
                    )
                    .unwrap();
                let mut res = vec![Buffer::from_vec::<u8>(vec![])];
                res.extend([output.into()]);
                let output =
                    primitive_array_from_arrow_buffers(arr.data_type(), res, vec_size.into())
                        .unwrap();
                assert_eq!(*arr, *output);
                outputs.push_back(output);
            }
            for _ in 0..iterations {
                outputs.pop_front();
            }
            let total = iterations;
            for _ in 0..total {
                // This closure contains the code to be measured
                let output = rt
                    .call_single_buf(
                        fff_test_util::BP_WASM_FUNC,
                        &FlatEncUnit::read_first_buffer(bytes.clone()).unwrap(),
                    )
                    .unwrap();
                let mut res = vec![Buffer::from_vec::<u8>(vec![])];
                res.extend([output.into()]);
                let output =
                    primitive_array_from_arrow_buffers(arr.data_type(), res, vec_size.into())
                        .unwrap();
                assert_eq!(*arr, *output);
                outputs.push_back(output);
            }
            for i in 0..total {
                println!("{:?}", outputs[i].to_data().buffers()[0].as_ptr());
                assert_eq!(*arr, outputs[i]);
            }
            // println!("size: {:?}", rt.memory_size());
        }

        let enc = VortexEncoder::default();
        let bytes = encode(enc, arr.clone()).freeze();

        let filename = fff_test_util::VORTEX_WASM_PATH.as_path();
        let rt = Runtime::try_new(&std::fs::read(filename).unwrap()).unwrap();
        let output = rt
            .call_single_buf(
                fff_test_util::VORTEX_WASM_FUNC,
                &FlatEncUnit::read_first_buffer(bytes.clone()).unwrap(),
            )
            .unwrap();
        let mut res = vec![Buffer::from_vec::<u8>(vec![])];
        res.extend([output.into()]);
        let output =
            primitive_array_from_arrow_buffers(arr.data_type(), res, vec_size.into()).unwrap();
        assert_eq!(*arr, *output);
    }

    #[test]
    fn multi_thread() {
        const CHUNK_SIZE: usize = 64 * 1024;
        const TOTAL_CHUNKS: usize = 64 * 16 * 1; // Just a parameter to control the total file size.

        let arr = fff_bench::generate_data(CHUNK_SIZE);
        let enc = BPEncoder;
        let bytes = encode(enc, arr.clone()).freeze();
        let first_buffer = FlatEncUnit::read_first_buffer(bytes.clone()).unwrap();
        let rt = Arc::new(
            Runtime::with_config_engine(
                &std::fs::read(fff_test_util::BP_WASM_PATH.as_path()).unwrap(),
                Config::default(),
                &wasmtime::Engine::new(
                    wasmtime::Config::new().profiler(wasmtime::ProfilingStrategy::None), // .static_memory_maximum_size(0)
                                                                                         // .guard_before_linear_memory(false)
                                                                                         // .static_memory_guard_size(64 * 1024)
                                                                                         // .dynamic_memory_reserved_for_growth(64 * 1024),
                )
                .unwrap(),
            )
            .unwrap(),
        );
        let output_arrays = Arc::new(Mutex::new(Vec::<ArrayRef>::new()));
        let num_threads = 16;
        let iterations = TOTAL_CHUNKS / num_threads;
        for _ in 0..iterations {
            let mut handles = vec![];
            for _ in 0..num_threads {
                let buf = first_buffer.clone();
                let output_arrays = output_arrays.clone();
                let rt = rt.clone();
                handles.push(std::thread::spawn(move || {
                    let dec = WASMEncUnitDecoder::new(
                        buf,
                        rt,
                        fff_test_util::BP_WASM_FUNC,
                        arrow::datatypes::DataType::UInt32,
                        CHUNK_SIZE as u64,
                    );
                    output_arrays.lock().unwrap().push(dec.decode().unwrap());
                }));
            }
            for handle in handles {
                handle.join().unwrap();
            }
        }
        for out in output_arrays.lock().unwrap().iter() {
            // let mut res = vec![Buffer::from_vec::<u8>(vec![])];
            // res.extend([buf.clone()]);
            // let output =
            //     primitive_array_from_arrow_buffers(arr.data_type(), res, CHUNK_SIZE as u64)
            //         .unwrap();
            assert_eq!(*arr, *out);
        }
    }

    #[test]
    fn memory_size() {
        const CHUNK_SIZE: usize = 64 * 1024;

        let arr = fff_bench::generate_data(CHUNK_SIZE);
        let enc = BPEncoder;
        let bytes = encode(enc, arr.clone()).freeze();
        let first_buffer = FlatEncUnit::read_first_buffer(bytes.clone()).unwrap();
        // size is the allocated size before actual testing.
        for size in [1, 64, 256, 1024, 4 * 1024, 15 * 1024, 16 * 1024].into_iter() {
            let rt = Arc::new(
                Runtime::with_config_engine(
                    &std::fs::read(fff_test_util::BP_WASM_PATH.as_path()).unwrap(),
                    Config::default(),
                    &wasmtime::Engine::new(
                        wasmtime::Config::new().profiler(wasmtime::ProfilingStrategy::None), // .static_memory_maximum_size(64 * 1024 * 4 * 4)
                                                                                             // .guard_before_linear_memory(false)
                                                                                             // .static_memory_guard_size(64 * 1024),
                    )
                    .unwrap(),
                )
                .unwrap(),
            );
            let mut output_arrays = vec![];
            let dec = WASMEncUnitDecoder::new(
                first_buffer.clone(),
                rt.clone(),
                fff_test_util::BP_WASM_FUNC,
                arrow::datatypes::DataType::UInt32,
                CHUNK_SIZE as u64,
            );
            for _ in 0..size {
                output_arrays.push(dec.decode().unwrap());
            }
            let start = std::time::Instant::now();
            for _ in 0..1000 {
                std::hint::black_box(dec.decode().unwrap());
            }
            println!("size: {:?}, time: {:?}", size, start.elapsed() / 1000);
        }
    }

    #[test]
    fn test_wasm_general() {
        use fff_ude_wasm::Runtime;
        let vec_size: u64 = 64 * 1024;
        let vec: Vec<String> = (1..=vec_size).map(|x| x.to_string()).collect();
        let arr = StringArray::from(vec);
        let arr = Arc::new(arr) as ArrayRef;
        let enc = VortexEncoder::default();
        let bytes = encode(enc, arr.clone()).freeze();

        fn arraydata_to_buffers(res: &mut Vec<Buffer>, array_data: &ArrayData) {
            res.push(match array_data.nulls() {
                Some(nulls) => nulls.buffer().clone(),
                None => MutableBuffer::new(0).into(),
            });
            for buffer in array_data.buffers() {
                res.push(buffer.clone());
            }
            for child in array_data.child_data() {
                arraydata_to_buffers(res, child);
            }
        }
        let mut native_decoder = VortexDecoder::try_new(
            FlatEncUnit::read_first_buffer(bytes.clone()).unwrap(),
            ALL_ENCODINGS_CONTEXT.clone(),
        )
        .unwrap();
        let native_output = native_decoder.decode_all_as_array().unwrap();
        assert_eq!(*arr, *native_output);
        let mut buffers = vec![];
        arraydata_to_buffers(&mut buffers, &native_output.to_data());
        let native_output =
            primitive_array_from_arrow_buffers_iter(arr.data_type(), buffers.into_iter(), vec_size)
                .unwrap();
        assert_eq!(*arr, *native_output);

        let filename = fff_test_util::VORTEX_WASM_PATH.as_path();
        let rt = Runtime::try_new(&std::fs::read(filename).unwrap()).unwrap();
        let mut outputs = vec_deque::VecDeque::new();
        // This closure contains the code to be measured
        for _ in 0..2 {
            let output = rt
                .call_multi_buf(
                    fff_test_util::VORTEX_WASM_FUNC_GENERAL,
                    &FlatEncUnit::read_first_buffer(bytes.clone()).unwrap(),
                )
                .unwrap();
            println!("{:?}", rt.memory_size());
            let output =
                primitive_array_from_arrow_buffers_iter(arr.data_type(), output, vec_size).unwrap();
            assert_eq!(*arr, *output);
            outputs.push_back(output);
        }
        println!("{:?}", outputs[0].to_data().buffers()[0].as_ptr());
        assert_eq!(*arr, outputs[0]);
        // println!("size: {:?}", rt.memory_size());
    }
}
