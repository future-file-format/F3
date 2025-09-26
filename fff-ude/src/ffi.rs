// Initial version vendored but significantly modified from https://github.com/risingwavelabs/arrow-udf

//! FFI interfaces.

use arrow_buffer::Buffer;
use fff_core::errors::Error;

use crate::{
    Decode, GeneralDecode, GeneralDecodeV2, Init, ScalarDecode, StatefulWasmDecoder, StringDecode,
};

/// A symbol indicating the ABI version.
///
/// The version follows semantic versioning `MAJOR.MINOR`.
/// - The major version is incremented when incompatible API changes are made.
/// - The minor version is incremented when new functionality are added in a backward compatible manner.
///
/// # Changelog
///
/// - 1.0: Initial version.
#[no_mangle]
#[used]
pub static FFFUDE_VERSION_1_0: () = ();

/// Allocate memory.
///
/// # Safety
///
/// See [`std::alloc::GlobalAlloc::alloc`].
#[no_mangle]
pub unsafe extern "C" fn alloc(len: usize, align: usize) -> *mut u8 {
    std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(len, align))
}

/// Deallocate memory.
///
/// # Safety
///
/// See [`std::alloc::GlobalAlloc::dealloc`].
#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize, align: usize) {
    std::alloc::dealloc(
        ptr,
        std::alloc::Layout::from_size_align_unchecked(len, align),
    );
}

/// A FFI-safe slice.
#[repr(C)]
#[derive(Debug)]
pub struct CSlice {
    pub ptr: *const u8,
    pub len: usize,
}

/// A wrapper for calling scalar functions from C.
///
/// The input encoded data is read from the buffer pointed to by `ptr` and `len`.
///
/// The output data is written to the buffer pointed to by `out_slice`.
/// The caller is responsible for deallocating the output buffer.
///
/// The return value is 0 on success, -1 on error.
/// If successful, the decoded data is written to the output buffer.
/// If failed, the error message is written to the buffer.
///
/// # Safety
///
/// `ptr`, `len`, `out_slice` must point to a valid buffer.
pub unsafe fn scalar_wrapper(
    function: ScalarDecode,
    ptr: *const u8,
    len: usize,
    out_slice: *mut CSlice,
) -> i32 {
    let input = std::slice::from_raw_parts(ptr, len);
    match call_primitive(function, input) {
        Ok(data) => {
            out_slice.write(CSlice {
                ptr: data.as_ptr(),
                len: data.len(),
            });
            std::mem::forget(data);
            0
        }
        Err(err) => {
            let msg = err.to_string().into_boxed_str();
            out_slice.write(CSlice {
                ptr: msg.as_ptr(),
                len: msg.len(),
            });
            std::mem::forget(msg);
            -1
        }
    }
}

// /// A FFI-safe Result<slice>.
// #[repr(C)]
// pub struct ResSlice {
//     pub status: i32,
//     pub ptr: *const u8,
//     pub len: usize,
// }

/// This does not work because if returning a struct, wasm FFI simply requires one extra input param of the address of the struct.
// pub unsafe fn scalar_wrapper_v2(function: ScalarDecode, ptr: *const u8, len: usize) -> ResSlice {
//     let input = std::slice::from_raw_parts(ptr, len);
//     match call_primitive(function, input) {
//         Ok(data) => {
//             let res = ResSlice {
//                 status: 0,
//                 ptr: data.as_ptr(),
//                 len: data.len(),
//             };
//             std::mem::forget(data);
//             res
//         }
//         Err(err) => {
//             let msg = err.to_string().into_boxed_str();
//             let res = ResSlice {
//                 status: -1,
//                 ptr: msg.as_ptr(),
//                 len: msg.len(),
//             };
//             std::mem::forget(msg);
//             res
//         }
//     }
// }

/// The internal wrapper that returns a Result.
fn call_primitive(function: ScalarDecode, input_bytes: &[u8]) -> Result<Box<[u8]>, Error> {
    let output_batch = function(input_bytes)?;
    Ok(output_batch)
}

