use std::sync::Arc;

use crate::{
    context::WASMReadingContext, dict::shared_dictionary_cache::SharedDictionaryCache,
    io::reader::Reader,
};
use arrow_array::{Array, ArrayRef, UInt16Array, UInt32Array, UInt64Array, UInt8Array};
use arrow_schema::{DataType, TimeUnit};
use bytes::BytesMut;
use fff_core::{errors::Result, general_error, non_nest_types, nyi_err};
use fff_format::File::fff::flatbuf as fb;
use flatbuffers::{ForwardsUOffset, VectorIter};

use super::encunit::create_encunit_decoder;

/// Stateful Chunk Decoder that will decode a EncUnit at a time.
pub trait ChunkDecoder {
    /// Decode out a EncUnit of data at a time.
    /// Return None if no more data to decode.
    fn decode_batch(&mut self) -> Result<Option<ArrayRef>>;

    /// Decode out the EncUnit at the given row_id_in_chunk in this Chunk.
    fn decode_row_at(&mut self, row_id_in_chunk: usize, len: usize) -> Result<Option<ArrayRef>>;
}

/// The column data is not encoded in dictionary, but Plain.
/// The scope of lifetime 'a is equal to the lifetime of metadata_owner in the reader.
pub struct NoDictColDecoder<'a, R> {
    /// The iterator of EncUnits metadata in a chunk.
    encunit_iter: VectorIter<'a, ForwardsUOffset<fb::EncUnit<'a>>>,
    /// The encoded chunk buffer is used to store the serialized bytes of a chunk.
    encoded_chunk_buf: BytesMut,
    /// The data type of the column.
    data_type: DataType,
    wasm_context: Option<Arc<WASMReadingContext<R>>>,
}

impl<'a, R: Reader> NoDictColDecoder<'a, R> {
    pub fn new(
        encunit_iter: VectorIter<'a, ForwardsUOffset<fb::EncUnit<'a>>>,
        encoded_chunk_buf: BytesMut,
        data_type: DataType,
        wasm_context: Option<Arc<WASMReadingContext<R>>>,
    ) -> Self {
        Self {
            encunit_iter,
            encoded_chunk_buf,
            data_type,
            wasm_context,
        }
    }
}

impl<'a, R: Reader> ChunkDecoder for NoDictColDecoder<'a, R> {
    fn decode_batch(&mut self) -> Result<Option<ArrayRef>> {
        let encunit = self.encunit_iter.next();
        if encunit.is_none() {
            return Ok(None);
        }
        let encblock_fb = encunit.unwrap();

        let data = self
            .encoded_chunk_buf
            .split_to(encblock_fb.size_() as usize);
        let decoder = create_encunit_decoder(
            encblock_fb.encoding().unwrap(),
            encblock_fb.compression(),
            data.freeze(),
            encblock_fb.num_rows() as u64,
            self.data_type.clone(),
            self.wasm_context
                .as_ref()
                .map(|wasm_context| Arc::clone(wasm_context)),
        )?;
        decoder.decode().map(|array| Some(array))
    }

