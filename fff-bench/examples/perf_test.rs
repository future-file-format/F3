use fff_bench::encode;
use fff_encoding::{enc_unit::FlatEncUnit, schemes::bp::BPEncoder};
use fff_poc::decoder::encunit::EncUnitDecoder;
use fff_poc::decoder::encunit::WASMEncUnitDecoder;
use fff_ude_wasm::{Config, Runtime};
use std::sync::Arc;
use sysinfo::System;

/// cargo build -p fff-encoding-bench --profile bench --examples
/// sudo perf record -k mono /home/xinyu/fff-devel/target/release/examples/perf_test
/// sudo perf inject --jit --input perf.data --output perf.jit.data
/// sudo perf report --input perf.jit.data
/// Doesn't provide too many insights since there is a weird buck of time. Also we couldn't assign the perf start and end intervel in the code.

fn main() {
    let mut system = System::new_all();

    // Refresh system data
    system.refresh_all();

    // Get the current process by its PID
    let pid = sysinfo::get_current_pid().expect("Failed to get PID");
    let print_memory_footprint = || {
        if let Some(process) = system.process(pid) {
            let memory = process.virtual_memory();
            println!("Current memory usage: {} B", memory);
        } else {
            eprintln!("Failed to acquire process information.");
        }
    };
    let size = 2048;
    const ITERATIONS: usize = 10000000;
    let arr = fff_bench::generate_data(size);
    let enc = BPEncoder;
    let bytes = encode(enc, arr.clone()).freeze();
    let first_buffer = FlatEncUnit::read_first_buffer(bytes.clone()).unwrap();
    print_memory_footprint();

    let rt = Arc::new(
        Runtime::with_config_engine(
            &std::fs::read(fff_test_util::NOOP_PATH.as_path()).unwrap(),
            Config::default(),
            &wasmtime::Engine::new(
                wasmtime::Config::new().profiler(wasmtime::ProfilingStrategy::JitDump),
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let dec = WASMEncUnitDecoder::new(
        first_buffer.clone(),
        rt.clone(),
        fff_test_util::NOOP_FUNC,
        arrow::datatypes::DataType::UInt32,
        0,
    );
    print_memory_footprint();
    for _ in 0..ITERATIONS {
        // decoded_vecs.push(dec.decode_all().unwrap());
        std::mem::forget(dec.decode().unwrap());
    }
    // println!("size: {:?}", rt.memory_size());
    print_memory_footprint();
    let instance2 = rt.get_an_instance().unwrap();
    print_memory_footprint();
    let instance3 = rt.get_an_instance().unwrap();
    print_memory_footprint();
    // println!("size: {:?}", rt.memory_size());
    println!("size: {:?}", instance2.memory_size());
    println!("size: {:?}", instance3.memory_size());

    // NOTE(xinyu): begin same instance testing.
    // The conclusion is that each instance has a unique memory to grow, even some of the instances can share a store.
    // But according to the doc, the memory of each instance will not be clean-up until the store is dropped.
    // So the current best practice is to have one instance for one store.

    // let module = &rt.module;
    // let engine = module.engine();

    // let wasi = wasi_common::sync::WasiCtxBuilder::new().build();
    // let limits = {
    //     let builder = wasmtime::StoreLimitsBuilder::new();
    //     builder.build()
    // };
    // let mut store = wasmtime::Store::new(engine, (wasi, limits));
    // store.limiter(|(_, limiter)| limiter);

    // let module = &rt.module;
    // let engine = module.engine();
    // let mut linker = wasmtime::Linker::new(engine);
    // wasi_common::sync::add_to_linker(&mut linker, |(wasi, _)| wasi).unwrap();
    // let mut instance1 = linker.instantiate(&mut store, module).unwrap();
    // let mut functions = std::collections::HashMap::new();
    // for export in module.exports() {
    //     // let Some(encoded) = export.name().strip_prefix("arrowudf_") else {
    //     //     continue;
    //     // };
    //     // let name = base64_decode(encoded).context("invalid symbol")?;
    //     // TODO: use base64 encoded function name
    //     if export.name().ends_with("ffi") {
    //         let func = instance1.get_typed_func(&mut store, export.name()).unwrap();
    //         functions.insert(export.name().to_string(), func);
    //     }
    // }
    // let alloc: wasmtime::TypedFunc<(u32, u32), u32> =
    //     instance1.get_typed_func(&mut store, "alloc").unwrap();
    // let dealloc: wasmtime::TypedFunc<(u32, u32, u32), ()> =
    //     instance1.get_typed_func(&mut store, "dealloc").unwrap();
    // // let record_batch_iterator_next =
    // //     instance.get_typed_func(&mut store, "record_batch_iterator_next")?;
    // // let record_batch_iterator_drop =
    // //     instance.get_typed_func(&mut store, "record_batch_iterator_drop")?;
    // let memory1 = instance1.get_memory(&mut store, "memory").unwrap();
    // let mut instance2 = linker.instantiate(&mut store, module).unwrap();
    // let memory2 = instance2.get_memory(&mut store, "memory").unwrap();
    // let name = fff_test_util::BP_WASM_FUNC;
    // let input = &first_buffer;
    // for _ in 0..ITERATIONS {
    //     // get function
    //     let func = functions.get(name).unwrap();

    //     // allocate memory for input buffer and output struct
    //     let alloc_len: u32 = u32::try_from(input.len() + 4 * 2).unwrap();
    //     let alloc_ptr: u32 = alloc.call(&mut store, (alloc_len, 4)).unwrap();
    //     let in_ptr: u32 = alloc_ptr + 4 * 2;

    //     // write input to memory
    //     memory1.write(&mut store, in_ptr as usize, input).unwrap();

    //     // call the function
    //     let result: Result<i32, anyhow::Error> =
    //         func.call(&mut store, (in_ptr, input.len() as u32, alloc_ptr));
    //     // let errno = self.append_stdio(result)?;
    //     // deallocate memory
    //     dealloc.call(&mut store, (alloc_ptr, alloc_len, 4)).unwrap();
    // }
    // print_memory_footprint();
    // println!("size: {:?}", memory1.data_size(&store));
    // println!("size: {:?}", memory2.data_size(&store));

    // let module = &rt.module;
    // let engine = module.engine();
    // let mut functions = std::collections::HashMap::new();
    // for export in module.exports() {
    //     // let Some(encoded) = export.name().strip_prefix("arrowudf_") else {
    //     //     continue;
    //     // };
    //     // let name = base64_decode(encoded).context("invalid symbol")?;
    //     // TODO: use base64 encoded function name
    //     if export.name().ends_with("ffi") {
    //         let func = instance2.get_typed_func(&mut store, export.name()).unwrap();
    //         functions.insert(export.name().to_string(), func);
    //     }
    // }
    // let alloc: wasmtime::TypedFunc<(u32, u32), u32> =
    //     instance2.get_typed_func(&mut store, "alloc").unwrap();
    // let dealloc: wasmtime::TypedFunc<(u32, u32, u32), ()> =
    //     instance2.get_typed_func(&mut store, "dealloc").unwrap();
    // let name = fff_test_util::BP_WASM_FUNC;
    // let input = &first_buffer;
    // for _ in 0..ITERATIONS {
    //     // get function
    //     let func = functions.get(name).unwrap();

    //     // allocate memory for input buffer and output struct
    //     let alloc_len: u32 = u32::try_from(input.len() + 4 * 2).unwrap();
    //     let alloc_ptr: u32 = alloc.call(&mut store, (alloc_len, 4)).unwrap();
    //     let in_ptr: u32 = alloc_ptr + 4 * 2;

    //     // write input to memory
    //     memory2.write(&mut store, in_ptr as usize, input).unwrap();

    //     // call the function
    //     let result: Result<i32, anyhow::Error> =
    //         func.call(&mut store, (in_ptr, input.len() as u32, alloc_ptr));
    //     // let errno = self.append_stdio(result)?;
    //     // deallocate memory
    //     dealloc.call(&mut store, (alloc_ptr, alloc_len, 4)).unwrap();
    // }
    // print_memory_footprint();
    // println!("size: {:?}", memory1.data_size(&store));
    // println!("size: {:?}", memory2.data_size(&store));
}
