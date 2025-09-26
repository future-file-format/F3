use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use fff_test_util::MEM_TEST_FUNC;
use pprof::criterion::{Output, PProfProfiler};
use wasi_common::sync::WasiCtxBuilder;
use wasmtime::TypedFunc;

fn memory_cost(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_cost");

    for size in [4, 4096, 65536].iter() {
        group.bench_with_input(BenchmarkId::new("native", size), size, |b, &size| {
            b.iter(|| {
                let vec = vec![0u8; size];
                black_box(vec.len());
            });
        });

        use wasmtime::*;
        let engine = Engine::default();

        let wasm = std::fs::read(fff_test_util::MEM_TEST_PATH.as_path()).unwrap();
        let module = Module::new(&engine, wasm).unwrap();

        let mut linker = Linker::new(&engine);
        wasi_common::sync::add_to_linker(&mut linker, |wasi| wasi).unwrap();

        let wasi = WasiCtxBuilder::new()
            .inherit_stdio()
            .inherit_args()
            .unwrap()
            .build();
        let mut store: Store<wasi_common::WasiCtx> = Store::new(&engine, wasi);
        let instance = linker.instantiate(&mut store, &module).unwrap();

        // Get the exported memory from the instance. Usually, the memory is exported by the module.
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or(anyhow::anyhow!("failed to find memory export"))
            .unwrap();

        let alloc: TypedFunc<(u32, u32), u32> = instance
            .get_typed_func::<(u32, u32), u32>(&mut store, "alloc")
            .unwrap();

        let dealloc: TypedFunc<(u32, u32, u32), ()> = instance
            .get_typed_func::<(u32, u32, u32), ()>(&mut store, "dealloc")
            .unwrap();

        let test = instance
            .get_typed_func::<(u32, u32, u32), i32>(&mut store, MEM_TEST_FUNC)
            .unwrap();
        let zeros = vec![0u8; *size];
        group.bench_with_input(BenchmarkId::new("wasm", size), size, |b, &size| {
            b.iter(|| {
                let input_ptr = alloc.call(&mut store, (size as u32, 4)).unwrap();
                memory
                    .write(&mut store, input_ptr as usize, &zeros)
                    .unwrap();
                black_box(
                    dealloc
                        .call(&mut store, (input_ptr, size as u32, 4))
                        .unwrap(),
                );
            });
        });
        let bytes = ((*size) as u32).to_le_bytes();
        group.bench_with_input(BenchmarkId::new("wasm-compiled", size), size, |b, &_| {
            b.iter(|| {
                let alloc_len = u32::try_from(bytes.len() + 4 * 2).unwrap();
                // let alloc_ptr = alloc.call(&mut store, (alloc_len, 4)).unwrap();
                let alloc_ptr = alloc_wasm(&alloc, &mut store, alloc_len, 4);
                let input_ptr = alloc_ptr + 4 * 2;
                memory
                    .write(&mut store, input_ptr as usize, &bytes)
                    .unwrap();
                black_box(
                    test.call(&mut store, (input_ptr, bytes.len() as u32, alloc_ptr))
                        .unwrap(),
                );

                let out_ptr = u32::from_le_bytes(
                    memory.data(&store)[alloc_ptr as usize..(alloc_ptr + 4) as usize]
                        .try_into()
                        .unwrap(),
                );
                let len_ptr = alloc_ptr + 4;
                let out_len = u32::from_le_bytes(
                    memory.data(&store)[len_ptr as usize..(len_ptr + 4) as usize]
                        .try_into()
                        .unwrap(),
                );
                let out_bytes = memory
                    .data(&store)
                    .get(out_ptr as usize..(out_ptr + out_len) as usize)
                    .unwrap();
                black_box(out_bytes);
                // dealloc.call(&mut store, (alloc_ptr, alloc_len, 4)).unwrap();
                // dealloc.call(&mut store, (out_ptr, out_len, 1)).unwrap();
                dealloc_wasm(&dealloc, &mut store, alloc_ptr, alloc_len, 4);
                dealloc_wasm(&dealloc, &mut store, out_ptr, out_len, 1);
            });
        });
    }
}

fn alloc_wasm(
    func: &TypedFunc<(u32, u32), u32>,
    store: &mut wasmtime::Store<wasi_common::WasiCtx>,
    len: u32,
    align: u32,
) -> u32 {
    func.call(store, (len, align)).unwrap()
}

fn dealloc_wasm(
    func: &TypedFunc<(u32, u32, u32), ()>,
    store: &mut wasmtime::Store<wasi_common::WasiCtx>,
    ptr: u32,
    len: u32,
    align: u32,
) {
    func.call(store, (ptr, len, align)).unwrap()
}

criterion_group! {
    name = benches;
    config = Criterion::default().with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)));
    targets = memory_cost
    // targets =simple_decode, two_k_decode, four_k_decode, diff_bw_decode
}
criterion_main!(benches);