pub unsafe fn string_wrapper(
    function: StringDecode,
    ptr: *const u8,
    len: usize,
    out_slice0: *mut CSlice,
    out_slice1: *mut CSlice,
) -> i32 {
    let input = std::slice::from_raw_parts(ptr, len);
    match call_string(function, input) {
        Ok(data) => {
            out_slice0.write(CSlice {
                ptr: data[0].as_ptr(),
                len: data[0].len(),
            });
            out_slice1.write(CSlice {
                ptr: data[1].as_ptr(),
                len: data[1].len(),
            });
            std::mem::forget(data);
            0
        }
        Err(err) => {
            let msg = err.to_string().into_boxed_str();
            out_slice0.write(CSlice {
                ptr: msg.as_ptr(),
                len: msg.len(),
            });
            std::mem::forget(msg);
            -1
        }
    }
}

/// The internal wrapper that returns a Result.
fn call_string(function: StringDecode, input_bytes: &[u8]) -> Result<Vec<Box<[u8]>>, Error> {
    let output_batch = function(input_bytes)?;
    Ok(output_batch)
}

/// Drop the array. Currently no use because we use ArrayData to transfer data
///
/// # Safety
///
/// `array` must be valid pointers.
// #[no_mangle]
// pub unsafe extern "C" fn array_drop(array: *mut ArrayRef) {
//     drop(Box::from_raw(array));
// }

/// An opaque type for iterating over Buffers.
pub struct BufferIter {
    iter: Box<dyn Iterator<Item = Buffer>>,
}

/// A wrapper for calling general decoding functions from C.
///
/// The input encoded data is read from the buffer pointed to by `ptr` and `len`.
///
/// The output iterator is written to `out_slice`.
///
/// The return value is 0 on success, -1 on error.
/// If successful, the Buffer iterator is written to the buffer.
/// If failed, the error message is written to the buffer.
///
/// # Safety
///
/// `ptr`, `len`, `out_slice` must point to a valid buffer.
pub unsafe fn general_wrapper(
    function: GeneralDecode,
    ptr: *const u8,
    len: usize,
    out_slice: *mut CSlice,
) -> i32 {
    let input = std::slice::from_raw_parts(ptr, len);
    match call_general(function, input) {
        Ok(iter) => {
            out_slice.write(CSlice {
                ptr: Box::into_raw(iter) as *const u8,
                len: std::mem::size_of::<BufferIter>(),
            });
            0
        }
        Err(err) => {
            let msg = err.to_string().into_boxed_str();
            out_slice.write(CSlice {
                ptr: msg.as_ptr(),
                len: msg.len(),
            });
            std::mem::forget(msg);
            -1
        }
    }
}

fn call_general(function: GeneralDecode, input_bytes: &[u8]) -> Result<Box<BufferIter>, Error> {
    let iter = function(input_bytes)?;
    Ok(Box::new(BufferIter { iter }))
}

pub unsafe fn general_wrapperv2(
    function: GeneralDecodeV2,
    ptr: *const u8,
    lengths: *const u32,
    num_slices: u32,
    out_slice: *mut CSlice,
) -> i32 {
    let slice_lengths = unsafe { std::slice::from_raw_parts(lengths, num_slices as usize) };
    let mut inputs = vec![];
    let mut prefix_sum = 0;
    for &len in slice_lengths {
        inputs.push(unsafe { std::slice::from_raw_parts(ptr.add(prefix_sum), len as usize) });
        prefix_sum += len as usize;
    }
    match call_generalv2(function, &inputs) {
        Ok(iter) => {
            out_slice.write(CSlice {
                ptr: Box::into_raw(iter) as *const u8,
                len: std::mem::size_of::<BufferIter>(),
            });
            0
        }
        Err(err) => {
            let msg = err.to_string().into_boxed_str();
            out_slice.write(CSlice {
                ptr: msg.as_ptr(),
                len: msg.len(),
            });
            std::mem::forget(msg);
            -1
        }
    }
}

fn call_generalv2(
    function: GeneralDecodeV2,
    input_bytes: &[&[u8]],
) -> Result<Box<BufferIter>, Error> {
    let iter = function(input_bytes)?;
    Ok(Box::new(BufferIter { iter }))
}
/// Get the next Buffer from the iterator.
///
/// The output Buffer is written to the buffer pointed to by `out`.
/// The caller is responsible for deallocating the output buffer.
///
/// # Safety
///
/// `iter` and `out` must be valid pointers.
#[no_mangle]

