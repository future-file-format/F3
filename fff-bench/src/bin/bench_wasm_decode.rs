use anyhow::Context;
use bytemuck;
use clap::{Parser, Subcommand};
use fff_bench::helper::RAND_ARR_B8_W32_ARR;
use std::borrow::BorrowMut;
use wasi_common::sync::WasiCtxBuilder;

pub const ITERATIONS: u32 = 1000;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Wasmtime,
    Wasmer,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let file = std::env::var("CARGO_MANIFEST_DIR").unwrap()
        + "/../fff-wasm/wasm-generated/fls_example.wasm";
    match args.command {
        Some(Commands::Wasmtime) => bench_wasmtime(&file)?,
        Some(Commands::Wasmer) => bench_wasmer(&file)?,
        None => anyhow::bail!("No command provided"),
    }
    Ok(())
}

fn failed_export_msg(name: &str) -> String {
    format!("failed to find {} function export", name)
}

fn bench_wasmtime(file: &str) -> anyhow::Result<()> {
    use wasmtime::*;
    let engine = Engine::default();

    let wasm = std::fs::read(&file).with_context(|| format!("failed to read wasm file {file}"))?;
    let module = Module::new(&engine, wasm)?;

    // Host functionality can be arbitrary Rust functions and is provided
    // to guests through a `Linker`.
    let mut linker = Linker::new(&engine);
    wasi_common::sync::add_to_linker(&mut linker, |wasi| wasi)?;

    // Configure WASI and insert it into a `Store`
    let wasi = WasiCtxBuilder::new()
        .inherit_stdio()
        .inherit_args()?
        .build();
    let mut store = Store::new(&engine, wasi);
    // Instantiation of a module requires specifying its imports and then
    // afterwards we can fetch exports by name, as well as asserting the
    // type signature of the function with `get_typed_func`.
    let instance = linker.instantiate(&mut store, &module)?;

    // Get the exported memory from the instance. Usually, the memory is exported by the module.
    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or(anyhow::anyhow!("failed to find memory export"))?;

    let pack_8bit_32ow = instance
        .get_typed_func::<(i32, i32), ()>(&mut store, "pack_8bit_32ow")
        .with_context(|| failed_export_msg("pack_8bit_32ow"))?;
    let unpack_8bw_32ow_32crw_1uf = instance
        .get_typed_func::<(i32, i32), ()>(&mut store, "unpack_8bw_32ow_32crw_1uf")
        .with_context(|| failed_export_msg("unpack_8bw_32ow_32crw_1uf"))?;
    let alloc = instance
        .get_typed_func::<(i32, i32), i32>(&mut store, "alloc")
        .with_context(|| failed_export_msg("alloc"))?;
    let dealloc = instance
        .get_typed_func::<i32, ()>(&mut store, "dealloc")
        .with_context(|| failed_export_msg("dealloc"))?;
    // create a memory for the input, which is 8bits * 1024 = 1024 bytes
    let input_ptr = alloc.call(&mut store, (4, 1024))?;
    let output_ptr = alloc.call(&mut store, (4, 1024 * 4))?;
    let untrans_ptr = alloc.call(&mut store, (4, 1024 * 4))?;
    let mut start = std::time::Instant::now();
    for _ in 0..ITERATIONS {
        memory.write(
            &mut store,
            output_ptr as usize,
            bytemuck::cast_slice(&RAND_ARR_B8_W32_ARR),
        )?;
    }
    println!("write taken: {:?}", start.elapsed() / ITERATIONS);
    pack_8bit_32ow.call(&mut store, (output_ptr, input_ptr))?;
    let mut output = vec![0u8; 1024 * 4];
    start = std::time::Instant::now();
    for _ in 0..ITERATIONS {
        unpack_8bw_32ow_32crw_1uf.call(&mut store, (input_ptr, output_ptr))?;
    }
    println!("Unpack taken: {:?}", start.elapsed() / ITERATIONS);
    let fls_untranspose_generated =
        instance.get_typed_func::<(i32, i32), ()>(&mut store, "fls_untranspose_generated")?;
    start = std::time::Instant::now();
    for _ in 0..ITERATIONS {
        std::hint::black_box(
            fls_untranspose_generated.call(&mut store, (output_ptr, untrans_ptr)),
        )?;
    }
    println!("Untranspose taken: {:?}", start.elapsed() / ITERATIONS);
    let unrolled_rle_decoding =
        instance.get_typed_func::<(i32, i32, i32), ()>(&mut store, "unrolled_RLE_decoding")?;
    let idx_ptr = alloc.call(&mut store, (4, 1024 * 4))?;
    let mut idx = vec![0u32; 1024];
    for i in 0..1024 {
        idx[i] = i as u32 / 4;
    }
    memory.write(&mut store, idx_ptr as usize, bytemuck::cast_slice(&idx))?;
    start = std::time::Instant::now();
    for _ in 0..ITERATIONS {
        std::hint::black_box(
            unrolled_rle_decoding.call(&mut store, (untrans_ptr, output_ptr, idx_ptr)),
        )?;
    }
    println!("RLE Decode taken: {:?}", start.elapsed() / ITERATIONS);
    let binding = store.borrow_mut();
    let sc = binding.as_context();
    let slice = memory
        .data(&sc)
        .get(output_ptr as usize..)
        .and_then(|s| s.get(..output.len()))
        .unwrap();
    start = std::time::Instant::now();
    for _ in 0..ITERATIONS {
        // copy values out
        output.copy_from_slice(slice);
    }
    println!("memcpy taken: {:?}", start.elapsed() / ITERATIONS);
    dealloc.call(&mut store, input_ptr)?;
    dealloc.call(&mut store, output_ptr)?;
    dealloc.call(&mut store, untrans_ptr)?;
    dealloc.call(&mut store, idx_ptr)?;
    // validate the output, compare with input
    assert_eq!(
        RAND_ARR_B8_W32_ARR,
        bytemuck::cast_slice::<u8, u32>(&output)
    );
    Ok(())
}

