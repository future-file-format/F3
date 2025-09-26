/// For finding the reason behind the wmemcheck error behind calling exported alloc funcion
/// 12/23/2024
use anyhow::Result;
use wasmtime::*;

fn main() -> Result<()> {
    // Modules can be compiled through either the text or binary format
    let engine = Engine::new(&Config::new().wmemcheck(true))?;
    let module = Module::from_file(
        &engine,
        "/home/xinyu/fff-devel/target/wasm32-unknown-unknown/release/test_wmemcheck.wasm",
        // "/home/xinyu/fff-devel/target/wasm32-wasip1/release/test_wmemcheck.wasm",
    )?;

    // Create a `Linker` which will be later used to instantiate this module.
    // Host functionality is defined by name within the `Linker`.
    let mut linker = Linker::new(&engine);
    linker.func_wrap(
        "host",
        "host_func",
        |caller: Caller<'_, u32>, param: i32| {
            println!("Got {} from WebAssembly", param);
            println!("my host state is: {}", caller.data());
        },
    )?;

    // All wasm objects operate within the context of a "store". Each
    // `Store` has a type parameter to store host-specific data, which in
    // this case we're using `4` for.
    let mut store = Store::new(&engine, 4);
    let instance = linker.instantiate(&mut store, &module)?;
    let alloc = instance.get_typed_func::<(u32, u32), (u32)>(&mut store, "alloc")?;

    // And finally we can call the wasm!
    alloc.call(&mut store, (64, 4))?;

    Ok(())
}
