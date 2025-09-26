// Vendored and modified from https://github.com/risingwavelabs/arrow-udf
//
// Copyright 2024 RisingWave Labs
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#![doc = include_str!("../README.md")]

use anyhow::{anyhow, bail, ensure, Context};
use arrow_buffer::Buffer;
use once_cell;
use ram_file::{RamFile, RamFileRef};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Debug;
use std::ops::Range;
use std::sync::{Arc, Mutex};
use wasi_common::{sync::WasiCtxBuilder, WasiCtx};
use wasm_buffer::WasmBuffer;
use wasmtime::*;

mod ram_file;
// pub mod wasm_array;
pub mod wasm_buffer;

/// 128 is not working for pco, lz4, flsbp
const INPUT_ALIGNMENT: u32 = 4;

/// The WASM UDF runtime.
///
/// This runtime contains an instance pool and can be shared by multiple threads.
pub struct Runtime {
    module: Module,
    /// Configurations.
    config: Config,
    /// Function names.
    functions: HashSet<String>,
    /// User-defined types.
    types: HashMap<String, String>,
    /// Instance pool.
    instances: Mutex<VecDeque<Arc<Mutex<Instance>>>>,
    /// ABI version. (major, minor)
    abi_version: (u8, u8),
}

/// Configurations.
#[derive(Default)]
// #[non_exhaustive]
pub struct Config {
    /// Memory size limit in bytes.
    memory_size_limit: Option<usize>,
    /// File size limit in bytes.
    file_size_limit: Option<usize>,
}

impl Config {
    /// Set the memory size limit.
    pub fn memory_size_limit(mut self, limit: usize) -> Self {
        self.memory_size_limit = Some(limit);
        self
    }

    /// Set the file size limit.
    pub fn file_size_limit(mut self, limit: usize) -> Self {
        self.file_size_limit = Some(limit);
        self
    }
}

impl Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("memory_size_limit", &self.memory_size_limit)
            .field("file_size_limit", &self.file_size_limit)
            .finish()
    }
}

pub struct Instance {
    // extern "C" fn(len: usize, align: usize) -> *mut u8
    alloc: TypedFunc<(u32, u32), u32>,
    // extern "C" fn(ptr: *mut u8, len: usize, align: usize)
    dealloc: TypedFunc<(u32, u32, u32), ()>,
    // extern "C" fn(iter: *mut BufferIter, out: *mut CSlice, out_buffer_box_ptr: *mut u32)
    buffer_iterator_next: TypedFunc<(u32, u32, u32), ()>,
    // extern "C" fn(iter: *mut BufferIter)
    buffer_iterator_drop: TypedFunc<u32, ()>,
    // extern "C" fn(iter: *mut Buffer)
    buffer_drop: TypedFunc<u32, ()>,
    // init_ffi
    // extern "C" fn(input_ptr: *const u8, input_len: usize, kwargs_ptr: *const u8, kwargs_len: usize, out: *mut fff_ude::ffi::CSlice) -> i32
    init: Option<TypedFunc<(u32, u32, u32, u32, u32), i32>>,
    // decode_ffi
    // extern "C" fn(decoder: *mut WasmDecoder,out: *mut CSlice) -> i32
    decode: Option<TypedFunc<(u32, u32), i32>>,
    // extern "C" fn(ptr: *const u8, len: usize, out: *mut CSlice) -> i32
    functions: HashMap<String, TypedFunc<(u32, u32, u32), i32>>,
    // Input pointer which can be reused during the lifetime of this instance
    cached_alloc_ptr: Option<u32>,
    // Input pointer len which can be reused during the lifetime of this instance
    cached_alloc_len: Option<u32>,
    memory: Memory,
    // store: Store<()>,
    store: Store<(WasiCtx, StoreLimits)>,
    stdout: RamFileRef,
    stderr: RamFileRef,
}

impl Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("config", &self.config)
            .field("functions", &self.functions)
            .field("types", &self.types)
            .field("instances", &self.instances.lock().unwrap().len())
            .finish()
    }
}

/// To be cleanup, my failed try of caching Wasm compiled code
#[derive(Debug, Default)]
#[allow(dead_code)]
struct MyCacheStore;
static CACHE: Mutex<Option<HashMap<Vec<u8>, Vec<u8>>>> = Mutex::new(None);

impl CacheStore for MyCacheStore {
    fn get(&self, key: &[u8]) -> Option<std::borrow::Cow<[u8]>> {
        let mut cache = CACHE.lock().unwrap();
        let cache = cache.get_or_insert_with(HashMap::new);
        cache.get(key).map(|s| s.to_vec().into())
    }
    fn insert(&self, key: &[u8], value: Vec<u8>) -> bool {
        let mut cache = CACHE.lock().unwrap();
        let cache = cache.get_or_insert_with(HashMap::new);
        cache.insert(key.to_vec(), value);
        true
    }
}

static ENGINE: once_cell::sync::Lazy<Engine> = once_cell::sync::Lazy::new(|| {
    Engine::new(
        wasmtime::Config::new()
            // this not work
            // .enable_incremental_compilation(Arc::new(MyCacheStore) as Arc<dyn CacheStore>)
            // .unwrap()
            // .debug_info(true)
            .cranelift_opt_level(wasmtime::OptLevel::None)
            .parallel_compilation(true),
    )
    .unwrap()
});

