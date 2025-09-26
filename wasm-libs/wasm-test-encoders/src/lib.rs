#![feature(vec_into_raw_parts)]
use core::panic;
use std::{
    collections::HashMap,
    io::{Cursor, Read, Write},
    sync::Arc,
    vec,
};

use arrow_array::{
    cast::AsArray,
    ffi::{FFI_ArrowArray, FFI_ArrowSchema},
    make_array, Array, ArrayRef, ArrowPrimitiveType, PrimitiveArray,
};
use arrow_buffer::{Buffer, MutableBuffer};
use arrow_schema::DataType;
use byteorder::{LittleEndian, ReadBytesExt};
use bytes::Bytes;
use fastlanes::BitPacking;
use fff_encoding::schemes::{
    vortex::{VortexDecoder, VortexEncoder},
    Decoder, Encoder,
};
use fff_ude::{arraydata_to_buffers, Result};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use lz4_flex::{compress_prepend_size, decompress_size_prepended};
use pco::{
    data_types::Number,
    standalone::{simple_compress, simple_decompress},
    ChunkConfig,
};
use uniffi_core::RustBuffer;
use vortex_sampling_compressor::ALL_ENCODINGS_CONTEXT;

pub fn encode_fff_general(input: ArrayRef) -> Vec<u8> {
    let enc = VortexEncoder::default();
    let res = Cursor::new(vec![]);
    enc.encode(input)
        .unwrap()
        .try_serialize(res)
        .unwrap()
        .into_inner()
}

pub fn decode_fff_general_normal_ver(input: &[u8]) -> Result<Box<dyn Iterator<Item = Buffer>>> {
    // let bytes = unsafe {
    //     Bytes::from(Vec::from_raw_parts(
    //         input.as_ptr() as *mut u8,
    //         input.len(),
    //         input.len(),
    //     ))
    // };
    // FIXME: there is a copy here. Just want to be on par with the wasm version
    let bytes = Bytes::copy_from_slice(input);
    let mut vortex_decoder = VortexDecoder::try_new(bytes.clone(), ALL_ENCODINGS_CONTEXT.clone())?;
    let data = vortex_decoder.decode_all_as_array().unwrap().to_data();
    // println!("{:?}", data.data_type());
    // Const ptr should not be dropped
    // std::mem::forget(bytes);
    let mut res: Vec<Buffer> = vec![];
    arraydata_to_buffers(&mut res, &data);
    Ok(Box::new(res.into_iter()))
}

pub fn decode_fff_general(input: &[u8]) -> Result<Box<dyn Iterator<Item = Buffer>>> {
    // We have to always copy here, since the vortx decoder may zero-copy from the input to output
    let bytes = Bytes::copy_from_slice(input);
    // let bytes = unsafe {
    //     Bytes::from(Vec::from_raw_parts(
    //         input.as_ptr() as *mut u8,
    //         input.len(),
    //         input.len(),
    //     ))
    // };
    let mut vortex_decoder = VortexDecoder::try_new(bytes.clone(), ALL_ENCODINGS_CONTEXT.clone())?;
    let data = vortex_decoder.decode_all_as_array().unwrap().to_data();

    // Const ptr should not be dropped
    // std::mem::forget(bytes);
    let mut res: Vec<Buffer> = vec![];
    arraydata_to_buffers(&mut res, &data);
    Ok(Box::new(res.into_iter()))
}

// pub fn decode_fff_general_ffi(input: &[u8]) -> Result<ffi::FFI_ArrowArray> {
//     // We have to always copy here, since the vortx decoder may zero-copy from the input to output
//     let bytes = Bytes::copy_from_slice(input);

//     let mut vortex_decoder = VortexDecoder::try_new(bytes.clone(), ALL_ENCODINGS_CONTEXT.clone())?;
//     let data = vortex_decoder.decode_all_as_array().unwrap().to_data();
//     Ok(ffi::FFI_ArrowArray::new(&data))
// }

