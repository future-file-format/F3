use fff_poc::decoder::encunit::EncUnitDecoder;
use std::sync::Arc;
use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use fff_bench::encode;
use fff_encoding::enc_unit::FlatEncUnit;
use fff_encoding::schemes::bp::{BPDecoder, BPEncoder};
use fff_encoding::schemes::vortex::{VortexDecoder, VortexEncoder};
use fff_encoding::schemes::Decoder;
use fff_poc::decoder::encunit::WASMEncUnitDecoder;
use fff_ude_wasm::{Config, Runtime};
use pprof::criterion::{Output, PProfProfiler};
use vortex_sampling_compressor::ALL_ENCODINGS_CONTEXT;

fn parameterized_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("parameterized_decode");
    group.warm_up_time(Duration::from_secs(2));
    // group.sample_size(10);

    for size in [1024, 2048, 4096, 8192, 16384, 32768, 65536].iter() {
        let arr = fff_bench::generate_data(*size);
        let enc = BPEncoder;
        let bytes = encode(enc, arr.clone()).freeze();
        let first_buffer = FlatEncUnit::read_first_buffer(bytes.clone()).unwrap();
        println!("size: {:?}", bytes.len());
        group.bench_with_input(BenchmarkId::new("fls-bp-size", size), size, |b, &size| {
            b.iter(|| {
                let mut dec = BPDecoder::new(first_buffer.clone());
                // This closure contains the code to be measured
                black_box(dec.decode_all())
            });
        });

        let engine = wasmtime::Engine::new(
            &wasmtime::Config::new(), // .allocation_strategy(wasmtime::InstanceAllocationStrategy::pooling()),
        )
        .unwrap();
        let rt = Arc::new(
            Runtime::with_config_engine(
                &std::fs::read(fff_test_util::BP_WASM_PATH.as_path()).unwrap(),
                Config::default(),
                &engine,
            )
            .unwrap(),
        );
        let dec = WASMEncUnitDecoder::new(
            first_buffer.clone(),
            rt,
            fff_test_util::BP_WASM_FUNC,
            arrow::datatypes::DataType::UInt32,
            *size as u64,
        );
        group.bench_with_input(BenchmarkId::new("fls-bp-wasm-size", size), size, |b, &_| {
            // // Test time withou dropping output
            // b.iter_custom(|iters| {
            //     let mut total = Duration::new(0, 0);
            //     for _ in 0..iters {
            //         let start = std::time::Instant::now();
            //         let output = black_box(dec.decode());
            //         total += start.elapsed();
            //         drop(output);
            //     }
            //     total
            // });
            b.iter(|| {
                // This closure contains the code to be measured
                black_box(dec.decode())
                // black_box(std::mem::forget(dec.decode()))
                // black_box(std::mem::forget(
                //     instance.call_scalar_function(fff_test_util::BP_WASM_FUNC, &first_buffer),
                // ))
            });
        });
        let rt = Arc::new(
            Runtime::with_config_engine(
                &std::fs::read(fff_test_util::NOOP_PATH.as_path()).unwrap(),
                Config::default(),
                &engine,
            )
            .unwrap(),
        );
        let dec = WASMEncUnitDecoder::new(
            first_buffer.clone(),
            rt,
            fff_test_util::NOOP_FUNC,
            arrow::datatypes::DataType::UInt32,
            *size as u64,
        );
        group.bench_with_input(
            BenchmarkId::new("fls-noop-wasm-size", size),
            size,
            |b, &_| {
                b.iter(|| {
                    // This closure contains the code to be measured
                    black_box(std::mem::forget(dec.decode()))
                    // black_box(std::mem::forget(dec.decode()))
                    // black_box(std::mem::forget(
                    //     instance.call_scalar_function(fff_test_util::BP_WASM_FUNC, &first_buffer),
                    // ))
                });
            },
        );

        let enc = VortexEncoder::default();
        let bytes = encode(enc, arr.clone()).freeze();
        let first_buffer = FlatEncUnit::read_first_buffer(bytes.clone()).unwrap();
        let mut dec =
            VortexDecoder::try_new(first_buffer.clone(), ALL_ENCODINGS_CONTEXT.clone()).unwrap();
        group.bench_with_input(BenchmarkId::new("fls-vortex-size", size), size, |b, &_| {
            b.iter(|| black_box(dec.decode_a_vector()));
        });

        let enc = VortexEncoder::default();
        let bytes = encode(enc, arr.clone()).freeze();
        let first_buffer = FlatEncUnit::read_first_buffer(bytes.clone()).unwrap();
        let rt = Arc::new(
            Runtime::with_config_engine(
                &std::fs::read(fff_test_util::VORTEX_WASM_PATH.as_path()).unwrap(),
                Config::default(),
                &engine,
            )
            .unwrap(),
        );
        let dec = WASMEncUnitDecoder::new(
            first_buffer.clone(),
            rt,
            fff_test_util::VORTEX_WASM_FUNC,
            arrow::datatypes::DataType::UInt32,
            *size as u64,
        );
        group.bench_with_input(
            BenchmarkId::new("fls-vortex-wasm-size", size),
            size,
            |b, &_| {
                b.iter(|| black_box(dec.decode()));
            },
        );
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default().with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)));
    targets = parameterized_decode
}
criterion_main!(benches);