impl Runtime {
    /// Create a new UDF runtime from a WASM binary.
    pub fn try_new(binary: &[u8]) -> Result<Self> {
        Self::with_config_engine(binary, Config::default(), &ENGINE)
    }

    /// Create a new UDF runtime from an AOT compiled binary.
    pub fn try_new_from_aot(aot_binary: &[u8]) -> Result<Self> {
        Self::with_config_engine_from_aot(aot_binary, Config::default(), &ENGINE)
    }

    fn init_from_module(module: Module, config: Config) -> Result<Self> {
        // check abi version
        let version = module
            .exports()
            .find_map(|e| e.name().strip_prefix("FFFUDE_VERSION_"))
            .context("version not found")?;
        let (major, minor) = version.split_once('_').context("invalid version")?;
        let (major, minor) = (major.parse::<u8>()?, minor.parse::<u8>()?);
        ensure!(major <= 3, "unsupported abi version: {major}.{minor}");

        let mut functions = HashSet::new();
        let types = HashMap::new();
        for export in module.exports() {
            functions.insert(export.name().to_string());
            // if let Some(encoded) = export.name().strip_prefix("arrowudf_") {
            //     let name = base64_decode(encoded).context("invalid symbol")?;
            //     functions.insert(name);
            // } else if let Some(encoded) = export.name().strip_prefix("arrowudt_") {
            //     let meta = base64_decode(encoded).context("invalid symbol")?;
            //     let (name, fields) = meta.split_once('=').context("invalid type string")?;
            //     types.insert(name.to_string(), fields.to_string());
            // }
        }

        Ok(Self {
            module,
            config,
            functions,
            types,
            instances: Mutex::new(vec![].into()),
            abi_version: (major, minor),
        })
    }

    /// Create a new UDF runtime from a WASM binary with a customized engine.
    pub fn with_config_engine(binary: &[u8], config: Config, engine: &Engine) -> Result<Self> {
        let module = Module::from_binary(engine, binary).context("failed to load wasm binary")?;
        Self::init_from_module(module, config)
    }

    /// Create a new UDF runtime from a WASM AOT-compiled binary with a customized engine.
    pub fn with_config_engine_from_aot(
        aot_binary: &[u8],
        config: Config,
        engine: &Engine,
    ) -> Result<Self> {
        let module = unsafe {
            Module::deserialize(engine, aot_binary).context("failed to load wasm binary")?
        };
        Self::init_from_module(module, config)
    }

    /// Return available functions.
    pub fn functions(&self) -> impl Iterator<Item = &str> {
        self.functions.iter().map(|s| s.as_str())
    }