pub fn encode_flsbp_general<T: fastlanes::FastLanes + BitPacking + 'static>(
    input: &[T],
) -> Vec<u8> {
    assert!(input.len() % 1024 == 0);
    let bit_width: usize = input.iter().fold(0, |acc, &x| {
        std::cmp::max(acc, size_of::<T>() * 8 - x.leading_zeros() as usize)
    });
    let type_id: u32 = match std::any::TypeId::of::<T>() {
        t if t == std::any::TypeId::of::<u8>() => 0,
        t if t == std::any::TypeId::of::<u16>() => 1,
        t if t == std::any::TypeId::of::<u32>() => 2,
        t if t == std::any::TypeId::of::<u64>() => 3,
        _ => panic!("Unsupported type"),
    };

    let packed_len = 128 * bit_width / size_of::<T>();

    let mut encoded_data: Vec<T> = Vec::new();
    for (start, end) in (0..input.len()).step_by(1024).map(|i| (i, i + 1024)) {
        let mini_block = &input[start..end];
        encoded_data.reserve(packed_len);
        let output_len = encoded_data.len();
        unsafe {
            encoded_data.set_len(output_len + packed_len);
            BitPacking::unchecked_pack(
                bit_width,
                mini_block,
                &mut encoded_data[output_len..][..packed_len],
            );
        }
    }
    let (p, l, c) = encoded_data.into_raw_parts();
    let mut encoded_data =
        unsafe { Vec::<u8>::from_raw_parts(p as *mut u8, l * size_of::<T>(), c * size_of::<T>()) };
    encoded_data.extend_from_slice(&(type_id as u32).to_le_bytes());
    encoded_data.extend_from_slice(&(bit_width as u32).to_le_bytes());
    encoded_data.extend_from_slice(&(input.len() as u32).to_le_bytes());
    encoded_data
}

fn decode_fls_bp(input: &[u8], _wasm: bool) -> Result<Box<dyn Iterator<Item = Buffer>>> {
    let len: u32 = (&input[input.len() - 4..input.len()]).read_u32::<LittleEndian>()?;
    let bitwidth: u32 = (&input[input.len() - 8..input.len() - 4]).read_u32::<LittleEndian>()?;
    let typeid: u32 = (&input[input.len() - 12..input.len() - 8]).read_u32::<LittleEndian>()?;
    let input = &input[0..input.len() - 12];
    let output_buffer = match typeid {
        0 => {
            type T = u8;
            let input = bytemuck::cast_slice(input);
            let mut output = Vec::<T>::new();
            let packed_len = 128 * bitwidth as usize / size_of::<T>();
            output.reserve(len as usize);
            unsafe {
                output.set_len(len as usize);
            }
            for (start, end) in (0..len as usize).step_by(1024).map(|i| (i, i + 1024)) {
                let i = start / 1024;
                unsafe {
                    BitPacking::unchecked_unpack(
                        bitwidth as usize,
                        &input[i * packed_len..(i + 1) * packed_len],
                        &mut output[start..end],
                    );
                }
            }
            Buffer::from(output)
        }
        1 => {
            type T = u16;
            let input = bytemuck::cast_slice(input);
            let mut output = Vec::<T>::new();
            let packed_len = 128 * bitwidth as usize / size_of::<T>();
            output.reserve(len as usize);
            unsafe {
                output.set_len(len as usize);
            }
            for (start, end) in (0..len as usize).step_by(1024).map(|i| (i, i + 1024)) {
                let i = start / 1024;
                unsafe {
                    BitPacking::unchecked_unpack(
                        bitwidth as usize,
                        &input[i * packed_len..(i + 1) * packed_len],
                        &mut output[start..end],
                    );
                }
            }
            Buffer::from(output)
        }
        2 => {
            type T = u32;
            let input = bytemuck::cast_slice(input);
            let mut output = Vec::<T>::new();
            let packed_len = 128 * bitwidth as usize / size_of::<T>();
            output.reserve(len as usize);
            unsafe {
                output.set_len(len as usize);
            }
            for (start, end) in (0..len as usize).step_by(1024).map(|i| (i, i + 1024)) {
                let i = start / 1024;
                unsafe {
                    BitPacking::unchecked_unpack(
                        bitwidth as usize,
                        &input[i * packed_len..(i + 1) * packed_len],
                        &mut output[start..end],
                    );
                }
            }
            Buffer::from(output)
        }
        3 => {
            type T = u64;
            let input = bytemuck::cast_slice(input);
            let mut output = Vec::<T>::new();
            let packed_len = 128 * bitwidth as usize / size_of::<T>();
            output.reserve(len as usize);
            unsafe {
                output.set_len(len as usize);
            }
            for (start, end) in (0..len as usize).step_by(1024).map(|i| (i, i + 1024)) {
                let i = start / 1024;
                unsafe {
                    BitPacking::unchecked_unpack(
                        bitwidth as usize,
                        &input[i * packed_len..(i + 1) * packed_len],
                        &mut output[start..end],
                    );
                }
            }
            Buffer::from(output)
        }
        _ => panic!(),
    };

    let mut res: Vec<Buffer> = vec![];
    // assume no nulls. We omit Nulls decode for purely testing the codec.
    res.push(MutableBuffer::from_len_zeroed(0).into());
    res.push(output_buffer);
    // if wasm {
    //     unsafe {
    //         dealloc(input.as_ptr() as *mut u8, input.len(), 1);
    //     }
    // }
    Ok(Box::new(res.into_iter()))
}

