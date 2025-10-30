use fff_ude::ffi::scalar_wrapper;
use fff_ude::Result;

/// WARNING: This function is only for testing purposes. Caller should forget the returned Box.
fn noop(_input: &[u8]) -> Result<Box<[u8]>> {
    // create some buffer to see if wasm can automatically free it
    // let some_buffer = vec![0u8; 1024 * 1024 * 1024];
    // black_box(some_buffer);
    // Th answer is yes.
    // let vec = Vec::new();
    let vec = unsafe { Vec::from_raw_parts(std::ptr::null_mut::<u8>(), 0, 0) };
    Ok(vec.into_boxed_slice())
}

/// TODO: use macro to generate
#[no_mangle]
pub unsafe extern "C" fn noop_ffi(
    ptr: *const u8,
    len: usize,
    out: *mut fff_ude::ffi::CSlice,
) -> i32 {
    scalar_wrapper(noop, ptr, len, out)
}

#[no_mangle]
pub unsafe extern "C" fn true_noop(_dummy: usize) {}