    /// Return available types.
    pub fn types(&self) -> impl Iterator<Item = (&str, &str)> {
        self.types.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Return the ABI version.
    pub fn abi_version(&self) -> (u8, u8) {
        self.abi_version
    }

    /// Given a function signature that inlines struct types, find the function name.
    ///
    /// # Example
    ///
    /// ```text
    /// types = { "KeyValue": "key:string,value:string" }
    /// input = "keyvalue(string, string) -> struct<key:string,value:string>"
    /// output = "keyvalue(string, string) -> struct KeyValue"
    /// ```
    pub fn find_function_by_inlined_signature(&self, s: &str) -> Option<&str> {
        self.functions
            .iter()
            .find(|f| self.inline_types(f) == s)
            .map(|f| f.as_str())
    }

    /// Inline types in function signature.
    ///
    /// # Example
    ///
    /// ```text
    /// types = { "KeyValue": "key:string,value:string" }
    /// input = "keyvalue(string, string) -> struct KeyValue"
    /// output = "keyvalue(string, string) -> struct<key:string,value:string>"
    /// ```
    fn inline_types(&self, s: &str) -> String {
        let mut inlined = s.to_string();
        loop {
            let replaced = inlined.clone();
            for (k, v) in self.types.iter() {
                inlined = inlined.replace(&format!("struct {k}"), &format!("struct<{v}>"));
            }
            if replaced == inlined {
                return inlined;
            }
        }
    }

    /// Call a function that returns a single Buffer.
    pub fn call_single_buf(&self, _name: &str, _input: &[u8]) -> Result<WasmBuffer> {
        panic!("Deprecated");
        // if !self.functions.contains(name) {
        //     bail!("function not found: {name}");
        // }

        // // get an instance from the pool, or create a new one if the pool is empty
        // // let mut guard = self.instances.lock().unwrap();
        // // let instance = if guard.front().is_none()
        // //     || guard.front().unwrap().lock().unwrap().memory_size()
        // //         >= (4 * 1024 * 1024 * 1024 - 256 * 1024)
        // // {
        // //     Arc::new(Mutex::new(Instance::new(self)?))
        // // } else {
        // //     guard.pop_front().unwrap()
        // // };
        // // drop(guard);

        // let mut instance = if let Some(instance) = self.instances.lock().unwrap().pop_front() {
        //     instance
        // } else {
        //     Arc::new(Mutex::new(Instance::new(self)?))
        // };
        // // let instance = Arc::new(Mutex::new(Instance::new(self)?));

        // // call the function
        // let mut guard = instance.lock().unwrap();
        // let mut output = guard.call_scalar_function(name, input);

        // // put the instance back to the pool
        // if output.is_ok() {
        //     self.instances.lock().unwrap().push_back(instance.clone());
        // } else {
        //     drop(guard);
        //     instance = Arc::new(Mutex::new(Instance::new(self)?));
        //     guard = instance.lock().unwrap();
        //     output = guard.call_scalar_function(name, input);
        //     assert!(output.is_ok());
        //     self.instances.lock().unwrap().push_back(instance.clone());
        // }

        // output.map(|o| {
        //     WasmBuffer::new(
        //         // FIXME: return raw pointer it highly unsafe. Any subsequent call to grow/release the WASM memory will
        //         // make this pointer invalid.
        //         // See https://docs.rs/wasmtime/24.0.0/wasmtime/struct.Memory.html
        //         o.0.as_ptr() as usize,
        //         o.1,
        //         o.1, // Deprecated!
        //         o.0.len() as u32,
        //         instance.clone(),
        //     )
        // })
    }

    /// Call a function that returns a Buffer Iterator.
    pub fn call_multi_buf(&self, name: &str, input: &[u8]) -> Result<impl Iterator<Item = Buffer>> {
        if !self.functions.contains(name) {
            bail!("function not found: {name}");
        }

        let mut instance = if let Some(instance) = self.instances.lock().unwrap().pop_front() {
            instance
        } else {
            // dbg!("new instance1");
            Arc::new(Mutex::new(Instance::new(self)?))
        };
        // call the function
        let mut guard = instance.lock().unwrap();
        // dbg!(guard.memory_size());
        let mut output = guard.call_generic_function(name, input, instance.clone());

        // put the instance back to the pool
        if output.is_ok() {
            self.instances.lock().unwrap().push_back(instance.clone());
        } else {
            // println!("{:?}", output.as_ref().err());
            // dbg!("new instance2");
            // eprintln!("error: {:?}", output.as_ref().err());
            drop(guard);
            // We drop the instance here, but it may still be Arc'ed in some output Arrow Arrays.
            drop(instance);
            instance = Arc::new(Mutex::new(Instance::new(self)?));
            guard = instance.lock().unwrap();
            output = guard.call_generic_function(name, input, instance.clone());
            assert!(output.is_ok(), "error: {:?}", output.as_ref().err());
            self.instances.lock().unwrap().push_back(instance.clone());
        }

        output
    }

    /// NYI
    pub fn read_batch(
        &self,
        name: &str,
        input: &[u8],
        selection: RowSelection,
        reused_instance: Option<Arc<Mutex<Instance>>>,
    ) -> Result<StreamReadResult<impl Iterator<Item = Buffer>>> {
        if !self.functions.contains(name) {
            bail!("function not found: {name}");
        }

        let instance = if let Some(instance) = reused_instance {
            instance
        } else {
            Arc::new(Mutex::new(Instance::new(self)?))
        };

        // call the function
        let mut guard = instance.lock().unwrap();
        let output = guard.read_batch(name, input, selection, instance.clone());
        assert!(output.is_ok(), "error: {:?}", output.as_ref().err());
        let output = output.unwrap();
        // put the instance back to the pool if no more results
        if output.is_none() {
            self.instances.lock().unwrap().push_back(instance.clone());
            Ok(StreamReadResult::End)
        } else {
            drop(guard);
            Ok(StreamReadResult::Batch((output.unwrap(), instance)))
        }
    }

    /// WARNING: This function is for testing only.
    pub fn get_an_instance(&self) -> Result<Instance> {
        Instance::new(self)
    }

    // WARNING: This function is for testing only.
    pub fn memory_size(&self) -> usize {
        let guard = self.instances.lock().unwrap();
        let guard2 = guard[0].lock().unwrap();
        guard2.memory.data_size(&guard2.store)
    }
}

pub enum StreamReadResult<Iter>
where
    Iter: Iterator<Item = Buffer>,
{
    Batch((Iter, Arc<Mutex<Instance>>)),
    End,
}

pub enum RowSelection {
    All,
    Select(Vec<Range<usize>>),
}

struct BufferIter {
    ptr: u32,
    alloc_ptr: u32,
    // alloc_len: u32,
    // FIXME: it may be too large overhead here. Need re-evaluate to see the performance impact.
    instance_arc: Arc<Mutex<Instance>>,
}

impl BufferIter {
    /// Get the next record batch.
    fn next(&mut self) -> Result<Option<Buffer>> {
        let mut guard = self.instance_arc.lock().unwrap();
        guard.buffer_iterator_next(self.ptr, self.alloc_ptr)?;
        // get return values
        let out_ptr = guard.read_u32(self.alloc_ptr)?;
        let out_len = guard.read_u32(self.alloc_ptr + 4)?;
        // This is for the buffer address for the Arrow Buffer (include refcnt etc) in Wasm
        let arrow_buffer_address = guard.read_u32(self.alloc_ptr + 8)?;

        if out_ptr == 0 {
            // end of iteration
            return Ok(None);
        }

        // read output from memory
        let out_bytes = guard
            .memory
            .data(&guard.store)
            .get(out_ptr as usize..(out_ptr + out_len) as usize)
            .context("output slice out of bounds")?;
        // println!(
        //     "host:{}, guest:{}, len:{}",
        //     out_bytes.as_ptr() as usize,
        //     out_ptr,
        //     out_len
        // );
        let batch = WasmBuffer::new(
            out_bytes.as_ptr() as usize,
            out_ptr,
            out_len,
            arrow_buffer_address,
            self.instance_arc.clone(),
        )
        .into();
        // println!("{out_bytes:?} {out_ptr} {out_len}");
        // BUGFIX(1020): not dealloc output, the lifetime is handed over to the caller
        // guard.dealloc(out_ptr, out_len, 1)?;

        Ok(Some(batch))
    }
}

impl Iterator for BufferIter {
    type Item = Buffer;