pub fn decode_flsbp_general(input: &[u8]) -> Result<Box<dyn Iterator<Item = Buffer>>> {
    decode_fls_bp(input, true)
}

pub fn decode_flsbp_native(input: &[u8]) -> Result<Box<dyn Iterator<Item = Buffer>>> {
    decode_fls_bp(input, false)
}

pub fn encode_pco_general<T: Number>(input: &[T]) -> Vec<u8> {
    let mut encoded = vec![];
    let type_id: u32 = match std::any::TypeId::of::<T>() {
        t if t == std::any::TypeId::of::<i16>() => 0,
        t if t == std::any::TypeId::of::<i32>() => 1,
        t if t == std::any::TypeId::of::<i64>() => 2,
        t if t == std::any::TypeId::of::<u16>() => 3,
        t if t == std::any::TypeId::of::<u32>() => 4,
        t if t == std::any::TypeId::of::<u64>() => 5,
        t if t == std::any::TypeId::of::<f32>() => 6,
        t if t == std::any::TypeId::of::<f64>() => 7,
        _ => panic!("Unsupported type"),
    };
    encoded.extend_from_slice(&type_id.to_le_bytes());
    encoded.extend_from_slice(&simple_compress(input, &ChunkConfig::default()).unwrap());
    encoded
}

pub fn decode_pco_general(input: &[u8]) -> Result<Box<dyn Iterator<Item = Buffer>>> {
    let data_type: u32 = (&input[0..4]).read_u32::<LittleEndian>()?;
    let input = &input[4..];
    // just some random magic to ensure it covers all the code
    let recovered = match data_type {
        0 => Buffer::from(simple_decompress::<i16>(input).unwrap()),
        1 => Buffer::from(simple_decompress::<i32>(input).unwrap()),
        2 => Buffer::from(simple_decompress::<i64>(input).unwrap()),
        3 => Buffer::from(simple_decompress::<u16>(input).unwrap()),
        4 => Buffer::from(simple_decompress::<u32>(input).unwrap()),
        5 => Buffer::from(simple_decompress::<u64>(input).unwrap()),
        6 => Buffer::from(simple_decompress::<f32>(input).unwrap()),
        7 => Buffer::from(simple_decompress::<f64>(input).unwrap()),
        _ => panic!(),
    };
    let mut res: Vec<Buffer> = vec![];
    // assume no nulls. We omit Nulls decode for purely testing the codec.
    res.push(MutableBuffer::from_len_zeroed(0).into());
    res.push(recovered);
    Ok(Box::new(res.into_iter()))
}

