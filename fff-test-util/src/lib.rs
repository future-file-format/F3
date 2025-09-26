use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static BASE_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest_dir)
        .parent()
        .expect("Failed to get parent directory of CARGO_MANIFEST_DIR")
        .to_path_buf()
});

pub static NOOP_PATH: LazyLock<PathBuf> =
    LazyLock::new(|| BASE_PATH.join("target/wasm32-wasip1/release/fff_ude_example_noop.wasm"));
pub const NOOP_FUNC: &str = "noop_ffi";

pub static MEM_TEST_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    BASE_PATH.join("target/wasm32-wasip1/release/fff_ude_example_memory_test.wasm")
});
pub const MEM_TEST_FUNC: &str = "test_ffi";

pub static BP_WASM_PATH: LazyLock<PathBuf> =
    LazyLock::new(|| BASE_PATH.join("target/wasm32-wasip1/release/fff_ude_example.wasm"));
pub const BP_WASM_FUNC: &str = "decode_bp_ffi";

pub static VORTEX_WASM_PATH: LazyLock<PathBuf> =
    LazyLock::new(|| BASE_PATH.join("target/wasm32-wasip1/release/fff_ude_example2.wasm"));
pub const VORTEX_WASM_FUNC: &str = "decode_vortex_ffi";
pub const VORTEX_WASM_FUNC_GENERAL: &str = "decode_vortex_general_ffi";

pub static BUILTIN_WASM_PATH: LazyLock<PathBuf> =
    LazyLock::new(|| BASE_PATH.join("target/wasm32-wasip1/opt-size-lvl3/fff_ude_example_fff.wasm"));
pub const WASM_FUNC_GENERAL: &str = "decode_general_ffi";

pub const TEST_SCHEMES: [&str; 6] = ["pco", "lz4", "flsbp", "fff", "gzip", "zstd"];