    fn decode_row_at(&mut self, row_id_in_chunk: usize, len: usize) -> Result<Option<ArrayRef>> {
        let mut cur = 0;
        let mut remaining = len;
        let mut arrays = vec![];
        loop {
            let encunit = self.encunit_iter.next();
            if encunit.is_none() {
                break;
            }
            let encblock_fb = encunit.unwrap();
            let last_cur = cur;
            let enc_unit_num_rows = encblock_fb.num_rows() as usize;
            cur += enc_unit_num_rows;
            if cur >= row_id_in_chunk {
                let idx = row_id_in_chunk - last_cur;
                let to_decode = std::cmp::min(remaining, enc_unit_num_rows - idx);
                remaining = remaining - to_decode;
                let data = self
                    .encoded_chunk_buf
                    .split_to(encblock_fb.size_() as usize);
                let decoder = create_encunit_decoder(
                    encblock_fb.encoding().unwrap(),
                    encblock_fb.compression(),
                    data.freeze(),
                    enc_unit_num_rows as u64,
                    self.data_type.clone(),
                    self.wasm_context
                        .as_ref()
                        .map(|wasm_context| Arc::clone(wasm_context)),
                )?;
                // Return the array with only one element at the given index.
                let array = match decoder.slice(idx, idx + to_decode) {
                    Ok(res) => res,
                    Err(_) => decoder.decode().map(|array| array.slice(idx, to_decode))?,
                };
                arrays.push(array);
            } else {
                let _ = self
                    .encoded_chunk_buf
                    .split_to(encblock_fb.size_() as usize);
            }
            if remaining == 0 {
                break;
            }
        }
        Ok(match arrays.is_empty() {
            true => None,
            // TODO: concat is inefficient but should be ok for testing list of structs for now.
            false => Some(arrow::compute::concat(
                arrays
                    .iter()
                    .map(|a| a.as_ref())
                    .collect::<Vec<_>>()
                    .as_slice(),
            )?),
        })
    }
    // Deprecated decode logic with null info
    // fn decode_batch(&mut self) -> Result<Option<ArrayRef>> {
    //     let block = self.encunit_iter.next();
    //     if block.is_none() {
    //         return Ok(None);
    //     }
    //     let block = block.unwrap();
    //     let mut buffers: Vec<BytesMut> = vec![];
    //     if NULLABLE {
    //         let null_info = block
    //             .null_info()
    //             .ok_or_else(|| Error::General("No null info for nullable col".to_string()))?;
    //         match null_info.null_type() {
    //             fb::NullType::AllNull => {
    //                 // TODO: unfortunately, even if AllNull, if the Arrow type is not Null, we have to alloc memory.
    //                 buffers.push(BytesMut::zeroed(bit_util::ceil(
    //                     block.num_rows() as usize,
    //                     8,
    //                 )));
    //                 buffers.push(BytesMut::zeroed(
    //                     block.num_rows() as usize * self.data_type.primitive_width().unwrap(),
    //                 ));
    //             }
    //             fb::NullType::SomeNull => {
    //                 buffers.push(NoDictColDecoder::<NULLABLE>::decode_encblock(
    //                     null_info.validity_block().ok_or_else(|| {
    //                         Error::General("No validity block in block".to_string())
    //                     })?,
    //                     &mut self.encoded_chunk_buf,
    //                 )?);
    //                 for data_block in block
    //                     .data_blocks()
    //                     .ok_or_else(|| Error::General("No data block in block".to_string()))?
    //                 {
    //                     buffers.push(NoDictColDecoder::<NULLABLE>::decode_encblock(
    //                         data_block,
    //                         &mut self.encoded_chunk_buf,
    //                     )?);
    //                 }
    //             }
    //             fb::NullType::NoNull => {
    //                 // TODO: unfortunately, even if NoNull, we have to alloc memory.
    //                 let mut bytes = BytesMut::zeroed(bit_util::ceil(block.num_rows() as usize, 8));
    //                 // fill bytes with 1
    //                 for i in 0..bytes.len() {
    //                     bytes[i] = 0b11111111;
    //                 }
    //                 buffers.push(bytes);
    //                 for data_block in block
    //                     .data_blocks()
    //                     .ok_or_else(|| Error::General("No data block in block".to_string()))?
    //                 {
    //                     buffers.push(NoDictColDecoder::<NULLABLE>::decode_encblock(
    //                         data_block,
    //                         &mut self.encoded_chunk_buf,
    //                     )?);
    //                 }
    //             }
    //             _ => panic!(),
    //         }
    //     } else {
    //         buffers.push(BytesMut::default());
    //         for data_block in block
    //             .data_blocks()
    //             .ok_or_else(|| Error::General("No data block for non-nullable col".to_string()))?
    //         {
    //             buffers.push(NoDictColDecoder::<NULLABLE>::decode_encblock(
    //                 data_block,
    //                 &mut self.encoded_chunk_buf,
    //             )?);
    //         }
    //     }
    //     Ok(Some(primitive_array_from_buffers(
    //         &self.data_type,
    //         buffers,
    //         block.num_rows(),
    //     )?))
    // }
}

/// Local dictionaries are used.
/// The scope of lifetime 'a is equal to the lifetime of metadata_owner in the reader.
pub struct DictColDecoder<'a, R> {
    /// The iterator of EncUnits metadata in a chunk.
    encunit_iter: VectorIter<'a, ForwardsUOffset<fb::EncUnit<'a>>>,
    /// The encoded chunk buffer is used to store the encoded chunk
    encoded_chunk_buf: BytesMut,
    /// The data type of the column.
    data_type: DataType,
    wasm_context: Option<Arc<WASMReadingContext<R>>>,
}

impl<'a, R: Reader> DictColDecoder<'a, R> {
    pub fn new(
        encunit_iter: VectorIter<'a, ForwardsUOffset<fb::EncUnit<'a>>>,
        encoded_chunk_buf: BytesMut,
        data_type: DataType,
        wasm_context: Option<Arc<WASMReadingContext<R>>>,
    ) -> Self {
        Self {
            encunit_iter,
            encoded_chunk_buf,
            data_type,
            wasm_context,
        }
    }
}