fn bench_wasmer(file: &str) -> anyhow::Result<()> {
    use wasmer::*;
    let engine = Engine::default();

    let wasm = std::fs::read(&file).with_context(|| format!("failed to read wasm file {file}"))?;
    let module = Module::new(&engine, wasm)?;

    let mut store = Store::new(engine);

    let import_object = imports! {};
    let instance = Instance::new(&mut store, &module, &import_object).unwrap();

    let memory = instance.exports.get_memory("memory")?;
    let pack_8bit_32ow = instance
        .exports
        .get_function("pack_8bit_32ow")
        .with_context(|| failed_export_msg("pack_8bit_32ow"))?;
    let unpack_8bw_32ow_32crw_1uf = instance
        .exports
        .get_function("unpack_8bw_32ow_32crw_1uf")
        .with_context(|| failed_export_msg("unpack_8bw_32ow_32crw_1uf"))?;
    let alloc = instance
        .exports
        .get_function("alloc")
        .with_context(|| failed_export_msg("alloc"))?;
    let dealloc = instance
        .exports
        .get_function("dealloc")
        .with_context(|| failed_export_msg("dealloc"))?;
    // create a memory for the input, which is 8bits * 1024 = 1024 bytes
    let input_ptr = alloc.call(&mut store, &[Value::I32(4), Value::I32(1024)])?[0]
        .i32()
        .unwrap();
    let output_ptr = alloc.call(&mut store, &[Value::I32(4), Value::I32(4 * 1024)])?[0]
        .i32()
        .unwrap();
    let untrans_ptr = alloc.call(&mut store, &[Value::I32(4), Value::I32(4 * 1024)])?[0]
        .i32()
        .unwrap();
    let mut start = std::time::Instant::now();
    let view = memory.view(&mut store);
    for _ in 0..ITERATIONS {
        std::hint::black_box(view.write(
            output_ptr as u64,
            bytemuck::cast_slice(&RAND_ARR_B8_W32_ARR),
        )?);
    }
    println!("write taken: {:?}", start.elapsed() / ITERATIONS);
    pack_8bit_32ow.call(&mut store, &[Value::I32(output_ptr), Value::I32(input_ptr)])?;
    let mut packed_host = vec![0u8; 1024];
    memory
        .view(&mut store)
        .read(input_ptr as u64, &mut packed_host)?;
    start = std::time::Instant::now();
    let view = memory.view(&mut store);
    for _ in 0..ITERATIONS {
        std::hint::black_box(view.write(input_ptr as u64, &packed_host))?;
    }
    println!(
        "Copy packed values to guest taken: {:?}",
        start.elapsed() / ITERATIONS
    );
    let mut output = vec![0u8; 1024 * 4];
    start = std::time::Instant::now();
    for _ in 0..ITERATIONS {
        std::hint::black_box(
            unpack_8bw_32ow_32crw_1uf
                .call(&mut store, &[Value::I32(input_ptr), Value::I32(output_ptr)]),
        )?;
    }
    println!("Unpack taken: {:?}", start.elapsed() / ITERATIONS);
    let fls_untranspose_generated = instance
        .exports
        .get_function("fls_untranspose_generated")
        .with_context(|| "failed to find fls_untranspose_generated function export")?;
    start = std::time::Instant::now();
    for _ in 0..ITERATIONS {
        std::hint::black_box(fls_untranspose_generated.call(
            &mut store,
            &[Value::I32(output_ptr), Value::I32(untrans_ptr)],
        ))?;
    }
    println!("Untranspose taken: {:?}", start.elapsed() / ITERATIONS);
    let unrolled_rle_decoding = instance
        .exports
        .get_function("unrolled_RLE_decoding")
        .with_context(|| "failed to find unrolled_RLE_decoding function export")?;
    let idx_ptr = alloc.call(&mut store, &[Value::I32(4), Value::I32(4 * 1024)])?[0]
        .i32()
        .unwrap();
    let mut idx = vec![0u32; 1024];
    for i in 0..1024 {
        idx[i] = i as u32 / 4;
    }
    memory
        .view(&mut store)
        .write(idx_ptr as u64, bytemuck::cast_slice(&idx))?;
    start = std::time::Instant::now();
    for _ in 0..ITERATIONS {
        std::hint::black_box(unrolled_rle_decoding.call(
            &mut store,
            &[
                Value::I32(untrans_ptr),
                Value::I32(output_ptr),
                Value::I32(idx_ptr),
            ],
        ))?;
    }
    println!("RLE Decode taken: {:?}", start.elapsed() / ITERATIONS);
    start = std::time::Instant::now();
    let view = memory.view(&mut store);
    for _ in 0..ITERATIONS {
        std::hint::black_box(view.read(output_ptr as u64, &mut output)?);
    }
    println!(
        "Copy output to host taken: {:?}",
        start.elapsed() / ITERATIONS
    );
    start = std::time::Instant::now();
    for _ in 0..ITERATIONS {
        std::hint::black_box(
            memory
                .view(&mut store)
                .write(input_ptr as u64, &packed_host),
        )?;
        std::hint::black_box(
            unpack_8bw_32ow_32crw_1uf
                .call(&mut store, &[Value::I32(input_ptr), Value::I32(output_ptr)]),
        )?;
        std::hint::black_box(
            memory
                .view(&mut store)
                .read(output_ptr as u64, &mut output)?,
        );
    }
    println!("together taken: {:?}", start.elapsed() / ITERATIONS);
    // dealloc
    dealloc.call(&mut store, &[Value::I32(input_ptr)])?;
    dealloc.call(&mut store, &[Value::I32(output_ptr)])?;
    dealloc.call(&mut store, &[Value::I32(untrans_ptr)])?;
    dealloc.call(&mut store, &[Value::I32(idx_ptr)])?;
    // validate the output, compare with input
    assert_eq!(
        RAND_ARR_B8_W32_ARR,
        bytemuck::cast_slice::<u8, u32>(&output)
    );
    Ok(())
}
