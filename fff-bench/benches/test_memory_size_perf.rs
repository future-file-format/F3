use fff_poc::decoder::encunit::EncUnitDecoder;
use std::sync::Arc;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use fff_bench::encode;
use fff_encoding::enc_unit::FlatEncUnit;
use fff_encoding::schemes::bp::BPEncoder;
use fff_poc::decoder::encunit::WASMEncUnitDecoder;
use fff_ude_wasm::{Config, Runtime};
use pprof::criterion::{Output, PProfProfiler};

fn memory_size_perf(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_size");
    group.sample_size(10);
    const CHUNK_SIZE: usize = 64 * 1024;

    let arr = fff_bench::generate_data(CHUNK_SIZE);
    let enc = BPEncoder;
    let bytes = encode(enc, arr.clone()).freeze();
    let first_buffer = FlatEncUnit::read_first_buffer(bytes.clone()).unwrap();
    // size is the allocated size before actual testing.
    for size in [1, 64, 256, 1024, 4 * 1024, 16 * 1024].into_iter() {
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
        let mut output_buffers = vec![];
        let dec = WASMEncUnitDecoder::new(
            first_buffer.clone(),
            rt.clone(),
            fff_test_util::BP_WASM_FUNC,
            arrow::datatypes::DataType::UInt32,
            CHUNK_SIZE as u64,
        );
        for _ in 0..size {
            output_buffers.push(dec.decode().unwrap());
        }
        group.bench_with_input(BenchmarkId::new("wasm", size), &size, |b, &size| {
            b.iter(|| {
                black_box(dec.decode().unwrap());
            });
        });
    }
    group.finish();
}
criterion_group! {
    name = benches;
    config = Criterion::default().with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)));
    targets = memory_size_perf
    // targets =simple_decode, two_k_decode, four_k_decode, diff_bw_decode
}
criterion_main!(benches);
