/// Just a testing file of a true no-op function call cost in wASM
/// The conclustion is ~23ns per call and the number is verified by the maintainer.
use wasmtime::*;

use wasi_common::sync::WasiCtxBuilder;

#[allow(unused)]
fn core_exec(
    alloc: TypedFunc<(u32, u32), u32>,
    noop: TypedFunc<(u32, u32, u32), i32>,
    mut store: Store<wasi_common::WasiCtx>,
) {
    // let guard = pprof::ProfilerGuardBuilder::default()
    //     .frequency(1000)
    //     .blocklist(&["libc", "libgcc", "pthread", "vdso"])
    //     .build()
    //     .unwrap();
    let alloc_ptr = alloc.call(&mut store, (8, 4)).unwrap();
    let start = std::time::Instant::now();
    const ITERATIONS: u32 = 10000000;
    for _ in 0..ITERATIONS {
        noop.call(&mut store, (alloc_ptr, 0, alloc_ptr)).unwrap();
    }
    let elapsed = start.elapsed();
    println!("Elapsed: {:?}", elapsed / ITERATIONS);
    // if let Ok(report) = guard.report().build() {
    //     println!("report: {:?}", &report);
    //     let file = File::create("flamegraph.svg").unwrap();
    //     report.flamegraph(file).unwrap();
    // }
}

fn core_exec2(noop: TypedFunc<u32, ()>, mut store: Store<wasi_common::WasiCtx>) {
    // let guard = pprof::ProfilerGuardBuilder::default()
    //     .frequency(1000)
    //     .blocklist(&["libc", "libgcc", "pthread", "vdso"])
    //     .build()
    //     .unwrap();
    let start = std::time::Instant::now();
    const ITERATIONS: u32 = 10000000;
    for _ in 0..ITERATIONS {
        noop.call(&mut store, 0).unwrap();
    }
    let elapsed = start.elapsed();
    println!("Elapsed: {:?}", elapsed / ITERATIONS);
    // if let Ok(report) = guard.report().build() {
    //     println!("report: {:?}", &report);
    //     let file = File::create("flamegraph.svg").unwrap();
    //     report.flamegraph(file).unwrap();
    // }
}
fn main() {
    let engine = Engine::new(Config::new().profiler(ProfilingStrategy::JitDump)).unwrap();

    let wasm = std::fs::read(fff_test_util::NOOP_PATH.as_path()).unwrap();
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

    // let alloc: TypedFunc<(u32, u32), u32> = instance
    //     .get_typed_func::<(u32, u32), u32>(&mut store, "alloc")
    //     .unwrap();
    // let noop: TypedFunc<(u32, u32, u32), i32> = instance
    //     .get_typed_func::<(u32, u32, u32), i32>(&mut store, fff_test_util::NOOP_FUNC)
    //     .unwrap();
    // core_exec(alloc, noop, store);
    let noop: TypedFunc<u32, ()> = instance
        .get_typed_func::<u32, ()>(&mut store, "true_noop")
        .unwrap();
    core_exec2(noop, store);
}
