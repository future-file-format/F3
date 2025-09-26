use arrow_array::ArrayRef;
use fff_poc::decoder::encunit::EncUnitDecoder;
use std::sync::{Arc, Mutex};
use std::thread;

use arrow_buffer::Buffer;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use fff_bench::encode;
use fff_encoding::enc_unit::FlatEncUnit;
use fff_encoding::schemes::bp::{BPDecoder, BPEncoder};
use fff_encoding::schemes::Decoder;
use fff_poc::decoder::encunit::WASMEncUnitDecoder;
use fff_ude_wasm::{Config, Runtime};
use pprof::criterion::{Output, PProfProfiler};

fn multi_thread(c: &mut Criterion) {
    let mut group = c.benchmark_group("multi_thread");
    group.sample_size(10);
    const CHUNK_SIZE: usize = 2 * 1024;
    const TOTAL_CHUNKS: usize = 64 * 16 * 512; // Just a parameter to control the total file size.
    let arr = fff_bench::generate_data(CHUNK_SIZE);
    let enc = BPEncoder;
    let bytes = encode(enc, arr.clone()).freeze();
    let first_buffer = FlatEncUnit::read_first_buffer(bytes.clone()).unwrap();

    for num_threads in [2, 4, 8, 16, 32, 64].iter() {
        let iterations = TOTAL_CHUNKS / num_threads;
        group.bench_with_input(
            BenchmarkId::new("native", num_threads),
            num_threads,
            |b, &num_threads| {
                b.iter(|| {
                    let output_buffers = Arc::new(Mutex::new(Vec::<Buffer>::new()));
                    for _ in 0..iterations {
                        let mut handles = vec![];
                        for _ in 0..num_threads {
                            let buf = first_buffer.clone();
                            let output_buffers = output_buffers.clone();
                            handles.push(thread::spawn(move || {
                                let mut dec = BPDecoder::new(buf);
                                output_buffers
                                    .lock()
                                    .unwrap()
                                    .push(dec.decode_all().unwrap().remove(0));
                            }));
                        }
                        for handle in handles {
                            handle.join().unwrap();
                        }
                    }
                });
            },
        );
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
        group.bench_with_input(
            BenchmarkId::new("wasm-shared-instance", num_threads),
            // BenchmarkId::new("wasm", num_threads),
            num_threads,
            |b, &num_threads| {
                b.iter(|| {
                    let output_buffers = Arc::new(Mutex::new(Vec::<ArrayRef>::new()));
                    for _ in 0..iterations {
                        let mut handles = vec![];
                        for _ in 0..num_threads {
                            let buf = first_buffer.clone();
                            let output_buffers = output_buffers.clone();
                            let rt = rt.clone();
                            handles.push(thread::spawn(move || {
                                let dec = WASMEncUnitDecoder::new(
                                    buf,
                                    rt,
                                    fff_test_util::BP_WASM_FUNC,
                                    arrow::datatypes::DataType::UInt32,
                                    CHUNK_SIZE as u64,
                                );
                                output_buffers.lock().unwrap().push(dec.decode().unwrap());
                            }));
                            // println!("Thread {} finished", j);
                        }
                        for handle in handles {
                            handle.join().unwrap();
                        }
                        // println!("Iteration {} finished", i);
                    }
                });
            },
        );
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)));
    targets = multi_thread
}
criterion_main!(benches);
