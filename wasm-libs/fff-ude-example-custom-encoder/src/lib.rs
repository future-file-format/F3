use arrow_array::ffi::{FFI_ArrowArray, FFI_ArrowSchema};
use uniffi_core::RustBuffer;
use wasm_test_encoders::encode_custom_c;

#[no_mangle]
pub unsafe extern "C" fn encode(input: FFI_ArrowArray, schema: FFI_ArrowSchema) -> RustBuffer {
    encode_custom_c(input, schema)
}
