use std::path::PathBuf;

/// For test wasm compile time
/// 12/24/2024
use anyhow::Result;
use wasi_common::sync::WasiCtxBuilder;
use wasmtime::*;

fn main() -> Result<()> {
    let start = std::time::Instant::now();
    // Modules can be compiled through either the text or binary format
    let engine = Engine::new(&Config::new())?;
    // let module = unsafe {
    //     Module::deserialize_file(
    //         &engine,
    //         PathBuf::from("/home/xinyu/fff-devel/target/wasm32-wasip1/opt-size-lvl3/fff_ude_example_fff.cwasm")
    //             ,
    //     )
    // }
    // .unwrap();
    let module = Module::from_file(
        &engine,
        "/home/xinyu/fff-devel/target/wasm32-wasip1/opt-size-lvl3/fff_ude_example_fff.wasm",
        // "/home/xinyu/fff-devel/target/wasm32-wasip1/release/test_wmemcheck.wasm",
    )?;

    let mut linker = Linker::new(&engine);
    wasi_common::sync::add_to_linker(&mut linker, |wasi| wasi)?;

    // Configure WASI and insert it into a `Store`
    let wasi = WasiCtxBuilder::new()
        .inherit_stdio()
        .inherit_args()?
        .build();
    let mut store = Store::new(&engine, wasi);
    let instance = linker.instantiate(&mut store, &module)?;
    // let alloc = instance.get_typed_func::<(u32, u32), (u32)>(&mut store, "alloc")?;

    // // And finally we can call the wasm!
    // alloc.call(&mut store, (64, 4))?;
    println!("Wasm compile time: {:?}", start.elapsed());
    Ok(())
}
