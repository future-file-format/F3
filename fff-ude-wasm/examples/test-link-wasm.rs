/// For testing linking multiple modules together.
/// Conclusion: it does not work at all, one module's function becomes imports of the other module.
/// The only way is imports the memory in each module, not export. But this seems to be non-trivial:
/// https://github.com/bytecodealliance/wasmtime/issues/5329
/// 12/28/2024
use anyhow::Result;
use wasi_common::sync::WasiCtxBuilder;
use wasmtime::*;

fn main() -> Result<()> {
    // Modules can be compiled through either the text or binary format
    let engine = Engine::new(&Config::new().wmemcheck(true))?;
    let module = Module::from_file(
        &engine,
        "/home/xinyu/fff-devel/target/wasm32-wasip1/opt-size-lvl3/fff_ude_example_pco.wasm",
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
    linker.module(&mut store, "pco", &module)?;
    let module = Module::from_file(
        &engine,
        "/home/xinyu/fff-devel/target/wasm32-wasip1/opt-size-lvl3/fff_ude_example_lz4_unique_name.wasm",
        // "/home/xinyu/fff-devel/target/wasm32-wasip1/release/test_wmemcheck.wasm",
    )?;
    let instance = linker.instantiate(&mut store, &module)?;
    for item in instance.exports(&mut store) {
        println!("{:?}", item.name());
    }
    Ok(())
}
