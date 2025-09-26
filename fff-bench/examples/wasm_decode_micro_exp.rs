/// This test tries to microbench the time of wasm decoder vs. native, excluding wasm JIT time.
/// Make sure you have built the wasm binaries via `./exp_scripts/build_wasm.sh` before running this test.
use std::{
    fs::File,
    io::Read,
    path::PathBuf,
    sync::{Arc, LazyLock},
};

use arrow_array::{ArrayRef, Float32Array, StringArray, UInt32Array};
use criterion::black_box;
use fff_test_util::TEST_SCHEMES;
use fff_ude_wasm::{Config, Runtime};
use itertools::Itertools;
use wasm_test_encoders::*;

const STR_LEN: usize = 21;

#[inline(never)]
fn run_wasm(encoded: &[u8], iterations: u32, rt: Arc<Runtime>) -> u128 {
    let buffers: Vec<arrow_buffer::Buffer> = rt
        .call_multi_buf("decode_general_ffi", &encoded)
        .unwrap()
        .collect();
    black_box(buffers);

    // let guard = pprof::ProfilerGuardBuilder::default()
    //     .frequency(1000)
    //     .blocklist(&["libc", "libgcc", "pthread", "vdso"])
    //     .build()
    //     .unwrap();
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        // let start1 = std::time::Instant::now();
        let buffers: Vec<arrow_buffer::Buffer> = rt
            .call_multi_buf("decode_general_ffi", &encoded)
            .unwrap()
            .collect();
        // debug_assert_eq!(array.slice(0, data_size).to_data().buffers()[0], buffers[1]);
        black_box(buffers);
        // println!("{:?}", start1.elapsed());
    }
    let elapsed = start.elapsed();
    // if let Ok(report) = guard.report().build() {
    //     let file = File::create("wasm_decode_flamegraph.svg").unwrap();
    //     report.flamegraph(file).unwrap();
    // };
    let wasm_time = (elapsed / iterations).as_nanos();
    wasm_time
}

static FFF_BENCH_PATH: LazyLock<PathBuf> =
    LazyLock::new(|| PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()));
static PROJ_ROOT: LazyLock<PathBuf> =
    LazyLock::new(|| FFF_BENCH_PATH.parent().unwrap().to_path_buf());

fn main() {
    let engine = wasmtime::Engine::new(
        &wasmtime::Config::new()
            // .profiler(wasmtime::ProfilingStrategy::PerfMap)
            // .debug_info(true)
            .cranelift_opt_level(wasmtime::OptLevel::None), // .wmemcheck(true)
                                                            // .coredump_on_trap(true),
    )
    .unwrap();
    let full_size = 65536;
    let int_data: Vec<u32> = (0..full_size).map(|x| x as u32 % 128).collect();
    let array = Arc::new(UInt32Array::from(int_data.clone())) as ArrayRef;
    let float_data: Vec<f32> = (0..full_size).map(|i| i as f32 * 0.5).collect();
    let mut string_data: Vec<u8> = vec![];
    File::open(PROJ_ROOT.join("fff-bench/examples/datasets/l_comment"))
        .unwrap()
        .read_to_end(&mut string_data)
        .unwrap();
    let email_data = String::from_utf8(string_data.clone()).unwrap();
    let emails: Vec<&str> = email_data.split('\n').collect();
    let str_array = Arc::new(StringArray::from(emails.clone())) as ArrayRef;
    println!("data_size,scheme,profile,unencoded_size,encoded_size,native_time(ns),wasm_time(ns)");
    // for data_size in [full_size] {
    for data_size in [2048, 4096, 8192, 16 * 1024, 32 * 1024, full_size] {
        for profile in ["release", "opt-size", "opt-size-lvl3"] {
            // for scheme in ["flsbp"] {
            for scheme in TEST_SCHEMES {
                let core_body = |array: ArrayRef, dtype: &str| {
                    let (encoded, unencoded_size) = match scheme {
                        "pco" => (encode_pco_general(&float_data[0..data_size]), data_size * 4),
                        "noop" => (encode_pco_general(&float_data[0..data_size]), data_size * 4),
                        "lz4" => {
                            let sliced = emails.iter().take(data_size).join("\n").into_bytes();
                            (encode_lz4_general(&sliced), sliced.len())
                        }
                        "flsbp" => (encode_flsbp_general(&int_data[0..data_size]), data_size * 4),
                        "fff" => (encode_fff_general(array.slice(0, data_size)), data_size * 4),
                        "gzip" => {
                            let sliced = emails.iter().take(data_size).join("\n").into_bytes();
                            (encode_gzip_general(&sliced), sliced.len())
                        }
                        "zstd" => {
                            let sliced = emails.iter().take(data_size).join("\n").into_bytes();
                            (encode_zstd_general(&sliced), sliced.len())
                        }
                        _ => panic!(),
                    };
                    let encoded_size = encoded.len();
                    let iterations = 1000;
                    // let mut buffers: Vec<arrow_buffer::Buffer> = vec![];
                    let start = std::time::Instant::now();
                    for _ in 0..iterations {
                        let buffers: Vec<arrow_buffer::Buffer> = match scheme {
                            "pco" => decode_pco_general(&encoded),
                            "noop" => decode_pco_general(&encoded),
                            "lz4" => decode_lz4_general(&encoded),
                            "gzip" => decode_gzip_general(&encoded),
                            "zstd" => decode_zstd_general(&encoded),
                            "flsbp" => decode_flsbp_native(&encoded),
                            "fff" => decode_fff_general_normal_ver(&encoded),
                            _ => panic!(),
                        }
                        .unwrap()
                        .collect();
                        black_box(buffers);
                    }
                    let elapsed = start.elapsed();
                    // println!("Native Elapsed: {:?}", elapsed / iterations);
                    let native_time = (elapsed / iterations).as_nanos();
                    // assert_eq!(
                    //     buffers[1].clone(),
                    //     arrow_buffer::Buffer::from(encoded.clone())
                    // );
                    let rt = Arc::new(
                        Runtime::with_config_engine(
                            &std::fs::read(PROJ_ROOT.join(format!(
                                "target/wasm32-wasip1/{profile}/fff_ude_example_{scheme}.wasm",
                            )))
                            .unwrap(),
                            Config::default(),
                            &engine,
                        )
                        .unwrap(),
                    );
                    let wasm_time = run_wasm(&encoded, iterations, rt);
                    // println!("Wasm Elapsed: {:?}", elapsed / iterations);
                    println!(
                        "{},{}{},{},{},{},{},{}",
                        data_size,
                        scheme,
                        dtype,
                        profile,
                        unencoded_size,
                        encoded_size,
                        native_time,
                        wasm_time
                    );
                };
                if scheme == "fff" {
                    core_body(array.clone(), "int");
                    let array = Arc::new(Float32Array::from(float_data.clone())) as ArrayRef;
                    core_body(array.clone(), "float");
                    core_body(str_array.clone(), "str");
                } else {
                    core_body(array.clone(), "");
                }
            }
        }
    }
}