/// FIXME: We cannot return RustBuffer in the desired dylib because the underlying lib may be conpiled from C or other languages.
/// In that sense, like the Arrow's FFI, it should also return a function pointer on how to drop this returned buffer.
/// We will fix this. Or find others who did this before.
pub unsafe extern "C" fn encode_pco_real_general_c(
    input: FFI_ArrowArray,
    schema: FFI_ArrowSchema,
) -> RustBuffer {
    let array_data = unsafe { arrow_array::ffi::from_ffi(input, &schema).unwrap() };
    let array = make_array(array_data);
    RustBuffer::from_vec(encode_pco_real_general(array))
}

fn encode_pco_real_general(input: ArrayRef) -> Vec<u8> {
    use arrow_array::types::*;
    match *input.data_type() {
        DataType::Int8 | DataType::UInt8 => unimplemented!(),
        DataType::UInt16 => {
            let input = input.as_primitive::<UInt16Type>();
            encode_pco_real_general_helper(input)
        }
        DataType::Int16 => {
            let input = input.as_primitive::<Int16Type>();
            encode_pco_real_general_helper(input)
        }
        DataType::UInt32 => {
            let input = input.as_primitive::<UInt32Type>();
            encode_pco_real_general_helper(input)
        }
        DataType::Int32 => {
            let input = input.as_primitive::<Int32Type>();
            encode_pco_real_general_helper(input)
        }
        DataType::UInt64 => {
            let input = input.as_primitive::<UInt64Type>();
            encode_pco_real_general_helper(input)
        }
        DataType::Int64 => {
            let input = input.as_primitive::<Int64Type>();
            encode_pco_real_general_helper(input)
        }
        DataType::Float16 => {
            let input = input.as_primitive::<Float16Type>();
            encode_pco_real_general_helper(input)
        }
        DataType::Float32 => {
            let input = input.as_primitive::<Float32Type>();
            encode_pco_real_general_helper(input)
        }
        DataType::Float64 => {
            let input = input.as_primitive::<Float64Type>();
            encode_pco_real_general_helper(input)
        }
        DataType::Timestamp(arrow_schema::TimeUnit::Nanosecond, _) => {
            let input = input.as_primitive::<TimestampNanosecondType>();
            encode_pco_real_general_helper(input)
        }
        DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, _) => {
            let input = input.as_primitive::<TimestampMicrosecondType>();
            encode_pco_real_general_helper(input)
        }
        _ => unimplemented!(),
    }
}

fn encode_pco_real_general_helper<T>(input: &PrimitiveArray<T>) -> Vec<u8>
where
    T: ArrowPrimitiveType,
    <T as ArrowPrimitiveType>::Native: pco::data_types::Number,
{
    let type_id: u32 = match std::any::TypeId::of::<T::Native>() {
        t if t == std::any::TypeId::of::<i16>() => 0,
        t if t == std::any::TypeId::of::<i32>() => 1,
        t if t == std::any::TypeId::of::<i64>() => 2,
        t if t == std::any::TypeId::of::<u16>() => 3,
        t if t == std::any::TypeId::of::<u32>() => 4,
        t if t == std::any::TypeId::of::<u64>() => 5,
        t if t == std::any::TypeId::of::<f32>() => 6,
        t if t == std::any::TypeId::of::<f64>() => 7,
        _ => panic!("Unsupported type"),
    };
    let mut encoded = vec![];
    let null_flag: u32 = input.nulls().is_some().then(|| 1).unwrap_or(0);
    encoded.extend_from_slice(&null_flag.to_le_bytes());
    if let Some(buf) = input.nulls() {
        encoded.extend_from_slice(&((buf.buffer().len() as u32).to_le_bytes()));
        encoded.extend_from_slice(buf.buffer());
    }
    encoded.extend_from_slice(&type_id.to_le_bytes());
    encoded.extend_from_slice(
        &simple_compress::<T::Native>(input.values(), &ChunkConfig::default()).unwrap(),
    );
    encoded
}

