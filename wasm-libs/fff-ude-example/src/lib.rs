use bytes::Bytes;
use fff_encoding::schemes::{bp::BPDecoder, Decoder};
use fff_ude::ffi::scalar_wrapper;
use fff_ude::Result;

fn decode_bp(input: &[u8]) -> Result<Box<[u8]>> {
    // FIXME: more elegant way to avoid copy?
    let bytes = unsafe {
        Bytes::from(Vec::from_raw_parts(
            input.as_ptr() as *mut u8,
            input.len(),
            input.len(),
        ))
    };
    let mut decoder = BPDecoder::new(bytes.clone());
    // Const ptr should not be dropped
    std::mem::forget(bytes);
    // let mut decoder = BPDecoder::new(Bytes::from(input.to_vec()));
    let output = decoder.decode_all()?;
    let mut first_buffer = output.into_iter().next().unwrap().into_mutable().unwrap();
    let vec = unsafe {
        Vec::<u8>::from_raw_parts(
            first_buffer.as_mut_ptr(),
            first_buffer.len(),
            first_buffer.capacity(),
        )
    };
    std::mem::forget(first_buffer);
    Ok(vec.into_boxed_slice())
    // Ok(Vec::<u8>::from(output.into_iter().next().unwrap()).into_boxed_slice())
}

/// TODO: use macro to generate
#[no_mangle]
pub unsafe extern "C" fn decode_bp_ffi(
    ptr: *mut u8,
    len: usize,
    out: *mut fff_ude::ffi::CSlice,
) -> i32 {
    scalar_wrapper(decode_bp, ptr, len, out)
}