#[test]
fn test() {
    let engine = wasmtime::Engine::new(
        &wasmtime::Config::new().cranelift_opt_level(wasmtime::OptLevel::None), // .wmemcheck(true)
                                                                                // .coredump_on_trap(true),
    )
    .unwrap();
    let mut string_data: Vec<u8> = vec![];
    File::open(PROJ_ROOT.join("fff-bench/examples/datasets/email"))
        .unwrap()
        .read_to_end(&mut string_data)
        .unwrap();
    let encoded = encode_lz4_general(&string_data[0..65536 * STR_LEN]);
    let rt = Arc::new(
        Runtime::with_config_engine(
            &std::fs::read(PROJ_ROOT.join(format!(
                "target/wasm32-wasip1/opt-size-lvl3/fff_ude_example_lz4.wasm",
            )))
            .unwrap(),
            Config::default(),
            &engine,
        )
        .unwrap(),
    );
    let buffers: Vec<arrow_buffer::Buffer> = rt
        .call_multi_buf("decode_general_ffi", &encoded)
        .unwrap()
        .collect();
    assert_eq!(&string_data[0..65536 * STR_LEN], buffers[1].to_vec());
}

#[test]
fn test2() {
    use fff_core::util::buffer_to_array::new_generic_byte_array_from_arrow_buffer_iter;
    let mut string_data: Vec<u8> = vec![];
    File::open(PROJ_ROOT.join("fff-bench/examples/datasets/email"))
        .unwrap()
        .read_to_end(&mut string_data)
        .unwrap();
    let email_data = String::from_utf8(string_data.clone()).unwrap();
    let emails: Vec<&str> = email_data.split('\n').collect();
    let str_array = Arc::new(StringArray::from(emails)) as ArrayRef;
    let encoded = encode_lz4_general2(str_array.clone());
    let decoded = decode_lz4_general2(&encoded).unwrap();
    let out = new_generic_byte_array_from_arrow_buffer_iter::<
        arrow::datatypes::GenericStringType<i32>,
    >(decoded, str_array.len() as u64);
    assert_eq!(*str_array, *out);
}

#[test]
fn test3() {
    let mut string_data: Vec<u8> = vec![];
    File::open(PROJ_ROOT.join("fff-bench/examples/datasets/email"))
        .unwrap()
        .read_to_end(&mut string_data)
        .unwrap();
    let email_data = String::from_utf8(string_data.clone()).unwrap();
    let encoded = encode_zstd_general(email_data.as_bytes());
    let mut decoded = decode_zstd_general(&encoded).unwrap();
    decoded.next().unwrap();
    assert_eq!(
        email_data.as_bytes(),
        decoded.next().unwrap().into_vec::<u8>().unwrap()
    );
}
