use arrow_buffer::{Buffer, MutableBuffer};
use arrow_data::ArrayData;
pub use fff_core::errors::Result;
use ffi::WasmDecoder;

pub mod ffi;
pub mod kwargs;

/// Decode scalar data like int32 and float32. i.e., single type, single buffer as input/output
pub type ScalarDecode = fn(input: &[u8]) -> Result<Box<[u8]>>;

/// Decode string data, two buffers as output, one for length and one for data
pub type StringDecode = fn(input: &[u8]) -> Result<Vec<Box<[u8]>>>;

/// A general decode function that returns an iterator of the decoded buffers.
pub type GeneralDecode = fn(input: &[u8]) -> Result<Box<dyn Iterator<Item = Buffer>>>;
/// An experimental API for more than one byte sequence of input
pub type GeneralDecodeV2 = fn(inputs: &[&[u8]]) -> Result<Box<dyn Iterator<Item = Buffer>>>;
/// An experiemntal API using Arrow FFI
pub type GeneralDecodeV3 = fn(inputs: &[u8]) -> Result<arrow_array::ffi::FFI_ArrowArray>;

pub fn arraydata_to_buffers(res: &mut Vec<Buffer>, array_data: &ArrayData) {
    res.push(match array_data.nulls() {
        Some(nulls) => nulls.buffer().clone(),
        None => MutableBuffer::new(0).into(),
    });
    // let ptr = res[0].as_ptr();
    // println!("{ptr:?}");
    for buffer in array_data.buffers() {
        // let ptr = buffer.as_ptr();
        // println!("{ptr:?}");
        res.push(buffer.clone());
    }
    for child in array_data.child_data() {
        arraydata_to_buffers(res, child);
    }
}

/// Stateful WasmDecoder for the Prepare-Init-Decode APIs
pub trait StatefulWasmDecoder {
    fn decode(&mut self) -> Result<Option<Box<dyn Iterator<Item = Buffer>>>>;
}

/// Init API
pub type Init = fn(input: &[u8], kwargs: &[u8]) -> Result<Box<dyn StatefulWasmDecoder>>;
/// Decode API
pub type Decode = fn(input: *mut WasmDecoder) -> Result<Option<Box<dyn Iterator<Item = Buffer>>>>;