    fn next(&mut self) -> Option<Self::Item> {
        let result = self.next();
        self.instance_arc
            .lock()
            .unwrap()
            .append_stdio(result)
            .unwrap()
    }
}

impl Drop for BufferIter {
    fn drop(&mut self) {
        let mut guard = self.instance_arc.lock().unwrap();
        // BUGFIX(1021): we reuse the input ptr and only reallocate if the input size is larger.
        // _ = guard.dealloc(self.alloc_ptr, self.alloc_len, 4).unwrap();
        guard.buffer_iterator_drop(self.ptr).unwrap();
    }
}

/// Store output like the Slice of WasmDecoder from Init()
pub struct WasmSlice {
    ptr: u32,
    len: u32,
}
impl WasmSlice {
    pub fn ptr(&self) -> u32 {
        self.ptr
    }

    pub fn len(&self) -> u32 {
        self.len
    }
}

impl Instance {
    /// Create a new instance.
    pub fn new(rt: &Runtime) -> Result<Self> {
        let module = &rt.module;
        let engine = module.engine();
        let mut linker = Linker::new(engine);
        wasi_common::sync::add_to_linker(&mut linker, |(wasi, _)| wasi)?;

        // Create a WASI context and put it in a Store; all instances in the store
        // share this context. `WasiCtxBuilder` provides a number of ways to
        // configure what the target program will have access to.
        let file_size_limit = rt.config.file_size_limit.unwrap_or(1024);
        let stdout = RamFileRef::new(RamFile::with_size_limit(file_size_limit));
        let stderr = RamFileRef::new(RamFile::with_size_limit(file_size_limit));
        let wasi = WasiCtxBuilder::new()
            .stdout(Box::new(stdout.clone()))
            .stderr(Box::new(stderr.clone()))
            .build();
        let limits = {
            let mut builder = StoreLimitsBuilder::new();
            if let Some(limit) = rt.config.memory_size_limit {
                builder = builder.memory_size(limit);
            }
            builder.build()
        };
        let mut store = Store::new(engine, (wasi, limits));
        store.limiter(|(_, limiter)| limiter);

        let instance = linker.instantiate(&mut store, module)?;
        // let mut store = Store::new(engine, ());
        // let instance = wasmtime::Instance::new(&mut store, module, &[])?;
        let mut functions = HashMap::new();
        for export in module.exports() {
            // let Some(encoded) = export.name().strip_prefix("arrowudf_") else {
            //     continue;
            // };
            // let name = base64_decode(encoded).context("invalid symbol")?;
            // TODO: use base64 encoded function name
            if export.name().ends_with("ffi") {
                let func = instance.get_typed_func(&mut store, export.name());
                if let Ok(func) = func {
                    functions.insert(export.name().to_string(), func);
                } else {
                    // eprintln!("Get typed function failed from {}", export.name());
                }
            }
        }
        let alloc = instance.get_typed_func(&mut store, "alloc")?;
        let dealloc = instance.get_typed_func(&mut store, "dealloc")?;

        let buffer_iterator_next = instance.get_typed_func(&mut store, "buffer_iterator_next")?;
        let buffer_iterator_drop = instance.get_typed_func(&mut store, "buffer_iterator_drop")?;
        let buffer_drop = instance.get_typed_func(&mut store, "buffer_drop")?;
        let init = instance.get_typed_func(&mut store, "init_ffi").ok();
        let decode = instance.get_typed_func(&mut store, "decode_ffi").ok();
        let memory = instance
            .get_memory(&mut store, "memory")
            .context("no memory")?;

        Ok(Instance {
            alloc,
            dealloc,
            buffer_iterator_next,
            buffer_iterator_drop,
            buffer_drop,
            init,
            decode,
            memory,
            store,
            functions,
            cached_alloc_ptr: None,
            cached_alloc_len: None,
            stdout,
            stderr,
        })
    }