pub fn decode_pco_real_general(input: &[u8]) -> Result<Box<dyn Iterator<Item = Buffer>>> {
    let null_flag: u32 = (&input[0..4]).read_u32::<LittleEndian>()?;
    let mut ptr = 4;
    let null_buffer = if null_flag == 1 {
        let null_len: u32 = (&input[4..8]).read_u32::<LittleEndian>()?;
        let null_buf = &input[8..8 + null_len as usize];
        ptr += 4 + null_len as usize;
        Buffer::from(null_buf)
    } else {
        MutableBuffer::from_len_zeroed(0).into()
    };
    let data_type = (&input[ptr..ptr + 4]).read_u32::<LittleEndian>()?;
    let input = &input[ptr + 4..];
    // just some random magic to ensure it covers all the code
    let recovered = match data_type {
        0 => Buffer::from(simple_decompress::<i16>(input).unwrap()),
        1 => Buffer::from(simple_decompress::<i32>(input).unwrap()),
        2 => Buffer::from(simple_decompress::<i64>(input).unwrap()),
        3 => Buffer::from(simple_decompress::<u16>(input).unwrap()),
        4 => Buffer::from(simple_decompress::<u32>(input).unwrap()),
        5 => Buffer::from(simple_decompress::<u64>(input).unwrap()),
        6 => Buffer::from(simple_decompress::<f32>(input).unwrap()),
        7 => Buffer::from(simple_decompress::<f64>(input).unwrap()),
        _ => panic!(),
    };
    Ok(Box::new(vec![null_buffer, recovered].into_iter()))
}

/// FIXME: We cannot return RustBuffer in the desired dylib because the underlying lib may be conpiled from C or other languages.
/// In that sense, like the Arrow's FFI, it should also return a function pointer on how to drop this returned buffer.
/// We will fix this. Or find others who did this before.
pub unsafe extern "C" fn encode_custom_c(
    input: FFI_ArrowArray,
    schema: FFI_ArrowSchema,
) -> RustBuffer {
    let array_data = unsafe { arrow_array::ffi::from_ffi(input, &schema).unwrap() };
    let array = make_array(array_data);
    RustBuffer::from_vec(encode_custom(array))
}

fn encode_custom(input: ArrayRef) -> Vec<u8> {
    use arrow_array::types::*;
    match *input.data_type() {
        DataType::Int8 | DataType::UInt8 => unimplemented!(),
        DataType::UInt16 => {
            let input = input.as_primitive::<UInt16Type>();
            encode_custom_helper(input)
        }
        DataType::Int16 => {
            let input = input.as_primitive::<Int16Type>();
            encode_custom_helper(input)
        }
        DataType::UInt32 => {
            let input = input.as_primitive::<UInt32Type>();
            encode_custom_helper(input)
        }
        DataType::Int32 => {
            let input = input.as_primitive::<Int32Type>();
            encode_custom_helper(input)
        }
        DataType::UInt64 => {
            let input = input.as_primitive::<UInt64Type>();
            encode_custom_helper(input)
        }
        DataType::Int64 => {
            let input = input.as_primitive::<Int64Type>();
            encode_custom_helper(input)
        }
        DataType::Timestamp(arrow_schema::TimeUnit::Nanosecond, _) => {
            let input = input.as_primitive::<TimestampNanosecondType>();
            encode_custom_helper(input)
        }
        DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, _) => {
            let input = input.as_primitive::<TimestampMicrosecondType>();
            encode_custom_helper(input)
        }
        _ => unimplemented!(),
    }
}

