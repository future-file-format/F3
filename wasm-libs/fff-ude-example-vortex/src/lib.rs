#![feature(stmt_expr_attributes)]
use std::sync::Arc;
use std::vec;

// use arrow_array::ffi::to_ffi;
use arrow_buffer::Buffer;
use byteorder::{LittleEndian, ReadBytesExt};
use bytes::Bytes;
use fff_core::util::bit_util::padding_size;
use fff_ude::ffi::general_wrapper;
use fff_ude::{arraydata_to_buffers, Result};
#[cfg(feature = "alp")]
use vortex_alp::ALPEncoding;
use vortex_array::array::ConstantEncoding;
use vortex_array::encoding::EncodingRef;
use vortex_array::{Context, IntoCanonical};
#[cfg(feature = "fsst")]
use vortex_fsst::FSSTEncoding;
use vortex_ipc::messages::reader::{ArrayMessageReader, DTypeBufferReader};
#[cfg(feature = "runend")]
use vortex_runend::RunEndEncoding;

const VORTEX_ALIGNMENT: usize = 64;

// fn fls_bp_decompress(input: BitPackedArray) -> Result<ArrayRef> {
//     Ok(input.into_canonical()?.into_arrow()?)
// }
// #[global_allocator]
// static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

fn decode_vortex_general(input: &[u8]) -> Result<Box<dyn Iterator<Item = Buffer>>> {
    let bytes = Bytes::copy_from_slice(input);
    let encblock = &mut bytes.clone();

    #[cfg(any(feature = "runend", feature = "fsst", feature = "alp"))]
    let mut encodings: Vec<EncodingRef> = vec![&ConstantEncoding as EncodingRef];
    #[cfg(not(any(feature = "runend", feature = "fsst", feature = "alp")))]
    let encodings: Vec<EncodingRef> = vec![&ConstantEncoding as EncodingRef];
    #[cfg(feature = "runend")]
    encodings.push(&RunEndEncoding as EncodingRef);
    #[cfg(feature = "fsst")]
    encodings.push(&FSSTEncoding);
    #[cfg(feature = "alp")]
    encodings.push(&ALPEncoding as EncodingRef);

    let context: Arc<Context> = vortex_array::Context::empty()
        .with_encodings(encodings)
        .into();
    let array = {
        let metadata_size = encblock.split_to(4).as_ref().read_u32::<LittleEndian>()?;
        let mut dtype_bytes = encblock.split_to(metadata_size as usize);

        let mut dtype_reader = DTypeBufferReader::new();
        let mut read_buf = Bytes::new();
        while let Some(u) = dtype_reader.read(read_buf)? {
            read_buf = dtype_bytes.split_to(u);
        }
        let dtype = dtype_reader.into_dtype();
        let _padding =
            encblock.split_to(padding_size(metadata_size as usize + 4, VORTEX_ALIGNMENT));
        let mut array_reader = ArrayMessageReader::new();
        let mut read_buf = Bytes::new();
        while let Some(u) = array_reader.read(read_buf)? {
            read_buf = encblock.split_to(u);
        }
        array_reader.into_array(context, dtype)?
    };
    // array
    //     .with_dyn(|a| a.compare(&array, vortex::compute::Operator::Eq))
    //     .unwrap()
    //     .unwrap();
    // slice(&array, 0, 10).unwrap();
    // Const ptr should not be dropped
    // std::mem::forget(bytes);
    let output = array.into_canonical()?.into_arrow()?;
    // to_ffi(&output.to_data());
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
