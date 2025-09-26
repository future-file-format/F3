use crate::base64_decode;
use fff_core::{general_err, errors::{Error, Result}};
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::sync::Mutex;
use wasmer::*;
use wasmer_wasix::WasiTtyState;

/// Decode a string from symbol name using customized base64.
fn base64_decode(input: &str) -> Result<String> {
    use base64::{
        alphabet::Alphabet,
        engine::{general_purpose::NO_PAD, GeneralPurpose},
        Engine,
    };
    // standard base64 uses '+' and '/', which is not a valid symbol name.
    // we use '$' and '_' instead.
    let alphabet =
        Alphabet::new("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789$_")
            .map_err(|e| Error::General(format!("Failed to create alphabet: {}", e)))?;
    let engine = GeneralPurpose::new(&alphabet, NO_PAD);
    let bytes = engine
        .decode(input)
        .map_err(|e| Error::General(format!("Failed to decode base64: {}", e)))?;
    String::from_utf8(bytes).map_err(|e| Error::General(format!("Invalid utf8: {}", e)))
}

/// The WASM encoders/decoders runtime.
///
/// This runtime contains an instance pool and can be shared by multiple threads.
pub struct Runtime {
    module: Module,
    /// Configurations.
    config: Config,
    /// Function names.
    functions: HashSet<String>,
    /// Instance pool.
    instances: Mutex<Vec<Instance>>,
}

/// Configurations.
#[derive(Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Config {
    /// Memory size limit in bytes.
    pub memory_size_limit: Option<usize>,
    /// File size limit in bytes.
    pub file_size_limit: Option<usize>,
}

struct Instance {
    // extern "C" fn(len: usize, align: usize) -> *mut u8
    alloc: Function,
    // extern "C" fn(ptr: *mut u8, len: usize, align: usize)
    dealloc: Function,
    // extern "C" fn(iter: *mut RecordBatchIter, out: *mut CSlice)
    encode: Function,
    // extern "C" fn(iter: *mut RecordBatchIter)
    decode: Function,
    // extern "C" fn(ptr: *const u8, len: usize, out: *mut CSlice) -> i32
    functions: HashMap<String, Function>,
    memory: Memory,
    store: Store,
}

impl Runtime {
    /// Create a new runtime from a WASM binary.
    pub fn new(binary: &[u8]) -> Result<Self> {
        Self::with_config(binary, Config::default())
    }

    /// Create a new runtime from a WASM binary with configuration.
    pub fn with_config(binary: &[u8], config: Config) -> Result<Self> {
        // use a global engine by default
        lazy_static::lazy_static! {
            static ref ENGINE: Engine = Engine::default();
        }
        Self::with_config_engine(binary, config, &ENGINE)
    }

    /// Create a new UDF runtime from a WASM binary with a customized engine.
    fn with_config_engine(binary: &[u8], config: Config, engine: &Engine) -> Result<Self> {
        let module = Module::from_binary(engine, binary)
            .map_err(|e| general_err!("failed to load wasm binary", e))?;

        let mut functions = HashSet::new();
        for export in module.exports() {
            let encoded = export.name();
            let name = base64_decode(encoded)
                .map_err(|e| general_err!("invalid symbol", e))?;
            functions.insert(name);
        }

        Ok(Self {
            module,
            config,
            functions,
            instances: Mutex::new(vec![]),
        })
    }
}
#[test]
fn test_mut() {
    struct Reader {
        r: i32,
        reader: u32,
    }
    struct Footer<'a> {
        a: &'a i32,
    }
    struct NoRef {
        a: i32,
    }
    let mut reader = Reader { r: 42, reader: 1 };
    fn mut_read<'a>(reader: &'a mut Reader) -> Footer<'a> {
        reader.r = 43;
        Footer { a: &reader.r }
    }
    fn mut_read_no_ref(reader: &mut Reader) -> NoRef {
        reader.r = 43;
        NoRef { a: reader.r }
    }
    fn change_r(reader: &mut Reader) {
        reader.r += 1;
    }
    fn change_reader(reader: &mut Reader) {
        reader.reader += 1;
    }
    // let footer = mut_read_no_ref(&mut reader);
    let footer = mut_read(&mut reader);
    reader.reader += 1;
    // change_reader(&mut reader);
    // change_r(&mut reader);
    // let borrowed_r = &mut reader.r;
    // *borrowed_r += 1;
    assert_eq!(footer.a, &44);
}
