use std::vec;

use arrow_buffer::{Buffer, MutableBuffer};
use arrow_data::ArrayData;
use bytes::Bytes;
use fff_encoding::schemes::{vortex::VortexDecoder, Decoder};
use fff_ude::ffi::{general_wrapper, scalar_wrapper};
use fff_ude::Result;
use vortex_sampling_compressor::ALL_ENCODINGS_CONTEXT;

fn decode_vortex(input: &[u8]) -> Result<Box<[u8]>> {
    // FIXME: more elegant way to avoid copy?
    let bytes = unsafe {
        Bytes::from(Vec::from_raw_parts(
            input.as_ptr() as *mut u8,
            input.len(),
            input.len(),
        ))
    };
    let mut decoder = VortexDecoder::try_new(bytes.clone(), ALL_ENCODINGS_CONTEXT.clone()).unwrap();
    // Const ptr should not be dropped
    std::mem::forget(bytes);
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
}

// /// TODO: use macro to generate
#[no_mangle]
pub unsafe extern "C" fn decode_vortex_ffi(
    ptr: *const u8,
    len: usize,
    out: *mut fff_ude::ffi::CSlice,
) -> i32 {
    scalar_wrapper(decode_vortex, ptr, len, out)
}

fn arraydata_to_buffers(res: &mut Vec<Buffer>, array_data: &ArrayData) {
    res.push(match array_data.nulls() {
        Some(nulls) => nulls.buffer().clone(),
        None => MutableBuffer::new(0).into(),
    });
    for buffer in array_data.buffers() {
        res.push(buffer.clone());
    }
    for child in array_data.child_data() {
        arraydata_to_buffers(res, child);
    }
}

fn decode_vortex_general(input: &[u8]) -> Result<Box<dyn Iterator<Item = Buffer>>> {
    // FIXME: more elegant way to avoid copy?
    let bytes = unsafe {
        Bytes::from(Vec::from_raw_parts(
            input.as_ptr() as *mut u8,
            input.len(),
            input.len(),
        ))
    };
    let mut decoder = VortexDecoder::try_new(
        bytes.clone(),
        ALL_ENCODINGS_CONTEXT.clone(), // vortex::Context::default().into(),
                                       // vortex::Context::empty()
                                       //     .with_encodings([&FSSTEncoding as EncodingRef])
                                       //     .into(),
    )
    .unwrap();
    // Const ptr should not be dropped
    std::mem::forget(bytes);
    let output = decoder.decode_all_as_array()?;
    let mut res: Vec<Buffer> = vec![];
    arraydata_to_buffers(&mut res, &output.to_data());
    Ok(Box::new(res.into_iter()))
}

#[no_mangle]
pub unsafe extern "C" fn decode_vortex_general_ffi(
    ptr: *const u8,
    len: usize,
    out: *mut fff_ude::ffi::CSlice,
) -> i32 {
    general_wrapper(decode_vortex_general, ptr, len, out)
}
