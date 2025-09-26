/// This test tries to profile the time of compiling a Wasm binary into native code before decoding the file.
use std::{fs::File, sync::Arc};

use criterion::black_box;
use fff_bench::encode;
use fff_encoding::{enc_unit::FlatEncUnit, schemes::bp::BPEncoder};
use fff_ude_wasm::{Config, Runtime};

fn main() {
    let arr = fff_bench::generate_data(65536);
    let enc = BPEncoder;
    let bytes = encode(enc, arr.clone()).freeze();
    let _first_buffer = FlatEncUnit::read_first_buffer(bytes.clone()).unwrap();
    println!("size: {:?}", bytes.len());

    let engine = wasmtime::Engine::new(
        &wasmtime::Config::new().parallel_compilation(true),
        // .strategy(wasmtime::Strategy::Winch)
        // .cranelift_opt_level(wasmtime::OptLevel::None), // .allocation_strategy(wasmtime::InstanceAllocationStrategy::pooling()),
    )
    .unwrap();
    let guard = pprof::ProfilerGuardBuilder::default()
        .frequency(1000)
        .blocklist(&["libc", "libgcc", "pthread", "vdso"])
        .build()
        .unwrap();
    let start = std::time::Instant::now();
    let iterations = 20;
    for _ in 0..iterations {
        let rt = Arc::new(
            Runtime::with_config_engine(
                &std::fs::read(
                    "/home/xinyu/fff-devel/target/wasm32-wasip1/release/fff_ude_example_fsst.wasm",
                )
                .unwrap(),
                Config::default(),
                &engine,
            )
            .unwrap(),
        );
        black_box(rt);
    }
    let elapsed = start.elapsed();
    println!("Elapsed: {:?}", elapsed / iterations);
    if let Ok(report) = guard.report().build() {
        let file = File::create("wasm_compile_test_flamegraph.svg").unwrap();
        report.flamegraph(file).unwrap();
    };
}