    /// Call a scalar function.
    pub fn call_scalar_function(&mut self, name: &str, input: &[u8]) -> Result<(&[u8], u32)> {
        // get function
        let func = self
            .functions
            .get(name)
            .with_context(|| format!("function not found: {name}"))?;

        // allocate memory for input buffer and output struct
        let len = u32::try_from(input.len() + 4 * 2).context("input too large")?;
        let alloc_len = match self.cached_alloc_len {
            Some(cached_len) => {
                if cached_len < len {
                    // resize cached_alloc_ptr if input size is larger than cached size.
                    self.dealloc.call(
                        &mut self.store,
                        (self.cached_alloc_ptr.unwrap(), cached_len, INPUT_ALIGNMENT),
                    )?;
                    self.cached_alloc_ptr =
                        Some(self.alloc.call(&mut self.store, (len, INPUT_ALIGNMENT))?);
                    len
                } else {
                    cached_len
                }
            }
            None => {
                self.cached_alloc_len = Some(len);
                len
            }
        };
        let alloc_ptr = match self.cached_alloc_ptr {
            Some(ptr) => ptr,
            None => {
                let ptr = self
                    .alloc
                    .call(&mut self.store, (alloc_len, INPUT_ALIGNMENT))?;
                self.cached_alloc_ptr = Some(ptr);
                ptr
            }
        };
        ensure!(alloc_ptr != 0, "failed to allocate for input");
        let in_ptr = alloc_ptr + 4 * 2;
        // write input to memory
        self.memory.write(&mut self.store, in_ptr as usize, input)?;

        // call the function
        let result = func.call(&mut self.store, (in_ptr, input.len() as u32, alloc_ptr));
        let errno = self.append_stdio(result)?;

        // get return values
        let out_ptr = self.read_u32(alloc_ptr)?;
        let out_len = self.read_u32(alloc_ptr + 4)?;

        // read output from memory
        let out_bytes = self
            .memory
            .data(&self.store)
            .get(out_ptr as usize..(out_ptr + out_len) as usize)
            .context("output slice out of bounds")?;
        let result = match errno {
            0 => Ok(out_bytes),
            _ => Err(anyhow!(
                "error number: {}, out bytes: {}",
                errno,
                std::str::from_utf8(out_bytes)?
            )),
        };
        // Return both the host-accessible slice and the WASM pointer.
        result.map(|o| (o, out_ptr))
    }

    /// Call a generic function that returns an iterator of Buffers. Those buffers together form an Arrow Array.
    pub fn call_generic_function(
        &mut self,
        name: &str,
        input: &[u8],
        instance_arc: Arc<Mutex<Instance>>,
    ) -> Result<impl Iterator<Item = Buffer>> {
        // allocate memory for input buffer and output struct
        let len = u32::try_from(input.len() + 4 * 3).context("input too large")?;
        // The following comment is deprecated. Host must dealloc the mem it alloc to have no bugs.
        // Always alloc, never free. It is the lib's responsibility to free this memory,
        // because the wasm decoding lib may zero-copy the data.
        // let alloc_ptr = self.alloc.call(&mut self.store, (len, INPUT_ALIGNMENT))?;
        let alloc_len = match self.cached_alloc_len {
            Some(cached_len) => {
                if cached_len < len {
                    // resize cached_alloc_ptr if input size is larger than cached size.
                    self.dealloc.call(
                        &mut self.store,
                        (self.cached_alloc_ptr.unwrap(), cached_len, INPUT_ALIGNMENT),
                    )?;
                    self.cached_alloc_ptr =
                        Some(self.alloc.call(&mut self.store, (len, INPUT_ALIGNMENT))?);
                    len
                } else {
                    cached_len
                }
            }
            None => {
                self.cached_alloc_len = Some(len);
                len
            }
        };
        let alloc_ptr = match self.cached_alloc_ptr {
            Some(ptr) => ptr,
            None => {
                let res = self
                    .alloc
                    .call(&mut self.store, (alloc_len, INPUT_ALIGNMENT));
                let ptr = match res {
                    Ok(ptr) => ptr,
                    Err(err) => {
                        return Err(err);
                    }
                };
                self.cached_alloc_ptr = Some(ptr);
                ptr
            }
        };
        ensure!(alloc_ptr != 0, "failed to allocate for input");
        let in_ptr = alloc_ptr + 4 * 3;
        // write input to memory
        self.memory.write(&mut self.store, in_ptr as usize, input)?;

        // get function
        let func = self
            .functions
            .get(name)
            .with_context(|| format!("function not found: {name}"))?;
        // call the function
        let result = func.call(&mut self.store, (in_ptr, input.len() as u32, alloc_ptr));
        // The following is for debugging uses.
        // if result.is_err() {
        //     let err = result.as_ref().unwrap_err();
        //     assert!(err.is::<Trap>());
        //     println!("Extract the captured core dump.");
        //     let dump = err
        //         .downcast_ref::<WasmCoreDump>()
        //         .expect("should have an attached core dump, since we configured core dumps on");
        //     println!("{}", dump);
        //     println!(
        //         "Number of memories in the core dump: {}",
        //         dump.memories().len()
        //     );
        //     for (i, mem) in dump.memories().iter().enumerate() {
        //         if let Some(addr) = mem.data(self.store()).iter().position(|byte| *byte != 0) {
        //             let val = mem.data(self.store())[addr];
        //             println!("  First nonzero byte for memory {i}: {val} @ {addr:#x}");
        //         } else {
        //             println!("  Memory {i} is all zeroes.");
        //         }
        //     }

        //     println!(
        //         "Number of globals in the core dump: {}",
        //         dump.globals().len()
        //     );
        //     for (i, global) in dump.globals().iter().enumerate() {
        //         let val = global.get(self.store());
        //         println!("  Global {i} = {val:?}");
        //     }

        //     println!("Serialize the core dump and write it to ./example.coredump");
        //     let serialized = dump.serialize(self.store(), "trapper.wasm");
        //     std::fs::write("./example.coredump", serialized)?;
        // }
        let errno = self.append_stdio(result)?;

        // get return values
        let out_ptr = self.read_u32(alloc_ptr)?;
        let out_len = self.read_u32(alloc_ptr + 4)?;

        // read output from memory
        let out_bytes = self
            .memory
            .data(&self.store)
            .get(out_ptr as usize..(out_ptr + out_len) as usize)
            .context("output slice out of bounds")?;
        let ptr = match errno {
            0 => out_ptr,
            _ => {
                return Err(anyhow!(
                    "error number: {}, out bytes: {}",
                    errno,
                    std::str::from_utf8(out_bytes)?
                ))
            }
        };
        Ok(BufferIter {
            ptr,
            alloc_ptr,
            instance_arc,
        })
    }