pub unsafe extern "C" fn buffer_iterator_next(
    iter: *mut BufferIter,
    out: *mut CSlice,
    out_buffer_box_ptr: *mut u32,
) {
    let iter = iter.as_mut().expect("null pointer");
    if let Some(buffer) = iter.iter.next() {
        out.write(CSlice {
            ptr: buffer.as_ptr(),
            len: buffer.len(),
        });
        // BUGFIX(1223): we have to put the buffer on heap, not stack
        let batch = Box::new(buffer);
        out_buffer_box_ptr.write(Box::into_raw(batch) as *const u8 as u32);
    } else {
        out.write(CSlice {
            ptr: std::ptr::null(),
            len: 0,
        });
    }
}

/// Drop the iterator.
///
/// # Safety
///
/// `iter` must be valid pointers.
#[no_mangle]
pub unsafe extern "C" fn buffer_iterator_drop(iter: *mut BufferIter) {
    drop(Box::from_raw(iter));
}

/// Drop the Arrow Buffer which should be on the Wasm heap.
///
/// # Safety
///
/// ` buffer` must be valid pointer.
#[no_mangle]
pub unsafe extern "C" fn buffer_drop(buffer: *mut Buffer) {
    // println!("drop arrow buffer at: {:?}", buffer);
    drop(Box::from_raw(buffer));
}

//----------Begin APIs with advanced features support (kwargs) ----------//

/// An opaque type for stateful Wasm Decoder API.
pub struct WasmDecoder {
    inner: Box<dyn StatefulWasmDecoder>,
}

impl WasmDecoder {
    pub fn decode(&mut self) -> crate::Result<Option<Box<dyn Iterator<Item = Buffer>>>> {
        self.inner.decode()
    }
}

/// A wrapper for calling `Init` from C.
///
/// The input encoded data is read from the buffer pointed to by `input_ptr` and `input_len`.
///
/// The kwargs is serialized into the buffer pointed to by `kwargs_ptr` and `kwargs_len`.
///
/// The output WasmDecoder is written to `out_slice`.
///
/// The return value is 0 on success, -1 on error.
/// If successful, the WasmDecoder is written to the buffer.
/// If failed, the error message is written to the buffer.
///
/// # Safety
///
/// `input_ptr`, `kwargs_ptr`, `out_slice` must point to a valid buffer.
pub unsafe fn init_wrapper(
    function: Init,
    input_ptr: *const u8,
    input_len: usize,
    kwargs_ptr: *const u8,
    kwargs_len: usize,
    out_slice: *mut CSlice,
) -> i32 {
    let input = std::slice::from_raw_parts(input_ptr, input_len);
    let kwargs = std::slice::from_raw_parts(kwargs_ptr, kwargs_len);
    match call_init(function, input, kwargs) {
        Ok(decoder) => {
            out_slice.write(CSlice {
                ptr: Box::into_raw(decoder) as *const u8,
                len: std::mem::size_of::<WasmDecoder>(),
            });
            0
        }
        Err(err) => {
            let msg = err.to_string().into_boxed_str();
            out_slice.write(CSlice {
                ptr: msg.as_ptr(),
                len: msg.len(),
            });
            std::mem::forget(msg);
            -1
        }
    }
}

fn call_init(function: Init, input: &[u8], kwargs: &[u8]) -> Result<Box<WasmDecoder>, Error> {
    let inner = function(input, kwargs)?;
    Ok(Box::new(WasmDecoder { inner }))
}

/// A wrapper for calling the `Decode` API from C.
///
/// The input encoded data is read from the buffer pointed to by `ptr` and `len`.
///
/// The output iterator is written to `out_slice`.
///
/// The return value is 0 on success with Some(val), 1 on success with None, -1 on error.
/// If successful, the WasmDecoder is written to the buffer.
/// If failed, the error message is written to the buffer.
///
/// # Safety
///
/// `wasm_decoder`, `out_slice` must point to a valid buffer.
pub unsafe fn decode_wrapper(
    function: Decode,
    wasm_decoder: *mut WasmDecoder,
    out_slice: *mut CSlice,
) -> i32 {
    match function(wasm_decoder) {
        Ok(iter) => {
            if let Some(iter) = iter {
                let iter = Box::new(BufferIter { iter });
                out_slice.write(CSlice {
                    ptr: Box::into_raw(iter) as *const u8,
                    len: std::mem::size_of::<BufferIter>(),
                });
                0
            } else {
                1
            }
        }
        Err(err) => {
            let msg = err.to_string().into_boxed_str();
            out_slice.write(CSlice {
                ptr: msg.as_ptr(),
                len: msg.len(),
            });
            std::mem::forget(msg);
            -1
        }
    }
}
//----------END APIs with advanced features support (kwargs) ----------//
