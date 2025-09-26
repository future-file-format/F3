use fff_ude::ffi::scalar_wrapper;
use fff_ude::Result;

fn test(input: &[u8]) -> Result<Box<[u8]>> {
    // read i32 from input
    let size: u32 = u32::from_le_bytes(input[0..4].try_into().unwrap());
    let vec = vec![0; size as usize];
    Ok(vec.into_boxed_slice())
}

/// TODO: use macro to generate
#[no_mangle]
pub unsafe extern "C" fn test_ffi(
    ptr: *const u8,
    len: usize,
    out: *mut fff_ude::ffi::CSlice,
) -> i32 {
    scalar_wrapper(test, ptr, len, out)
}