    /// Call the adv init API
    pub fn call_init(&mut self, input: &[u8], kwargs: &[u8]) -> Result<WasmSlice> {
        // allocate memory for input buffer and output struct
        let len = u32::try_from(input.len() + kwargs.len() + 4 * 3).context("input too large")?;
        // The following comment is deprecated. Host must dealloc the mem it alloc to have no bugs.
        // Always alloc, never free. It is the lib's responsibility to free this memory,
        // because the wasm decoding lib may zero-copy the data.
        // let alloc_ptr = self.alloc.call(&mut self.store, (len, INPUT_ALIGNMENT))?;
        let alloc_len = match self.cached_alloc_len {
            Some(cached_len) => {
                if cached_len < len {
                    // resize cached_alloc_ptr if input size is larger than cached size.
                    self.dealloc.call(
                        &mut self.store,
                        (self.cached_alloc_ptr.unwrap(), cached_len, INPUT_ALIGNMENT),
                    )?;
                    self.cached_alloc_ptr =
                        Some(self.alloc.call(&mut self.store, (len, INPUT_ALIGNMENT))?);
                    len
                } else {
                    cached_len
                }
            }
            None => {
                self.cached_alloc_len = Some(len);
                len
            }
        };
        let alloc_ptr = match self.cached_alloc_ptr {
            Some(ptr) => ptr,
            None => {
                let res = self
                    .alloc
                    .call(&mut self.store, (alloc_len, INPUT_ALIGNMENT));
                let ptr = match res {
                    Ok(ptr) => ptr,
                    Err(err) => {
                        return Err(err);
                    }
                };
                self.cached_alloc_ptr = Some(ptr);
                ptr
            }
        };
        ensure!(alloc_ptr != 0, "failed to allocate for input");
        let in_ptr = alloc_ptr + 4 * 3;
        // write input to memory
        self.memory.write(&mut self.store, in_ptr as usize, input)?;
        let kwargs_ptr = alloc_ptr + 4 * 3 + input.len() as u32;
        // write kwargs to memory
        self.memory
            .write(&mut self.store, kwargs_ptr as usize, kwargs)?;

        // call the function
        let result = self.init.as_ref().unwrap().call(
            &mut self.store,
            (
                in_ptr,
                input.len() as u32,
                kwargs_ptr,
                kwargs.len() as u32,
                alloc_ptr,
            ),
        );
        let errno = self.append_stdio(result)?;

        // get return values
        let out_ptr = self.read_u32(alloc_ptr)?;
        let out_len = self.read_u32(alloc_ptr + 4)?;

        // read output from memory
        let out_bytes = self
            .memory
            .data(&self.store)
            .get(out_ptr as usize..(out_ptr + out_len) as usize)
            .context("output slice out of bounds")?;
        let ptr = match errno {
            0 => out_ptr,
            _ => {
                return Err(anyhow!(
                    "error number: {}, out bytes: {}",
                    errno,
                    std::str::from_utf8(out_bytes)?
                ))
            }
        };
        Ok(WasmSlice { ptr, len: out_len })
    }

