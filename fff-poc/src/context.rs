use std::{
    collections::HashMap,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, OnceLock},
};

use arrow_schema::DataType;
use fff_format::File::fff::flatbuf as fb;
use fff_test_util::BUILTIN_WASM_PATH;
use fff_ude_wasm::Runtime;
use semver::Version;

use crate::{file::footer::MetadataSection, io::reader::Reader};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct WASMId(pub u32);

#[derive(Debug, PartialEq, Clone)]
pub struct WasmLib {
    encode_lib_path: Rc<PathBuf>,
    decode_wasm_binary: Rc<Vec<u8>>,
}

impl WasmLib {
    pub fn new(enc_path: PathBuf, dec_wasm: Vec<u8>) -> Self {
        Self {
            encode_lib_path: Rc::new(enc_path),
            decode_wasm_binary: Rc::new(dec_wasm),
        }
    }

    pub fn encode_lib_path(&self) -> Rc<PathBuf> {
        self.encode_lib_path.clone()
    }
}

/// Behavior is a little weird for the research use now. We either use default_with_always_set_custom_wasm() to write all built-in as wasm,
/// or we set built-in as native and allow custom wasm.
#[derive(Debug)]
pub struct WASMWritingContext {
    /// WASMId to its binaries
    wasms: HashMap<WASMId, WasmLib>,
    /// DataType to its WASMId
    data_type_to_wasm_id: HashMap<DataType, WASMId>,
    /// Always write CUSTOM_WASM encoding, this is mainly for testing
    always_set_custom_wasm_for_built_in: bool,
    /// WasmId for built-in
    builtin_wasm_id: Option<WASMId>,
}

impl Default for WASMWritingContext {
    fn default() -> Self {
        Self {
            // TODO: allow custom wasm path
            wasms: HashMap::from([(
                WASMId(0),
                WasmLib {
                    encode_lib_path: PathBuf::from("/").into(),
                    decode_wasm_binary: std::fs::read(BUILTIN_WASM_PATH.as_path()).unwrap().into(),
                },
            )]),
            data_type_to_wasm_id: HashMap::default(),
            always_set_custom_wasm_for_built_in: false,
            builtin_wasm_id: Some(WASMId(0)),
        }
    }
}

impl WASMWritingContext {
    pub fn default_with_always_set_custom_wasm() -> Self {
        Self {
            always_set_custom_wasm_for_built_in: true,
            ..Self::default()
        }
    }

    pub fn empty() -> Self {
        Self {
            wasms: HashMap::new(),
            data_type_to_wasm_id: HashMap::new(),
            always_set_custom_wasm_for_built_in: false,
            builtin_wasm_id: None,
        }
    }

    /// Built-in Wasm will not be included in the final WasmIds
    pub fn with_custom_wasms(
        wasms: HashMap<WASMId, WasmLib>,
        data_type_to_wasm_id: HashMap<DataType, WASMId>,
    ) -> Self {
        Self {
            wasms,
            data_type_to_wasm_id,
            always_set_custom_wasm_for_built_in: false,
            builtin_wasm_id: None,
        }
    }

    pub fn get_sorted_wasms(&self) -> Vec<&[u8]> {
        let mut wasms = self.wasms.iter().collect::<Vec<_>>();
        wasms.sort_by_key(|(k, _)| k.0);
        wasms
            .into_iter()
            .map(|(_, v)| v.decode_wasm_binary.as_slice())
            .collect()
    }

    pub fn data_type_to_wasm_id(&self, dt: &DataType) -> Option<WASMId> {
        self.data_type_to_wasm_id.get(dt).copied()
    }

    pub fn data_type_to_wasm_lib(&self, dt: &DataType) -> Option<WasmLib> {
        self.data_type_to_wasm_id
            .get(dt)
            .and_then(|x| self.wasms.get(x).cloned())
    }

    pub fn always_set_custom_wasm_for_built_in(&self) -> bool {
        self.always_set_custom_wasm_for_built_in
    }

    pub fn builtin_wasm_id(&self) -> Option<WASMId> {
        self.builtin_wasm_id
    }
}

pub struct WASMReadingContext<R> {
    /// runtime
    lazy_wasm: OnceLock<HashMap<WASMId, Arc<Runtime>>>,
    wasm_locations: Option<MetadataSection>,
    r: Option<R>,
    /// Mapping of encoding types to their semantic versions
    encoding_versions: Option<HashMap<fb::EncodingType, Version>>,
}

impl<R: Reader> WASMReadingContext<R> {
    // Private constructor to reduce code duplication
    fn new_internal(
        lazy_wasm: OnceLock<HashMap<WASMId, Arc<Runtime>>>,
        wasm_locations: Option<MetadataSection>,
        r: Option<R>,
        encoding_versions: Option<HashMap<fb::EncodingType, Version>>,
    ) -> Self {
        Self {
            lazy_wasm,
            wasm_locations,
            r,
            encoding_versions,
        }
    }

    // For lazy loading from file
    pub fn new(wasm_locations: MetadataSection, r: R) -> Self {
        Self::new_with_versions(wasm_locations, r, None)
    }

    pub fn new_with_versions(
        wasm_locations: MetadataSection,
        r: R,
        encoding_versions: Option<HashMap<fb::EncodingType, Version>>,
    ) -> Self {
        Self::new_internal(
            OnceLock::new(),
            Some(wasm_locations),
            Some(r),
            encoding_versions,
        )
    }

    // For pre-built runtimes
    pub fn new_with_rt(wasm_rts: HashMap<WASMId, Arc<Runtime>>) -> Self {
        Self::new_with_rt_and_versions(wasm_rts, None)
    }

    pub fn new_with_rt_and_versions(
        wasm_rts: HashMap<WASMId, Arc<Runtime>>,
        encoding_versions: Option<HashMap<fb::EncodingType, Version>>,
    ) -> Self {
        let lazy_wasm = OnceLock::new();
        lazy_wasm.get_or_init(|| wasm_rts);
        Self::new_internal(lazy_wasm, None, None, encoding_versions)
    }

    pub fn get_runtime(&self, wasm_id: WASMId) -> Arc<Runtime> {
        self.lazy_wasm
            .get_or_init(|| {
                let wasm_locations = self.wasm_locations.as_ref().unwrap();
                let mut wasms = HashMap::new();
                let mut buf = vec![0; wasm_locations.size as usize];
                let read = self.r.as_ref().unwrap();
                read.read_exact_at(&mut buf, wasm_locations.offset).unwrap();
                let wasm_binaries = flatbuffers::root::<fb::WASMBinaries>(&buf).unwrap();
                for (id, loc) in wasm_binaries.wasm_binaries().unwrap().iter().enumerate() {
                    let mut buf: Vec<u8> = vec![0; loc.size_() as usize];
                    read.read_exact_at(&mut buf, loc.offset()).unwrap();
                    let wasm_id = WASMId(id as u32);
                    // let start = std::time::Instant::now();
                    let rt = Arc::new(Runtime::try_new(&buf).unwrap());
                    // println!("WASM runtime creation time: {:?}", start.elapsed());
                    wasms.insert(wasm_id, rt);
                }
                wasms
            })
            .get(&wasm_id)
            .unwrap()
            .clone()
    }

    pub fn get_encoding_versions(&self) -> Option<&HashMap<fb::EncodingType, Version>> {
        self.encoding_versions.as_ref()
    }
}