fn encode_custom_helper<T>(input: &PrimitiveArray<T>) -> Vec<u8>
where
    T: ArrowPrimitiveType,
{
    let type_id: u32 = match std::any::TypeId::of::<T::Native>() {
        t if t == std::any::TypeId::of::<i16>() => 0,
        t if t == std::any::TypeId::of::<i32>() => 1,
        t if t == std::any::TypeId::of::<i64>() => 2,
        t if t == std::any::TypeId::of::<u16>() => 3,
        t if t == std::any::TypeId::of::<u32>() => 4,
        t if t == std::any::TypeId::of::<u64>() => 5,
        t if t == std::any::TypeId::of::<f32>() => 6,
        t if t == std::any::TypeId::of::<f64>() => 7,
        _ => panic!("Unsupported type"),
    };
    let mut encoded = vec![];
    let null_flag: u32 = input.nulls().is_some().then(|| 1).unwrap_or(0);
    encoded.extend_from_slice(&null_flag.to_le_bytes());
    if let Some(buf) = input.nulls() {
        encoded.extend_from_slice(&((buf.buffer().len() as u32).to_le_bytes()));
        encoded.extend_from_slice(buf.buffer());
    }
    encoded.extend_from_slice(&type_id.to_le_bytes());
    encoded.extend_from_slice(&(input.len() as u32).to_le_bytes());
    let values = input.values();
    // Use byte code to uniquely identify the 128 values
    let mut dict_idx = 0_u8;
    let mut dict = HashMap::<Vec<u8>, u8>::new();
    assert!(values.len() % 128 == 0);
    // iterate values 128 at a time
    for (start, _) in (0..values.len()).step_by(128).map(|i| (i, i + 128)) {
        let mini_block = values.slice(start, 128).into_inner();
        match dict.get(mini_block.as_ref()) {
            Some(&idx) => encoded.extend_from_slice(&idx.to_le_bytes()),
            None => {
                dict.insert(mini_block.to_vec(), dict_idx);
                encoded.extend_from_slice(&dict_idx.to_le_bytes());
                dict_idx = dict_idx
                    .checked_add(1)
                    .expect("Too many repeated patterns, cannot use this encoding");
            }
        }
    }
    // Write down dict
    // Iterate dict in the order of idx
    // Step 1: Collect the keys and values into a vector of tuples
    let mut pairs: Vec<_> = dict.iter().map(|(k, v)| (k, v)).collect();
    // Step 2: Sort the vector based on the values
    pairs.sort_by(|a, b| a.1.cmp(b.1));
    for (key, _) in pairs.into_iter() {
        encoded.extend_from_slice(key);
    }
    encoded
}

pub fn decode_custom(input: &[u8]) -> Result<Box<dyn Iterator<Item = Buffer>>> {
    let null_flag: u32 = (&input[0..4]).read_u32::<LittleEndian>()?;
    let mut ptr = 4;
    let null_buffer = if null_flag == 1 {
        let null_len: u32 = (&input[4..8]).read_u32::<LittleEndian>()?;
        let null_buf = &input[8..8 + null_len as usize];
        ptr += 4 + null_len as usize;
        Buffer::from(null_buf)
    } else {
        MutableBuffer::from_len_zeroed(0).into()
    };
    let data_type = (&input[ptr..ptr + 4]).read_u32::<LittleEndian>()?;
    ptr += 4;
    let len = (&input[ptr..ptr + 4]).read_u32::<LittleEndian>()?;
    ptr += 4;
    let input = &input[ptr..];
    // just some random magic to ensure it covers all the code
    let recovered = match data_type {
        0 => Buffer::from(custom_decompress::<i16>(input, len).unwrap()),
        1 => Buffer::from(custom_decompress::<i32>(input, len).unwrap()),
        2 => Buffer::from(custom_decompress::<i64>(input, len).unwrap()),
        3 => Buffer::from(custom_decompress::<u16>(input, len).unwrap()),
        4 => Buffer::from(custom_decompress::<u32>(input, len).unwrap()),
        5 => Buffer::from(custom_decompress::<u64>(input, len).unwrap()),
        6 => Buffer::from(custom_decompress::<f32>(input, len).unwrap()),
        7 => Buffer::from(custom_decompress::<f64>(input, len).unwrap()),
        _ => panic!(),
    };
    Ok(Box::new(vec![null_buffer, recovered].into_iter()))
}