macro_rules! index_downcast {
    ($dict_downcast_type: ty, $index_downcast_type: ty, $dict_ref: ident, $indices: ident) => {{
        Ok(Some(Arc::new(<$dict_downcast_type>::from_iter(
            $indices
                .as_any()
                .downcast_ref::<$index_downcast_type>()
                .ok_or_else(|| general_error!("Incorrect type of indices"))?
                .iter()
                .map(|ind_opt| ind_opt.map(|ind| $dict_ref.value(ind as usize))),
        )) as ArrayRef))
    }};
}

macro_rules! dict_index_to_data {
    ($downcast_type: ty, $dict: ident, $indices: ident) => {{
        let dict_ref = $dict.as_any().downcast_ref::<$downcast_type>().unwrap();
        let dict_len = dict_ref.len();
        if dict_len < (1 << 8) {
            index_downcast!($downcast_type, UInt8Array, dict_ref, $indices)
        } else if dict_len < (1 << 16) {
            index_downcast!($downcast_type, UInt16Array, dict_ref, $indices)
        } else if dict_len < (1 << 32) {
            index_downcast!($downcast_type, UInt32Array, dict_ref, $indices)
        } else {
            index_downcast!($downcast_type, UInt64Array, dict_ref, $indices)
        }
    }};
}

impl<'a, R: Reader> ChunkDecoder for DictColDecoder<'a, R> {
    fn decode_batch(&mut self) -> Result<Option<ArrayRef>> {
        let dict_encunit = self.encunit_iter.next();
        if dict_encunit.is_none() {
            return Ok(None);
        }
        let dict_encblock_fb = dict_encunit.unwrap();

        let dict = self
            .encoded_chunk_buf
            .split_to(dict_encblock_fb.size_() as usize);
        let dict_decoder = create_encunit_decoder(
            dict_encblock_fb.encoding().unwrap(),
            dict_encblock_fb.compression(),
            dict.freeze(),
            dict_encblock_fb.num_rows() as u64,
            self.data_type.clone(),
            self.wasm_context
                .as_ref()
                .map(|wasm_context| Arc::clone(wasm_context)),
        )?;
        let dict = if dict_encblock_fb.num_rows() > 0 {
            dict_decoder.decode()?
        } else {
            Arc::new(arrow_array::Int32Array::new_null(1))
        };
        let index_encunit = self.encunit_iter.next();
        if index_encunit.is_none() {
            return nyi_err!("Index does not exists");
        }
        let index_encblock_fb = index_encunit.unwrap();

        let indices = self
            .encoded_chunk_buf
            .split_to(index_encblock_fb.size_() as usize);
        let indices_decoder = create_encunit_decoder(
            index_encblock_fb.encoding().unwrap(),
            index_encblock_fb.compression(),
            indices.freeze(),
            index_encblock_fb.num_rows() as u64,
            DataType::Int64,
            self.wasm_context
                .as_ref()
                .map(|wasm_context| Arc::clone(wasm_context)),
        )?;
        let indices_ref = indices_decoder.decode()?;
        let indices = indices_ref.as_any().downcast_ref::<UInt64Array>().ok_or(
            fff_core::errors::Error::General("Incorrect type of indices".to_owned()),
        )?;
        // Create an array of the same type as dict, then map
        // TODO: use DictionaryArray with zero-copy
        // DictionaryArray::<arrow::datatypes::Int64Type>::try_new(indices.clone(), dict)
        //     .map_err(|err| fff_core::errors::Error::External(Box::new(err)))
        //     .map(|arr| Some(Arc::new(arr) as ArrayRef))
        match *dict.data_type() {
            DataType::Int32 => {
                dict_index_to_data!(arrow_array::Int32Array, dict, indices)
            }
            DataType::Int64 => {
                dict_index_to_data!(arrow_array::Int64Array, dict, indices)
            }
            DataType::Float32 => {
                dict_index_to_data!(arrow_array::Float32Array, dict, indices)
            }
            DataType::Float64 => {
                dict_index_to_data!(arrow_array::Float64Array, dict, indices)
            }
            DataType::Utf8 | DataType::LargeUtf8 => {
                dict_index_to_data!(arrow_array::StringArray, dict, indices)
            }
            DataType::Utf8View => {
                dict_index_to_data!(arrow_array::StringViewArray, dict, indices)
            }
            DataType::Timestamp(TimeUnit::Second, _) => {
                dict_index_to_data!(arrow_array::TimestampSecondArray, dict, indices)
            }
            DataType::Timestamp(TimeUnit::Millisecond, _) => {
                dict_index_to_data!(arrow_array::TimestampMillisecondArray, dict, indices)
            }
            DataType::Timestamp(TimeUnit::Microsecond, _) => {
                dict_index_to_data!(arrow_array::TimestampMicrosecondArray, dict, indices)
            }
            DataType::Timestamp(TimeUnit::Nanosecond, _) => {
                dict_index_to_data!(arrow_array::TimestampNanosecondArray, dict, indices)
            }
            DataType::Time32(TimeUnit::Second) => {
                dict_index_to_data!(arrow_array::Time32SecondArray, dict, indices)
            }
            DataType::Time32(TimeUnit::Millisecond) => {
                dict_index_to_data!(arrow_array::Time32MillisecondArray, dict, indices)
            }
            DataType::Time64(TimeUnit::Microsecond) => {
                dict_index_to_data!(arrow_array::Time64MicrosecondArray, dict, indices)
            }
            DataType::Time64(TimeUnit::Nanosecond) => {
                dict_index_to_data!(arrow_array::Time64NanosecondArray, dict, indices)
            }
            DataType::Date32 => {
                dict_index_to_data!(arrow_array::Date32Array, dict, indices)
            }
            DataType::Boolean => {
                dict_index_to_data!(arrow_array::BooleanArray, dict, indices)
            }
            _ => Err(fff_core::errors::Error::General(
                "Decoding other datatypes are not supported".to_owned(),
            )),
        }
    }

