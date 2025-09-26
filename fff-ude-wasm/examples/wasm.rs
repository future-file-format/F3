use std::{
    io::{BufWriter, Cursor},
    sync::Arc,
};

use anyhow::{ensure, Context, Ok, Result};
use arrow_array::{ArrayRef, UInt32Array};
use bytes::BytesMut;
use fff_core::util::buffer_to_array::primitive_array_from_buffers;
use fff_encoding::{
    enc_unit::FlatEncUnit,
    schemes::{bp::BPEncoder, Encoder},
};
use fff_ude_wasm::Runtime;
use wasi_common::{sync::WasiCtxBuilder, WasiCtx};
use wasmtime::{Memory, Store};

fn failed_export_msg(name: &str) -> String {
    format!("failed to find {} function export", name)
}

fn read_u32(memory: &Memory, store: &mut Store<WasiCtx>, ptr: u32) -> Result<u32> {
    Ok(u32::from_le_bytes(
        memory.data(&store)[ptr as usize..(ptr + 4) as usize]
            .try_into()
            .unwrap(),
    ))
}

fn main() -> Result<()> {
    let filename = std::env::args().nth(1).expect("no filename");
    // let runtime = Runtime::with_config(
    //     &std::fs::read(filename).unwrap(),
    //     Config {
    //         memory_size_limit: None,
    //         file_size_limit: None,
    //         allocation_strategy: Some(InstanceAllocationStrategy::pooling()),
    //     },
    // )
    // .unwrap();
    let runtime = Runtime::try_new(&std::fs::read(filename).unwrap()).unwrap();

    println!("{runtime:#?}");

    // create 64k vector, with max value 127, randomly
    let vec_size = 64 * 1024;
    let vec: Vec<u32> = (1..=vec_size).map(|x| x % 128).collect();
    let arr = UInt32Array::from(vec);
    let arr = Arc::new(arr) as ArrayRef;
    let enc = BPEncoder {};
    let input_buf = Cursor::new(vec![]);
    let input = enc
        .encode(arr.clone())
        .unwrap()
        .try_serialize(input_buf)
        .unwrap();
    let input = BytesMut::from(input.into_inner().as_ref() as &[u8]);
    let input = FlatEncUnit::read_first_buffer(input.freeze()).unwrap();
    let start = std::time::Instant::now();
    let output = runtime.call_single_buf("decode_bp_ffi", &input).unwrap();
    println!("Execution: {:?}", start.elapsed());
    // println!("{}", runtime.memory_size());

    // NB: Should use wasm_buffer_to_arrow_buffer if cares copy overhead.
    let mut res = vec![BytesMut::default()];
    res.extend([BytesMut::from(output.as_ref())]);

    let output = primitive_array_from_buffers(arr.data_type(), res, vec_size.into()).unwrap();
    assert_eq!(*arr, *output);
    println!("output: {:?}", output);
    Ok(())
}

fn old_main() -> Result<()> {
    let filename = std::env::args().nth(1).expect("no filename");
    use arrow_array::ArrayRef;
    use wasmtime::*;
    let engine = Engine::default();

    let wasm =
        std::fs::read(&filename).with_context(|| format!("failed to read wasm file {filename}"))?;
    let start = std::time::Instant::now();
    let module = Module::new(&engine, wasm)?;
    println!("Compilation: {:?}", start.elapsed());
    let start = std::time::Instant::now();

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

    let decode_bp_ffi = instance
        .get_typed_func::<(u32, u32, u32), i32>(&mut store, "decode_bp_ffi")
        .with_context(|| "func failed to find")?;
    let alloc = instance
        .get_typed_func::<(u32, u32), u32>(&mut store, "alloc")
        .with_context(|| failed_export_msg("alloc"))?;
    let dealloc = instance
        .get_typed_func::<(u32, u32, u32), ()>(&mut store, "dealloc")
        .with_context(|| failed_export_msg("dealloc"))?;

    println!("Instantiation: {:?}", start.elapsed());

    // create 64k vector, with max value 127, randomly
    let vec_size = 64 * 1024;
    let vec: Vec<u32> = (1..=vec_size).map(|x| x % 128).collect();
    let arr = UInt32Array::from(vec);
    let arr = Arc::new(arr) as ArrayRef;
    let enc = BPEncoder {};
    let input = enc
        .encode(arr.clone())
        .unwrap()
        .try_serialize(Cursor::new(vec![]))
        .unwrap();
    let input = input.into_inner();
    let alloc_len = u32::try_from(input.len() + 4 * 2).context("input too large")?;
    let alloc_ptr = alloc.call(&mut store, (alloc_len, 4))?;
    ensure!(alloc_ptr != 0, "failed to allocate for input");
    let in_ptr = alloc_ptr + 4 * 2;
    println!("{}", memory.data_size(&mut store));
    // write input to memory
    memory.write(&mut store, in_ptr as usize, &input)?;
    println!("{}", memory.data_size(&mut store));

    // call the function
    let result = decode_bp_ffi.call(&mut store, (in_ptr, input.len() as u32, alloc_ptr))?;
    println!("{}", memory.data_size(&mut store));

    // get return values
    let out_ptr = read_u32(&memory, &mut store, alloc_ptr)?;
    let out_len = read_u32(&memory, &mut store, alloc_ptr + 4)?;
    // read output from memory
    let out_bytes = memory
        .data(&store)
        .get(out_ptr as usize..(out_ptr + out_len) as usize)
        .context("output slice out of bounds")?;

    let mut res = vec![BytesMut::default()];
    res.extend([BytesMut::from(out_bytes)]);

    let output = primitive_array_from_buffers(arr.data_type(), res, vec_size.into()).unwrap();
    assert_eq!(*arr, *output);
    println!("output: {:?}", output);
    // deallocate memory
    dealloc.call(&mut store, (alloc_ptr, alloc_len, 4))?;
    println!("{}", memory.data_size(&mut store));
    dealloc.call(&mut store, (out_ptr, out_len, 1))?;
    Ok(())
}