fn custom_decompress<T>(input: &[u8], len: u32) -> Result<Vec<u8>> {
    assert!(len % 128 == 0);
    let dict_start = len as usize / 128;
    let mut res = Vec::new();
    for i in 0..dict_start {
        let idx = input[i as usize];
        let start = dict_start + (idx as usize) * 128 * size_of::<T>();
        let end = start + 128 * size_of::<T>();
        res.extend_from_slice(&input[start..end]);
    }
    Ok(res)
}

pub fn encode_lz4_general(input: &[u8]) -> Vec<u8> {
    compress_prepend_size(input)
}
pub fn decode_lz4_general(input: &[u8]) -> Result<Box<dyn Iterator<Item = Buffer>>> {
    let out = decompress_size_prepended(input).unwrap();
    let mut res: Vec<Buffer> = vec![];
    // assume no nulls
    res.push(MutableBuffer::from_len_zeroed(0).into());
    // This is incorrect because the result is not Arrow Array. We omit it here simply for testing decoding performance.
    res.push(Buffer::from(out));
    Ok(Box::new(res.into_iter()))
}

pub fn encode_gzip_general(input: &[u8]) -> Vec<u8> {
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(input).unwrap();
    e.finish().unwrap()
}
pub fn decode_gzip_general(input: &[u8]) -> Result<Box<dyn Iterator<Item = Buffer>>> {
    let mut d = GzDecoder::new(input);
    let mut s = String::new();
    d.read_to_string(&mut s).unwrap();
    let mut res: Vec<Buffer> = vec![];
    // assume no nulls
    res.push(MutableBuffer::from_len_zeroed(0).into());
    // This is incorrect because the result is not Arrow Array. We omit it here simply for testing decoding performance.
    res.push(Buffer::from(s.into_bytes()));
    Ok(Box::new(res.into_iter()))
}

pub fn encode_zstd_general(input: &[u8]) -> Vec<u8> {
    let res = Vec::new();
    let mut encoder = zstd::stream::Encoder::new(res, 0).unwrap();
    let mut reader = Cursor::new(input);
    std::io::copy(&mut reader, &mut encoder).unwrap();
    encoder.finish().unwrap()
}
pub fn decode_zstd_general(input: &[u8]) -> Result<Box<dyn Iterator<Item = Buffer>>> {
    let mut out = Vec::new();
    zstd::stream::copy_decode(input, &mut out).unwrap();
    let mut res: Vec<Buffer> = vec![];
    // assume no nulls
    res.push(MutableBuffer::from_len_zeroed(0).into());
    // This is incorrect because the result is not Arrow Array. We omit it here simply for testing decoding performance.
    res.push(Buffer::from_vec(out));
    Ok(Box::new(res.into_iter()))
}

pub fn encode_lz4_general2(input: ArrayRef) -> Vec<u8> {
    // Step 1: Create a RecordBatch from the ArrayRef
    let schema = arrow_schema::Schema::new(vec![arrow_schema::Field::new(
        "column",
        input.data_type().clone(),
        false,
    )]);
    let batch = arrow_array::RecordBatch::try_new(Arc::new(schema), vec![input]).unwrap();
    let buffer: Vec<u8> = Vec::new();
    let mut writer = arrow_ipc::writer::StreamWriter::try_new(buffer, &batch.schema()).unwrap();
    writer.write(&batch).unwrap();
    let res = writer.into_inner().unwrap();
    compress_prepend_size(&res)
}

