#![feature(stmt_expr_attributes)]
use std::sync::Arc;
use std::vec;

use arrow_buffer::Buffer;
use byteorder::{LittleEndian, ReadBytesExt};
use bytes::Bytes;
use fff_core::util::bit_util::padding_size;
use fff_ude::ffi::general_wrapper;
use fff_ude::{arraydata_to_buffers, Result};
#[cfg(feature = "alp")]
use vortex_alp::ALPEncoding;
use vortex_array::array::{PrimitiveEncoding, VarBinEncoding};
use vortex_array::canonical::primitive_to_arrow;
use vortex_array::encoding::EncodingRef;
use vortex_array::{Context, IntoCanonical};
#[cfg(feature = "fsst")]
use vortex_fsst::FSSTEncoding;
#[cfg(feature = "runend")]
use vortex_runend::RunEndEncoding;
use vortex_ipc::{ArrayMessageReader, DTypeBufferReader};

const VORTEX_ALIGNMENT: usize = 64;

// fn fls_bp_decompress(input: BitPackedArray) -> Result<ArrayRef> {
//     Ok(input.into_canonical()?.into_arrow()?)
// }

fn decode_vortex_general(input: &[u8]) -> Result<Box<dyn Iterator<Item = Buffer>>> {
    // FIXME: more elegant way to avoid copy?
    let bytes = unsafe {
        Bytes::from(Vec::from_raw_parts(
            input.as_ptr() as *mut u8,
            input.len(),
            input.len(),
        ))
    };
    let encblock = &mut bytes.clone();
    let mut encodings: Vec<EncodingRef> = vec![&PrimitiveEncoding as EncodingRef, &VarBinEncoding];
    #[cfg(feature = "runend")]
    encodings.push(&RunEndEncoding as EncodingRef);
    #[cfg(feature = "fsst")]
    encodings.push(&FSSTEncoding);
    #[cfg(feature = "alp")]
    encodings.push(&ALPEncoding as EncodingRef);

    let context: Arc<Context> = vortex::Context::empty().with_encodings(encodings).into();
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
    // Const ptr should not be dropped
    std::mem::forget(bytes);
    let output = primitive_to_arrow(array.into_canonical()?.into_primitive()?)?;
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