    fn decode_row_at(&mut self, _row_id_in_chunk: usize, _len: usize) -> Result<Option<ArrayRef>> {
        // TODO: random access for dict (decode the index first then the dict?)
        nyi_err!("Random access for dict is not implemented yet")
    }
}

/// Local dictionaries are used.
/// The scope of lifetime 'a is equal to the lifetime of metadata_owner in the reader.
pub struct SharedDictColDecoder<'a, R> {
    /// The iterator of EncUnits metadata in a chunk.
    encunit_iter: VectorIter<'a, ForwardsUOffset<fb::EncUnit<'a>>>,
    /// The encoded chunk buffer is used to store the encoded chunk
    encoded_chunk_buf: BytesMut,
    /// The data type of the column.
    _data_type: DataType,
    wasm_context: Option<Arc<WASMReadingContext<R>>>,
    shared_dictionary: ArrayRef,
}

impl<'a, R: Reader> SharedDictColDecoder<'a, R> {
    pub fn new(
        encunit_iter: VectorIter<'a, ForwardsUOffset<fb::EncUnit<'a>>>,
        encoded_chunk_buf: BytesMut,
        data_type: DataType,
        wasm_context: Option<Arc<WASMReadingContext<R>>>,
        shared_dictionary: ArrayRef,
    ) -> Self {
        Self {
            encunit_iter,
            encoded_chunk_buf,
            _data_type: data_type,
            wasm_context,
            shared_dictionary,
        }
    }
}