    /// Call the adv Decode
    /// TODO: majority of the code is the same as call_generic_function, except the input is removed.
    /// And we call Decode now, with return number 1 indicating it is None for the Option
    pub fn call_decode(
        &mut self,
        decoder: u32,
        instance_arc: Arc<Mutex<Instance>>,
    ) -> Result<Option<impl Iterator<Item = Buffer>>> {
        // allocate memory for output struct
        let len = u32::try_from(4 * 3).context("input too large")?;
        // The following comment is deprecated. Host must dealloc the mem it alloc to have no bugs.
        // Always alloc, never free. It is the lib's responsibility to free this memory,
        // because the wasm decoding lib may zero-copy the data.
        // let alloc_ptr = self.alloc.call(&mut self.store, (len, INPUT_ALIGNMENT))?;
        let alloc_len = match self.cached_alloc_len {
            Some(cached_len) => {
                if cached_len < len {
                    // resize cached_alloc_ptr if input size is larger than cached size.
                    self.dealloc.call(
                        &mut self.store,
                        (self.cached_alloc_ptr.unwrap(), cached_len, INPUT_ALIGNMENT),
                    )?;
                    self.cached_alloc_ptr =
                        Some(self.alloc.call(&mut self.store, (len, INPUT_ALIGNMENT))?);
                    len
                } else {
                    cached_len
                }
            }
            None => {
                self.cached_alloc_len = Some(len);
                len
            }
        };
        let alloc_ptr = match self.cached_alloc_ptr {
            Some(ptr) => ptr,
            None => {
                let res = self
                    .alloc
                    .call(&mut self.store, (alloc_len, INPUT_ALIGNMENT));
                let ptr = match res {
                    Ok(ptr) => ptr,
                    Err(err) => {
                        return Err(err);
                    }
                };
                self.cached_alloc_ptr = Some(ptr);
                ptr
            }
        };
        ensure!(alloc_ptr != 0, "failed to allocate for input");

        // call the function
        let result = self
            .decode
            .as_ref()
            .unwrap()
            .call(&mut self.store, (decoder, alloc_ptr));

        let errno = self.append_stdio(result)?;

        // get return values
        let out_ptr = self.read_u32(alloc_ptr)?;
        let out_len = self.read_u32(alloc_ptr + 4)?;

        // read output from memory
        let out_bytes = self
            .memory
            .data(&self.store)
            .get(out_ptr as usize..(out_ptr + out_len) as usize)
            .context("output slice out of bounds")?;
        let ptr = match errno {
            0 => Some(out_ptr),
            1 => None,
            _ => {
                return Err(anyhow!(
                    "error number: {}, out bytes: {}",
                    errno,
                    std::str::from_utf8(out_bytes)?
                ))
            }
        };
        Ok(ptr.map(|ptr| BufferIter {
            ptr,
            alloc_ptr,
            instance_arc,
        }))
    }

    #[allow(unreachable_code)]
    pub fn read_batch(
        &mut self,
        _name: &str,
        _input: &[u8],
        _selection: RowSelection,
        _instance_arc: Arc<Mutex<Instance>>,
    ) -> Result<Option<impl Iterator<Item = Buffer>>> {
        todo!();
        // get function
        let func = self
            .functions
            .get(_name)
            .with_context(|| format!("function not found: {_name}"))?;

        // allocate memory for input buffer and output struct
        let len = u32::try_from(_input.len() + 4 * 2).context("input too large")?;
        let alloc_len = match self.cached_alloc_len {
            Some(cached_len) => {
                if cached_len < len {
                    // resize cached_alloc_ptr if input size is larger than cached size.
                    self.dealloc.call(
                        &mut self.store,
                        (self.cached_alloc_ptr.unwrap(), cached_len, INPUT_ALIGNMENT),
                    )?;
                    self.cached_alloc_ptr =
                        Some(self.alloc.call(&mut self.store, (len, INPUT_ALIGNMENT))?);
                    len
                } else {
                    cached_len
                }
            }
            None => {
                self.cached_alloc_len = Some(len);
                len
            }
        };
        let alloc_ptr = match self.cached_alloc_ptr {
            Some(ptr) => ptr,
            None => {
                let ptr = self
                    .alloc
                    .call(&mut self.store, (alloc_len, INPUT_ALIGNMENT))?;
                self.cached_alloc_ptr = Some(ptr);
                ptr
            }
        };
        ensure!(alloc_ptr != 0, "failed to allocate for input");
        let in_ptr = alloc_ptr + 4 * 2;
        // write input to memory
        self.memory
            .write(&mut self.store, in_ptr as usize, _input)?;

        // call the function
        let result = func.call(&mut self.store, (in_ptr, _input.len() as u32, alloc_ptr));
        let errno = self.append_stdio(result)?;

        // get return values
        let out_ptr = self.read_u32(alloc_ptr)?;
        let out_len = self.read_u32(alloc_ptr + 4)?;

        // read output from memory
        let out_bytes = self
            .memory
            .data(&self.store)
            .get(out_ptr as usize..(out_ptr + out_len) as usize)
            .context("output slice out of bounds")?;
        let ptr = match errno {
            0 => out_ptr,
            _ => {
                return Err(anyhow!(
                    "error number: {}, out bytes: {}",
                    errno,
                    std::str::from_utf8(out_bytes)?
                ))
            }
        };
        Ok(Some(BufferIter {
            ptr,
            alloc_ptr,
            instance_arc: _instance_arc,
        }))
    }

    pub fn dealloc(&mut self, ptr: u32, len: u32, align: u32) -> Result<()> {
        self.dealloc.call(&mut self.store, (ptr, len, align))?;
        Ok(())
    }

    // pub fn array_drop(&mut self, ptr: u32) -> Result<()> {
    //     self.array_drop.call(&mut self.store, ptr)?;
    //     Ok(())
    // }

    fn buffer_iterator_next(&mut self, ptr: u32, alloc_ptr: u32) -> Result<()> {
        self.buffer_iterator_next
            .call(&mut self.store, (ptr, alloc_ptr, alloc_ptr + 8))?;
        Ok(())
    }

    fn buffer_iterator_drop(&mut self, ptr: u32) -> Result<()> {
        self.buffer_iterator_drop.call(&mut self.store, ptr)?;
        Ok(())
    }

    pub fn print_stdio(&self) {
        self.stdout.print();
    }