pub fn decode_lz4_general2(input: &[u8]) -> Result<Box<dyn Iterator<Item = Buffer>>> {
    let input = decompress_size_prepended(input).unwrap();
    let input = Cursor::new(input);
    let mut reader = arrow_ipc::reader::StreamReader::try_new(input, None).unwrap();
    let batch = reader.next().unwrap().unwrap();
    let mut res: Vec<Buffer> = vec![];
    arraydata_to_buffers(&mut res, &batch.column(0).to_data());
    Ok(Box::new(res.into_iter()))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_array::{
        builder::PrimitiveBuilder, ffi::to_ffi, types::Int32Type, Array, ArrayRef, PrimitiveArray,
    };
    use fff_core::util::buffer_to_array::primitive_array_from_arrow_buffers_iter;
    use rand::Rng;
    use rand_distr::{Distribution, Zipf};

    use crate::{
        decode_custom, decode_pco_real_general, encode_custom_c, encode_pco_real_general_c,
    };

    fn test(input: PrimitiveArray<Int32Type>) {
        let (ffi_arr, ffi_schema) = to_ffi(&input.to_data()).unwrap();
        let encoded = unsafe { encode_pco_real_general_c(ffi_arr, ffi_schema) };
        let encoded = encoded.destroy_into_vec();
        let out = primitive_array_from_arrow_buffers_iter(
            input.data_type(),
            decode_pco_real_general(&encoded).unwrap(),
            input.len() as u64,
        )
        .unwrap();
        assert_eq!(&(Arc::new(input) as ArrayRef), &out);
    }

    fn test2(input: PrimitiveArray<Int32Type>) {
        let (ffi_arr, ffi_schema) = to_ffi(&input.to_data()).unwrap();
        let encoded = unsafe { encode_custom_c(ffi_arr, ffi_schema) };
        let encoded = encoded.destroy_into_vec();
        println!("encoded size: {}", encoded.len());
        let out = primitive_array_from_arrow_buffers_iter(
            input.data_type(),
            decode_custom(&encoded).unwrap(),
            input.len() as u64,
        )
        .unwrap();
        assert_eq!(&(Arc::new(input) as ArrayRef), &out);
    }

    #[test]
    fn test_pco_general() {
        let mut builder = PrimitiveBuilder::<Int32Type>::new();
        builder.append_values(&vec![5, 4, 3, 2, 1], &vec![true, true, false, true, false]);
        test(builder.finish());
    }

    #[test]
    fn test_pco_general2() {
        test(PrimitiveArray::<Int32Type>::from(
            (0..65536).collect::<Vec<i32>>(),
        ));
    }

    #[test]
    fn test_custom() {
        // Define the size of the vector and the window
        let total_values = 65536; // 64k values
        let window_size = 128;

        // Create a random number generator
        let mut rng = rand::thread_rng();

        // Define the number of unique windows
        let num_unique_windows = total_values / window_size; // 65536 / 128 = 512 windows

        // Generate unique windows with uniformly distributed values
        let mut unique_windows: Vec<Vec<i32>> = Vec::with_capacity(num_unique_windows);
        for _ in 0..num_unique_windows {
            let window: Vec<i32> = (0..window_size)
                .map(|_| rng.gen_range(0..i32::MAX))
                .collect(); // Uniformly distributed values
            unique_windows.push(window);
        }

        // Define the Zipf distribution for window frequencies
        let zipf = Zipf::new(num_unique_windows as u64, 1.03)
            .expect("Invalid Zipf distribution parameters");

        // Assign frequencies to each window based on the Zipf distribution
        let mut final_windows: Vec<_> = Vec::with_capacity(num_unique_windows);
        for _ in 0..num_unique_windows {
            let freq = zipf.sample(&mut rng) as usize;
            final_windows.push(&unique_windows[freq - 1]);
        }

        // Build the final vector by repeating windows according to their scaled frequencies
        let mut vec: Vec<i32> = Vec::with_capacity(total_values);
        for window in final_windows.iter() {
            vec.extend_from_slice(window);
        }

        test2(PrimitiveArray::<Int32Type>::from(vec));
    }
}