impl<'a, R: Reader> ChunkDecoder for SharedDictColDecoder<'a, R> {
    fn decode_batch(&mut self) -> Result<Option<ArrayRef>> {
        let index_encunit = self.encunit_iter.next();
        if index_encunit.is_none() {
            return Ok(None);
        }
        let index_encblock_fb = index_encunit.unwrap();

        let indices = self
            .encoded_chunk_buf
            .split_to(index_encblock_fb.size_() as usize);
        let indices_decoder = create_encunit_decoder(
            index_encblock_fb.encoding().unwrap(),
            index_encblock_fb.compression(),
            indices.freeze(),
            index_encblock_fb.num_rows() as u64,
            DataType::Int64,
            self.wasm_context
                .as_ref()
                .map(|wasm_context| Arc::clone(wasm_context)),
        )?;
        let indices = indices_decoder.decode()?;
        let dict = &self.shared_dictionary;
        // Create an array of the same type as dict, then map
        match dict.data_type() {
            DataType::Int32 => {
                dict_index_to_data!(arrow_array::Int32Array, dict, indices)
            }
            DataType::Int64 => {
                dict_index_to_data!(arrow_array::Int64Array, dict, indices)
            }
            DataType::Float32 => {
                dict_index_to_data!(arrow_array::Float32Array, dict, indices)
            }
            DataType::Float64 => {
                dict_index_to_data!(arrow_array::Float64Array, dict, indices)
            }
            DataType::Utf8 | DataType::LargeUtf8 => {
                dict_index_to_data!(arrow_array::StringArray, dict, indices)
            }
            DataType::Utf8View => {
                dict_index_to_data!(arrow_array::StringViewArray, dict, indices)
            }
            DataType::Timestamp(TimeUnit::Second, _) => {
                dict_index_to_data!(arrow_array::TimestampSecondArray, dict, indices)
            }
            DataType::Timestamp(TimeUnit::Millisecond, _) => {
                dict_index_to_data!(arrow_array::TimestampMillisecondArray, dict, indices)
            }
            DataType::Timestamp(TimeUnit::Microsecond, _) => {
                dict_index_to_data!(arrow_array::TimestampMicrosecondArray, dict, indices)
            }
            DataType::Timestamp(TimeUnit::Nanosecond, _) => {
                dict_index_to_data!(arrow_array::TimestampNanosecondArray, dict, indices)
            }
            DataType::Time32(TimeUnit::Second) => {
                dict_index_to_data!(arrow_array::Time32SecondArray, dict, indices)
            }
            DataType::Time32(TimeUnit::Millisecond) => {
                dict_index_to_data!(arrow_array::Time32MillisecondArray, dict, indices)
            }
            DataType::Time64(TimeUnit::Microsecond) => {
                dict_index_to_data!(arrow_array::Time64MicrosecondArray, dict, indices)
            }
            DataType::Time64(TimeUnit::Nanosecond) => {
                dict_index_to_data!(arrow_array::Time64NanosecondArray, dict, indices)
            }
            DataType::Date32 => {
                dict_index_to_data!(arrow_array::Date32Array, dict, indices)
            }
            DataType::Boolean => {
                dict_index_to_data!(arrow_array::BooleanArray, dict, indices)
            }
            _ => Err(fff_core::errors::Error::General(
                "Decoding other datatypes are not supported".to_owned(),
            )),
        }
    }

    fn decode_row_at(&mut self, _row_id_in_chunk: usize, _len: usize) -> Result<Option<ArrayRef>> {
        // TODO: random access for dict (decode the index first then the dict?)
        nyi_err!("Random access for dict is not implemented yet")
    }
}

pub fn create_physical_decoder<'a, R: Reader + 'a>(
    encunit_iter: VectorIter<'a, ForwardsUOffset<fb::EncUnit<'a>>>,
    dict_encoding_type: fb::DictionaryEncoding,
    shared_dictionary_id: Option<fb::SharedDictionary>,
    data_type: &DataType,
    encoded_chunk_buf: BytesMut,
    wasm_context: Option<Arc<WASMReadingContext<R>>>,
    shared_dictionary_cache: Option<&'a SharedDictionaryCache>,
) -> Result<Box<dyn ChunkDecoder + 'a>> {
    if dict_encoding_type == fb::DictionaryEncoding::NoDictionary {
        match *data_type {
            non_nest_types!() | DataType::List(_) | DataType::LargeList(_) => {
                Ok(Box::new(NoDictColDecoder::new(
                    encunit_iter,
                    encoded_chunk_buf,
                    data_type.clone(),
                    wasm_context,
                )))
            }
            _ => todo!("Implement other data types"),
        }
    } else if dict_encoding_type == fb::DictionaryEncoding::LocalDictionary {
        match *data_type {
            non_nest_types!() | DataType::List(_) | DataType::LargeList(_) => {
                Ok(Box::new(DictColDecoder::new(
                    encunit_iter,
                    encoded_chunk_buf,
                    data_type.clone(),
                    wasm_context,
                )))
            }
            _ => todo!("Implement other data types"),
        }
    } else if dict_encoding_type == fb::DictionaryEncoding::SharedDictionary {
        match *data_type {
            non_nest_types!() | DataType::List(_) | DataType::LargeList(_) => {
                Ok(Box::new(SharedDictColDecoder::new(
                    encunit_iter,
                    encoded_chunk_buf,
                    data_type.clone(),
                    wasm_context,
                    shared_dictionary_cache
                        .ok_or_else(|| {
                            general_error!(
                                "Shared dictionary cache not found for a shared dictionary column"
                            )
                        })?
                        .get_dict(
                            shared_dictionary_id
                                .ok_or_else(|| general_error!("Shared dictionary ID not found"))?
                                .shared_dictionary_idx() as usize,
                        )
                        .ok_or_else(|| general_error!("Shared dictionary not found in cache"))?,
                )))
            }
            _ => todo!("Implement other data types"),
        }
    } else {
        todo!("Implement shared dict")
    }
}
