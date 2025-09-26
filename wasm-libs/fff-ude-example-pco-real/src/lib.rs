use fff_ude::ffi::general_wrapper;
use wasm_test_encoders::decode_pco_real_general;

#[no_mangle]
pub unsafe extern "C" fn decode_general_ffi(
    ptr: *const u8,
    len: usize,
    out: *mut fff_ude::ffi::CSlice,
) -> i32 {
    general_wrapper(decode_pco_real_general, ptr, len, out)
}
