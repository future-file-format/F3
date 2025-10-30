//! A very simple Wasm lib to test the output size of Wasm between Wasi and unknown

#[unsafe(no_mangle)]
pub unsafe extern "C" fn decode_vortex_general_ffi(input: i32) -> i32 {
    if input == 0 {
        1
    } else {
        input * decode_vortex_general_ffi(input - 1)
    }
}
