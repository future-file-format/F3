use arrow_buffer::{Buffer, MutableBuffer};
use byteorder::{LittleEndian, ReadBytesExt};
use bytes::Bytes;
use fff_ude::ffi::general_wrapper;
use fff_ude::Result;
use fsst::Decompressor;

fn decode_fsst_general(input: &[u8]) -> Result<Box<dyn Iterator<Item = Buffer>>> {
    // FIXME: more elegant way to avoid copy?
    let mut bytes = unsafe {
        Bytes::from(Vec::from_raw_parts(
            input.as_ptr() as *mut u8,
            input.len(),
            input.len(),
        ))
    };
    // A very simple serde of the FSST lib
    let symbols_size = bytes.split_to(8).as_ref().read_u64::<LittleEndian>()?;
    let length_size = bytes.split_to(8).as_ref().read_u64::<LittleEndian>()?;
    let symbols = bytes.split_to(symbols_size as usize);
    let symbols = bytemuck::cast_slice(&symbols);
    let lengths = bytes.split_to(length_size as usize);
    let decompressor = Decompressor::new(symbols, &lengths);
    let output = Buffer::from(decompressor.decompress(&bytes));
    // Const ptr should not be dropped
    std::mem::forget(bytes);
    // assume no nulls
    let res = [MutableBuffer::from_len_zeroed(0).into(), output];
    Ok(Box::new(res.into_iter()))
}

#[no_mangle]
pub unsafe extern "C" fn decode_fsst_general_ffi(
    ptr: *const u8,
    len: usize,
    out: *mut fff_ude::ffi::CSlice,
) -> i32 {
    general_wrapper(decode_fsst_general, ptr, len, out)
}