    pub fn buffer_drop(&mut self, ptr: u32) -> Result<()> {
        self.buffer_drop.call(&mut self.store, ptr)?;
        Ok(())
    }

    /// WARNING: This function is for testing only.
    pub fn memory_size(&self) -> usize {
        self.memory.data_size(&self.store)
    }

    pub fn store(&mut self) -> &mut Store<(WasiCtx, StoreLimits)> {
        &mut self.store
    }

    /// Read a `u32` from memory.
    fn read_u32(&mut self, ptr: u32) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.memory.data(&self.store)[ptr as usize..(ptr + 4) as usize]
                .try_into()
                .unwrap(),
        ))
    }

    /// Take stdout and stderr, append to the error context.
    fn append_stdio<T>(&self, result: Result<T>) -> Result<T> {
        match result {
            Ok(v) => Ok(v),
            Err(e) => {
                let stdout = self.stdout.take();
                let stderr = self.stderr.take();
                Err(e.context(format!(
                    "--- stdout\n{}\n--- stderr\n{}",
                    String::from_utf8_lossy(&stdout),
                    String::from_utf8_lossy(&stderr),
                )))
            }
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        if let Some(ptr) = self.cached_alloc_ptr {
            // deallocate memory
            self.dealloc
                .call(
                    &mut self.store,
                    (ptr, self.cached_alloc_len.unwrap(), INPUT_ALIGNMENT),
                )
                .unwrap();
        }
    }
}

/// Decode a string from symbol name using customized base64.
fn _base64_decode(input: &str) -> Result<String> {
    use base64::{
        alphabet::Alphabet,
        engine::{general_purpose::NO_PAD, GeneralPurpose},
        Engine,
    };
    // standard base64 uses '+' and '/', which is not a valid symbol name.
    // we use '$' and '_' instead.
    let alphabet =
        Alphabet::new("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789$_").unwrap();
    let engine = GeneralPurpose::new(&alphabet, NO_PAD);
    let bytes = engine.decode(input)?;
    String::from_utf8(bytes).context("invalid utf8")
}

// fn encode_record_batch(batch: &RecordBatch) -> Result<Vec<u8>> {
//     let mut buf = vec![];
//     let mut writer = arrow_ipc::writer::FileWriter::try_new(&mut buf, &batch.schema())?;
//     writer.write(batch)?;
//     writer.finish()?;
//     drop(writer);
//     Ok(buf)
// }

// fn decode_record_batch(bytes: &[u8]) -> Result<RecordBatch> {
//     let mut reader = arrow_ipc::reader::FileReader::try_new(std::io::Cursor::new(bytes), None)?;
//     let batch = reader.next().unwrap()?;
//     Ok(batch)
// }

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use arrow_array::{ArrayRef, UInt32Array};
    use fff_core::util::buffer_to_array::primitive_array_from_arrow_buffers_iter;
    use wasm_test_encoders::encode_fff_general;
    use wasmtime::Engine;

    use crate::{Config, Instance, Runtime};

    #[test]
    #[ignore]
    fn test() {
        let engine =
            Engine::new(wasmtime::Config::new().profiler(wasmtime::ProfilingStrategy::None))
                .unwrap();
        let rt = Arc::new(
            Runtime::with_config_engine(
                &std::fs::read(fff_test_util::BP_WASM_PATH.as_path()).unwrap(),
                Config::default(),
                &engine,
            )
            .unwrap(),
        );
        let mut instance = Instance::new(&rt).unwrap();
        for _ in 0..20 {
            // allocate memory for input buffer and output struct
            let alloc_len = 8192;
            let alloc_ptr = instance
                .alloc
                .call(&mut instance.store, (alloc_len, 4))
                .unwrap();
            println!("size: {:?}", instance.memory.data_size(&instance.store));
            // deallocate memory
            instance
                .dealloc
                .call(&mut instance.store, (alloc_ptr, alloc_len, 4))
                .unwrap();
            println!("size: {:?}", instance.memory.data_size(&instance.store));
        }
    }

    #[test]
    #[ignore]
    fn test_adv() {
        let engine =
            Engine::new(wasmtime::Config::new().profiler(wasmtime::ProfilingStrategy::None))
                .unwrap();
        let rt = Arc::new(
            Runtime::with_config_engine(
                &std::fs::read(
                    "/home/xinyu/fff-devel/target/wasm32-wasip1/opt-size-lvl3/adv_ude_fff.wasm",
                )
                .unwrap(),
                Config::default(),
                &engine,
            )
            .unwrap(),
        );
        let instance = Arc::new(Mutex::new(Instance::new(&rt).unwrap()));
        let mut guard = instance.lock().unwrap();

        let full_size = 65536;
        let int_data: Vec<u32> = (0..full_size).map(|x| x as u32 % 128).collect();
        let array = Arc::new(UInt32Array::from(int_data.clone())) as ArrayRef;
        let encoded = encode_fff_general(array.clone());
        let slice = guard.call_init(&encoded, &[]).unwrap();
        let iter = guard
            .call_decode(slice.ptr, instance.clone())
            .unwrap()
            .unwrap();
        drop(guard);
        let out =
            primitive_array_from_arrow_buffers_iter(array.data_type(), iter, full_size).unwrap();
        assert_eq!(*array, *out);
    }
}
