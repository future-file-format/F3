use std::{io::Cursor, sync::Arc};

use crate::{
    compression::compress_data,
    context::WASMWritingContext,
    counter::EncodingCounter,
    dict::{shared_dictionary_context::SharedDictionaryContext, Dictionary, DictionaryTypeOptions},
    file::footer::{self, WASMEncoding},
};
use arrow::{array::AsArray, datatypes::UInt64Type};
use arrow_array::{array::ArrayRef, Array, UInt16Array, UInt32Array, UInt8Array};
use arrow_schema::DataType;
use bytes::Bytes;
use fff_core::{errors::Result, non_nest_types};
use fff_format::File::fff::flatbuf as fb;
use itertools::Itertools;
use rand::seq::IteratorRandom;

use super::{
    encoded_column_chunk::{EncodedColumnChunk, SerializedEncUnit},
    encunit::create_encunit_encoder,
};

use fff_encoding::schemes::{encode_to_bytes, vortex::VortexEncoder, Encoder};

/// This level handles using dictionary or not.
pub trait PhysicalColEncoder {
    /// TODO: Let us only consider sync writing.
    fn encode(
        &mut self,
        array: ArrayRef,
        counter: &mut EncodingCounter,
        shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Vec<EncodedColumnChunk>>;

    /// Return the size of the accumulated chunk in this encoder.
    fn memory_size(&self) -> usize;

    fn finish(
        &mut self,
        counter: &mut EncodingCounter,
        shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Vec<EncodedColumnChunk>>;

    fn submit_dict(&mut self, shared_dict_ctx: &mut SharedDictionaryContext) -> Result<()>;
}

/// A specific experimental encoder for testing List of Struct of non nest types.
pub struct ListOfStructColEncoder {
    // TODO: in-memory buffer size threshold and flush size threshold
    accumulated_chunk: EncodedColumnChunk,
    accumulated_size: u64,
    /// The desired encoded column chunk size, should match I/O unit size (e.g., 8MB on S3)
    column_chunk_size: u64,
    wasm_context: Arc<WASMWritingContext>,
    compression_type: fb::CompressionType,
}

impl ListOfStructColEncoder {
    pub fn new(
        column_chunk_size: u64,
        wasm_context: Arc<WASMWritingContext>,
        compression_type: fb::CompressionType,
    ) -> Self {
        Self {
            accumulated_chunk: EncodedColumnChunk::builder()
                .set_dict_encoding(footer::DictionaryEncoding::NoDictionary)
                .build(),
            accumulated_size: 0,
            column_chunk_size,
            wasm_context,
            compression_type,
        }
    }

    pub fn memory_size(&self) -> usize {
        self.accumulated_size as usize
    }

    pub fn encode(
        &mut self,
        list_array: ArrayRef,
        field: ArrayRef,
    ) -> Result<Option<EncodedColumnChunk>> {
        let encoder = VortexEncoder::default();
        let list_len = list_array.len();
        let enc_unit: Bytes = {
            let encblock = encoder
                .list_struct_encode(list_array.clone(), field.clone())
                .unwrap();
            let mut buffer = Vec::new();
            let mut cursor = Cursor::new(&mut buffer);
            encblock.try_serialize(&mut cursor).unwrap();
            buffer.into()
        };

        // Compress the data if compression is enabled
        let compressed_enc_unit = compress_data(enc_unit, self.compression_type)?;
        let compressed_size = compressed_enc_unit.len() as u64;

        self.accumulated_size += compressed_size;
        self.accumulated_chunk.encunits.push(SerializedEncUnit::new(
            compressed_enc_unit,
            list_len as u32,
            {
                let encoding_type = encoder.encoding_type();
                footer::Encoding::try_new(
                    if self.wasm_context.always_set_custom_wasm_for_built_in() {
                        fb::EncodingType::CUSTOM_WASM
                    } else {
                        encoding_type.to_fbs_encoding()
                    },
                    if self.wasm_context.always_set_custom_wasm_for_built_in() {
                        self.wasm_context.builtin_wasm_id()
                    } else {
                        self.wasm_context
                            .data_type_to_wasm_id(list_array.data_type())
                    }
                    .map(|id| WASMEncoding::new(id.0, Vec::new())),
                )?
            },
            self.compression_type,
        ));
        self.accumulated_chunk.num_rows += list_len;
        if self.accumulated_size > self.column_chunk_size {
            let chunk = std::mem::take(&mut self.accumulated_chunk);
            self.accumulated_size = 0;
            Ok(Some(chunk))
        } else {
            Ok(None)
        }
    }

    pub fn finish(&mut self) -> Result<Option<EncodedColumnChunk>> {
        match self.accumulated_chunk.encunits.len() {
            0 => Ok(None),
            _ => Ok(Some(std::mem::take(&mut self.accumulated_chunk))),
        }
    }
}

/// No dictionary is used. Encoding is based on EncUnit.
pub struct EncoderDictColEncoder {
    // TODO: in-memory buffer size threshold and flush size threshold
    accumulated_chunk: EncodedColumnChunk,
    accumulated_size: u64,
    /// The desired encoded column chunk size, should match I/O unit size (e.g., 8MB on S3)
    column_chunk_size: u64,
    wasm_context: Arc<WASMWritingContext>,
    enable_dict: bool,
    compression_type: fb::CompressionType,
}

impl EncoderDictColEncoder {
    pub fn new(
        column_chunk_size: u64,
        wasm_context: Arc<WASMWritingContext>,
        enable_dict: bool,
        compression_type: fb::CompressionType,
    ) -> Self {
        Self {
            accumulated_chunk: EncodedColumnChunk::builder()
                .set_dict_encoding(footer::DictionaryEncoding::NoDictionary)
                .build(),
            accumulated_size: 0,
            column_chunk_size,
            wasm_context,
            enable_dict,
            compression_type,
        }
    }
}

impl PhysicalColEncoder for EncoderDictColEncoder {
    fn encode(
        &mut self,
        array: ArrayRef,
        counter: &mut EncodingCounter,
        _shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Vec<EncodedColumnChunk>> {
        let encoder = create_encunit_encoder(
            self.wasm_context.clone(),
            array.data_type().clone(),
            self.enable_dict,
        );
        let enc_unit = encode_to_bytes(encoder.clone(), array.clone());

        // Compress the data if compression is enabled
        let compressed_enc_unit = compress_data(enc_unit, self.compression_type)?;
        let compressed_size = compressed_enc_unit.len() as u64;

        // Update accumulated size with compressed size
        self.accumulated_size += compressed_size;
        counter.index_size += compressed_enc_unit.len();

        self.accumulated_chunk.encunits.push(SerializedEncUnit::new(
            compressed_enc_unit,
            array.len() as u32,
            {
                let encoding_type = encoder.encoding_type();
                footer::Encoding::try_new(
                    if self.wasm_context.always_set_custom_wasm_for_built_in() {
                        fb::EncodingType::CUSTOM_WASM
                    } else {
                        encoding_type.to_fbs_encoding()
                    },
                    if self.wasm_context.always_set_custom_wasm_for_built_in() {
                        self.wasm_context.builtin_wasm_id()
                    } else {
                        self.wasm_context.data_type_to_wasm_id(array.data_type())
                    }
                    .map(|id| WASMEncoding::new(id.0, Vec::new())),
                )?
            },
            self.compression_type,
        ));
        self.accumulated_chunk.num_rows += array.len();
        if self.accumulated_size > self.column_chunk_size {
            let chunk = std::mem::take(&mut self.accumulated_chunk);
            self.accumulated_size = 0;
            Ok(vec![chunk])
        } else {
            Ok(vec![])
        }
    }

    fn memory_size(&self) -> usize {
        self.accumulated_size as usize
    }

    fn finish(
        &mut self,
        counter: &mut EncodingCounter,
        _shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Vec<EncodedColumnChunk>> {
        counter.dict_type = DictionaryTypeOptions::EncoderDictionary;
        match self.accumulated_chunk.encunits.len() {
            0 => Ok(vec![]),
            _ => {
                self.accumulated_size = 0;
                Ok(vec![std::mem::take(&mut self.accumulated_chunk)])
            }
        }
    }

    fn submit_dict(&mut self, _shared_dict_ctx: &mut SharedDictionaryContext) -> Result<()> {
        Ok(())
    }
    //  /// Note: deprecated encode function when null info is explicitly managed.
    // /// If Nullable, each time we flush a validity page first and then data page.
    // fn encode(&mut self, array: ArrayRef) -> Result<Option<EncodedColumnChunk>> {
    //     let get_data_blocks = || -> Result<Vec<EncUnit>> {
    //         let buffers = array
    //             .to_data()
    //             .buffers()
    //             .iter()
    //             .map(|buffer| buffer.clone())
    //             .collect::<Vec<Buffer>>();
    //         Self::encode_encblock(buffers, array.data_type().clone())
    //     };
    //     // 1. nullable and array.nulls null count=len: all null, no validity, no data block
    //     // 2. nullable and some null: both are needed
    //     // 3. not nullable: no validity, only data block
    //     let (null_info, data_blocks) = match (NULLABLE, array.null_count()) {
    //         (true, count) if count == array.len() => (
    //             Some(NullInfo {
    //                 null_type: fb::NullType::AllNull,
    //                 validity_block: None,
    //             }),
    //             None,
    //         ),
    //         (true, count) if count == 0 => (
    //             Some(NullInfo {
    //                 null_type: fb::NullType::NoNull,
    //                 validity_block: None,
    //             }),
    //             Some(get_data_blocks()?),
    //         ),
    //         (true, _) => {
    //             let buffer = array.nulls().ok_or_else(|| {
    //                 fff_core::errors::Error::General(
    //                     "Array is nullable but null buffer is missing".to_string(),
    //                 )
    //             })?;
    //             (
    //                 Some(NullInfo {
    //                     null_type: fb::NullType::SomeNull,
    //                     validity_block: Some(EncodedUnit {
    //                         buffer: {
    //                             // TODO: better way to copy validity values?
    //                             let mut builder = BooleanBufferBuilder::new(array.len() as usize);
    //                             builder.append_buffer(buffer.inner());
    //                             builder.finish().into_inner()
    //                         },
    //                         encoding: footer::Encoding::try_new(fb::EncodingType::PLAIN, None)?,
    //                     }),
    //                 }),
    //                 Some(get_data_blocks()?),
    //             )
    //         }
    //         (false, _) => (None, Some(get_data_blocks()?)),
    //     };
    //     let size = data_blocks
    //         .as_ref()
    //         .map_or(0, |x| x.iter().map(|chunk| chunk.buffer.len() as u64).sum())
    //         + null_info.as_ref().map_or(0, |info| {
    //             info.validity_block
    //                 .as_ref()
    //                 .map_or(0, |x| x.buffer.len() as u64)
    //         });
    //     self.accumulated_chunk.encunits.push(Block {
    //         num_rows: array.len() as u64,
    //         null_info,
    //         data_blocks,
    //     });
    //     self.accumulated_chunk.num_rows += array.len();
    //     self.accumulated_size += size;
    //     if size + self.accumulated_size > self.column_chunk_size {
    //         self.accumulated_size = 0;
    //         Ok(Some(std::mem::take(&mut self.accumulated_chunk)))
    //     } else {
    //         Ok(None)
    //     }
    // }
}

/// Cast the datatype of index array to smallest unsigned int
fn cast_index_dtype(index_array: ArrayRef, dict_len: usize) -> ArrayRef {
    if dict_len < (1 << 8) {
        Arc::new(UInt8Array::from_iter(
            index_array
                .as_primitive::<UInt64Type>()
                .iter()
                .map(|x| x.map(|y| y as u8)),
        ))
    } else if dict_len < (1 << 16) {
        Arc::new(UInt16Array::from_iter(
            index_array
                .as_primitive::<UInt64Type>()
                .iter()
                .map(|x| x.map(|y| y as u16)),
        ))
    } else if dict_len < (1 << 32) {
        Arc::new(UInt32Array::from_iter(
            index_array
                .as_primitive::<UInt64Type>()
                .iter()
                .map(|x| x.map(|y| y as u32)),
        ))
    } else {
        index_array
    }
}

/// Local dictionaries are used. Encoding is based on EncUnit.
pub struct DictColEncoder {
    // TODO: in-memory buffer size threshold and flush size threshold
    accumulated_chunk: EncodedColumnChunk,
    accumulated_size: u64,
    /// The desired encoded column chunk size, should match I/O unit size (e.g., 8MB on S3)
    column_chunk_size: u64,
    wasm_context: Arc<WASMWritingContext>,
    compression_type: fb::CompressionType,
}

impl DictColEncoder {
    pub fn new(
        column_chunk_size: u64,
        wasm_context: Arc<WASMWritingContext>,
        compression_type: fb::CompressionType,
    ) -> Self {
        Self {
            accumulated_chunk: EncodedColumnChunk::builder()
                // TODO: currently we use a dictionary for each **encoding unit**
                // And stores the dict first, then the indices
                // So this may not be needed
                // until we change to share a dict in a chunk
                // (how to determine the chunk size then?)
                .set_dict_encoding(footer::DictionaryEncoding::Dictionary(vec![]))
                .build(),
            accumulated_size: 0,
            column_chunk_size,
            wasm_context,
            compression_type,
        }
    }
}

impl PhysicalColEncoder for DictColEncoder {
    fn encode(
        &mut self,
        array: ArrayRef,
        counter: &mut EncodingCounter,
        _shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Vec<EncodedColumnChunk>> {
        let dtype = array.data_type().clone();
        let mut dict = Dictionary::try_new(dtype.clone())?;
        dict.extend(array)?;
        let (dict, indices) = dict.finish()?;
        let indices = cast_index_dtype(indices, dict.len());
        let indices_dtype = indices.data_type().clone();
        let dict_encoder =
            create_encunit_encoder(self.wasm_context.clone(), dict.data_type().clone(), false);
        let dict_enc_unit = encode_to_bytes(dict_encoder.clone(), dict.clone());
        let indices_encoder = create_encunit_encoder(
            self.wasm_context.clone(),
            indices.data_type().clone(),
            false,
        );
        let indices_enc_unit = encode_to_bytes(indices_encoder.clone(), indices.clone());

        // Compress the dictionary data if compression is enabled
        let compressed_dict_enc_unit = compress_data(dict_enc_unit, self.compression_type)?;
        let compressed_indices_enc_unit = compress_data(indices_enc_unit, self.compression_type)?;

        let dict_compressed_size = compressed_dict_enc_unit.len() as u64;
        let indices_compressed_size = compressed_indices_enc_unit.len() as u64;

        // Update accumulated size with compressed sizes
        self.accumulated_size += dict_compressed_size + indices_compressed_size;
        counter.dict_size += compressed_dict_enc_unit.len();
        counter.index_size += compressed_indices_enc_unit.len();

        self.accumulated_chunk.encunits.push(SerializedEncUnit::new(
            compressed_dict_enc_unit,
            // TODO: be careful this num_rows does not correspond to the original table
            dict.len() as u32,
            {
                let encoding_type = dict_encoder.encoding_type();
                footer::Encoding::try_new(
                    if self.wasm_context.always_set_custom_wasm_for_built_in() {
                        fb::EncodingType::CUSTOM_WASM
                    } else {
                        encoding_type.to_fbs_encoding()
                    },
                    if self.wasm_context.always_set_custom_wasm_for_built_in() {
                        self.wasm_context.builtin_wasm_id()
                    } else {
                        self.wasm_context.data_type_to_wasm_id(&dtype)
                    }
                    .map(|id| WASMEncoding::new(id.0, Vec::new())),
                )?
            },
            self.compression_type,
        ));
        self.accumulated_chunk.encunits.push(SerializedEncUnit::new(
            compressed_indices_enc_unit,
            indices.len() as u32,
            {
                let encoding_type = indices_encoder.encoding_type();
                footer::Encoding::try_new(
                    if self.wasm_context.always_set_custom_wasm_for_built_in() {
                        fb::EncodingType::CUSTOM_WASM
                    } else {
                        encoding_type.to_fbs_encoding()
                    },
                    if self.wasm_context.always_set_custom_wasm_for_built_in() {
                        self.wasm_context.builtin_wasm_id()
                    } else {
                        self.wasm_context.data_type_to_wasm_id(&indices_dtype)
                    }
                    .map(|id| WASMEncoding::new(id.0, Vec::new())),
                )?
            },
            self.compression_type,
        ));
        self.accumulated_chunk.num_rows += indices.len() as usize;
        if self.accumulated_size > self.column_chunk_size {
            self.accumulated_size = 0;
            let chunk = std::mem::take(&mut self.accumulated_chunk);
            self.accumulated_chunk.dict_encoding = footer::DictionaryEncoding::Dictionary(vec![]);
            Ok(vec![chunk])
        } else {
            Ok(vec![])
        }
    }

    fn memory_size(&self) -> usize {
        self.accumulated_size as usize
    }

    fn finish(
        &mut self,
        counter: &mut EncodingCounter,
        _shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Vec<EncodedColumnChunk>> {
        counter.dict_type = DictionaryTypeOptions::LocalDictionary;
        match self.accumulated_chunk.encunits.len() {
            0 => Ok(vec![]),
            _ => Ok(vec![std::mem::take(&mut self.accumulated_chunk)]),
        }
    }

    fn submit_dict(&mut self, _shared_dict_ctx: &mut SharedDictionaryContext) -> Result<()> {
        Ok(())
    }
}

/// Shared dictionaries are used.
pub struct SharedDictColEncoder {
    fixed_dict_scope: u64,
    buffered_arrays: Vec<ArrayRef>,
    buffered_array_len: u64,
    buffered_array_mem_size: usize,
    /// The desired encoded column chunk size, should match I/O unit size (e.g., 8MB on S3)
    column_chunk_size: u64,
    wasm_context: Arc<WASMWritingContext>,
    submitted_dict_idx: Option<u32>,
    compression_type: fb::CompressionType,
}

impl SharedDictColEncoder {
    pub fn new(
        fixed_dict_scope: u64,
        column_chunk_size: u64,
        wasm_context: Arc<WASMWritingContext>,
        compression_type: fb::CompressionType,
    ) -> Self {
        Self {
            fixed_dict_scope,
            buffered_arrays: vec![],
            buffered_array_len: 0,
            buffered_array_mem_size: 0,
            column_chunk_size,
            wasm_context,
            submitted_dict_idx: None,
            compression_type,
        }
    }

    fn encode_dict_scope(
        &mut self,
        counter: &mut EncodingCounter,
        shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Vec<EncodedColumnChunk>> {
        let buffered_arrs = std::mem::take(&mut self.buffered_arrays);
        self.buffered_array_len = 0;
        self.buffered_array_mem_size = 0;
        let dict_idx = match self.submitted_dict_idx {
            Some(idx) => idx,
            None => shared_dict_ctx
                .new_dictionary(buffered_arrs.first().unwrap().data_type().clone())?,
        };
        let indices_arrs = buffered_arrs
            .into_iter()
            .map(|arr| shared_dict_ctx.extend_and_get_index(dict_idx, arr))
            .collect::<Result<Vec<_>>>()?;
        let dict_len = shared_dict_ctx.dict_len(dict_idx)?;
        let indices_arrs = indices_arrs
            .into_iter()
            .map(|arr| cast_index_dtype(arr, dict_len))
            .collect::<Vec<_>>();
        let mut res = vec![];
        let mut accumulated_chunk = EncodedColumnChunk::builder()
            .set_dict_encoding(footer::DictionaryEncoding::SharedDictionary(dict_idx))
            .build();
        let mut accumulated_size = 0;
        for arr in indices_arrs {
            let encoder =
                create_encunit_encoder(self.wasm_context.clone(), arr.data_type().clone(), false);
            let enc_unit = encode_to_bytes(encoder.clone(), arr.clone());

            // Compress the data if compression is enabled
            let compressed_enc_unit = compress_data(enc_unit, self.compression_type)?;
            let compressed_size = compressed_enc_unit.len() as u64;

            accumulated_size += compressed_size;
            counter.index_size += compressed_enc_unit.len();
            accumulated_chunk.encunits.push(SerializedEncUnit::new(
                compressed_enc_unit,
                arr.len() as u32,
                {
                    let encoding_type = encoder.encoding_type();
                    footer::Encoding::try_new(
                        if self.wasm_context.always_set_custom_wasm_for_built_in() {
                            fb::EncodingType::CUSTOM_WASM
                        } else {
                            encoding_type.to_fbs_encoding()
                        },
                        if self.wasm_context.always_set_custom_wasm_for_built_in() {
                            self.wasm_context.builtin_wasm_id()
                        } else {
                            self.wasm_context.data_type_to_wasm_id(arr.data_type())
                        }
                        .map(|id| WASMEncoding::new(id.0, Vec::new())),
                    )?
                },
                self.compression_type,
            ));
            accumulated_chunk.num_rows += arr.len();
            if accumulated_size > self.column_chunk_size {
                res.push(accumulated_chunk);
                accumulated_chunk = EncodedColumnChunk::builder()
                    .set_dict_encoding(footer::DictionaryEncoding::SharedDictionary(dict_idx))
                    .build();
                accumulated_size = 0;
            }
        }
        if accumulated_size > 0 {
            res.push(accumulated_chunk);
        }
        Ok(res)
    }
}

impl PhysicalColEncoder for SharedDictColEncoder {
    fn encode(
        &mut self,
        array: ArrayRef,
        counter: &mut EncodingCounter,
        shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Vec<EncodedColumnChunk>> {
        if self
            .buffered_arrays
            .first()
            .is_some_and(|buf_arr| *buf_arr.data_type() != *array.data_type())
        {
            Err(fff_core::errors::Error::General(
                "Datatypes of arrays do not match".to_owned(),
            ))
        } else {
            self.buffered_array_len += array.len() as u64;
            self.buffered_array_mem_size += array.get_array_memory_size();
            self.buffered_arrays.push(array);
            if self.buffered_array_len >= self.fixed_dict_scope {
                self.encode_dict_scope(counter, shared_dict_ctx)
            } else {
                Ok(vec![])
            }
        }
    }

    fn memory_size(&self) -> usize {
        self.buffered_array_mem_size
    }

    fn finish(
        &mut self,
        counter: &mut EncodingCounter,
        shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Vec<EncodedColumnChunk>> {
        if !self.buffered_arrays.is_empty() {
            self.encode_dict_scope(counter, shared_dict_ctx)
        } else {
            Ok(vec![])
        }
    }

    fn submit_dict(&mut self, shared_dict_ctx: &mut SharedDictionaryContext) -> Result<()> {
        let dict_idx = match self.submitted_dict_idx {
            Some(idx) => idx,
            None => {
                let idx = shared_dict_ctx
                    .new_dictionary(self.buffered_arrays.first().unwrap().data_type().clone())?;
                self.submitted_dict_idx = Some(idx);
                idx
            }
        };
        self.buffered_arrays
            .iter()
            .map(|arr| shared_dict_ctx.submit_values(dict_idx, arr.clone()))
            .collect::<Result<Vec<_>>>()?;
        Ok(())
    }
}

/// Best of global/local dictionaries is used (may use sampling to estimate).
pub struct GLBestEncoder {
    sample_size: Option<(f64, usize)>,
    buffered_arrays: Vec<ArrayRef>,
    buffered_array_len: u64,
    buffered_array_mem_size: usize,
    /// The desired encoded column chunk size, should match I/O unit size (e.g., 8MB on S3)
    column_chunk_size: u64,
    wasm_context: Arc<WASMWritingContext>,
    compression_type: fb::CompressionType,
}

impl GLBestEncoder {
    pub fn new(
        sample_size: Option<(f64, usize)>,
        column_chunk_size: u64,
        wasm_context: Arc<WASMWritingContext>,
        compression_type: fb::CompressionType,
    ) -> Self {
        Self {
            sample_size,
            buffered_arrays: vec![],
            buffered_array_len: 0,
            buffered_array_mem_size: 0,
            column_chunk_size,
            wasm_context,
            compression_type,
        }
    }

    fn estimate_arrays_encoded_size(
        &self,
        arrs: &[ArrayRef],
        arr_total_size: usize,
        sample_count: usize,
    ) -> Result<f64> {
        let mut counter = EncodingCounter::default();
        if arrs.len() <= sample_count {
            self.encode_shared_to_chunks(arrs, &mut counter, Some(1))?;
            Ok(counter.index_size as f64)
        } else {
            let sample_arrs = (0..arrs.len())
                .choose_multiple(&mut rand::thread_rng(), sample_count)
                .iter()
                .map(|i| arrs[*i].clone())
                .collect::<Vec<_>>();
            self.encode_shared_to_chunks(&sample_arrs, &mut counter, Some(1))?;
            Ok((counter.index_size * arr_total_size) as f64
                / (sample_arrs.iter().map(|x| x.len()).sum::<usize>()) as f64)
        }
    }

    fn estimate_arr_encoded_size(
        &self,
        arr: ArrayRef,
        sample_len: usize,
        sample_count: usize,
    ) -> Result<f64> {
        let mut counter = EncodingCounter::default();
        if arr.len() <= sample_len * sample_count {
            self.encode_shared_to_chunks(&[arr], &mut counter, None)?;
            Ok(counter.dict_size as f64)
        } else {
            let num_slices = arr.len() / sample_len;
            let sample_dict_arrs = (0..num_slices)
                .choose_multiple(&mut rand::thread_rng(), sample_count)
                .iter()
                .map(|&i| arr.slice(i * sample_len, sample_len))
                .collect::<Vec<_>>();
            self.encode_shared_to_chunks(&sample_dict_arrs, &mut counter, None)?;
            Ok((counter.dict_size * arr.len()) as f64
                / (sample_dict_arrs.iter().map(|x| x.len()).sum::<usize>()) as f64)
        }
    }

    fn encode_shared_to_chunks(
        &self,
        arrs: &[ArrayRef],
        counter: &mut EncodingCounter,
        dict_idx: Option<u32>,
    ) -> Result<Vec<EncodedColumnChunk>> {
        let mut res = vec![];
        let dict_enc = if let Some(dict_idx) = dict_idx {
            footer::DictionaryEncoding::SharedDictionary(dict_idx)
        } else {
            footer::DictionaryEncoding::NoDictionary
        };
        let mut accumulated_chunk = EncodedColumnChunk::builder()
            .set_dict_encoding(dict_enc.clone())
            .build();
        let mut accumulated_size = 0;
        for arr in arrs {
            let encoder =
                create_encunit_encoder(self.wasm_context.clone(), arr.data_type().clone(), false);
            let enc_unit = encode_to_bytes(encoder.clone(), arr.clone());

            // Compress the data if compression is enabled
            let compressed_enc_unit = compress_data(enc_unit, self.compression_type)?;
            let compressed_size = compressed_enc_unit.len() as u64;

            accumulated_size += compressed_size;
            if dict_idx.is_some() {
                counter.index_size += compressed_enc_unit.len();
            } else {
                counter.dict_size += compressed_enc_unit.len();
            }
            accumulated_chunk.encunits.push(SerializedEncUnit::new(
                compressed_enc_unit,
                arr.len() as u32,
                {
                    let encoding_type = encoder.encoding_type();
                    footer::Encoding::try_new(
                        if self.wasm_context.always_set_custom_wasm_for_built_in() {
                            fb::EncodingType::CUSTOM_WASM
                        } else {
                            encoding_type.to_fbs_encoding()
                        },
                        if self.wasm_context.always_set_custom_wasm_for_built_in() {
                            self.wasm_context.builtin_wasm_id()
                        } else {
                            self.wasm_context.data_type_to_wasm_id(arr.data_type())
                        }
                        .map(|id| WASMEncoding::new(id.0, Vec::new())),
                    )?
                },
                self.compression_type,
            ));
            accumulated_chunk.num_rows += arr.len();
            // Only split to multiple chunks for indices
            if dict_idx.is_some() && accumulated_size > self.column_chunk_size {
                res.push(accumulated_chunk);
                accumulated_chunk = EncodedColumnChunk::builder()
                    .set_dict_encoding(dict_enc.clone())
                    .build();
                accumulated_size = 0;
            }
        }
        if accumulated_size > 0 {
            res.push(accumulated_chunk);
        }
        Ok(res)
    }
}

impl PhysicalColEncoder for GLBestEncoder {
    fn encode(
        &mut self,
        array: ArrayRef,
        _counter: &mut EncodingCounter,
        _shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Vec<EncodedColumnChunk>> {
        if self
            .buffered_arrays
            .first()
            .is_some_and(|buf_arr| *buf_arr.data_type() != *array.data_type())
        {
            Err(fff_core::errors::Error::General(
                "Datatypes of arrays do not match".to_owned(),
            ))
        } else {
            self.buffered_array_len += array.len() as u64;
            self.buffered_array_mem_size += array.get_array_memory_size();
            self.buffered_arrays.push(array);
            Ok(vec![])
        }
    }

    fn memory_size(&self) -> usize {
        self.buffered_array_mem_size
    }

    fn finish(
        &mut self,
        counter: &mut EncodingCounter,
        shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Vec<EncodedColumnChunk>> {
        if self.buffered_arrays.is_empty() {
            Ok(vec![])
        } else {
            let buffered_arrs = std::mem::take(&mut self.buffered_arrays);
            let buffered_array_len = self.buffered_array_len;
            self.buffered_array_len = 0;
            self.buffered_array_mem_size = 0;

            let mut global_dict = Dictionary::try_new(buffered_arrs[0].data_type().clone())?;
            let global_indices_arrs = buffered_arrs
                .iter()
                .map(|arr| global_dict.extend_and_get_index(arr.clone()))
                .collect::<Result<Vec<_>>>()?;
            let global_dict_len = global_dict.len()?;
            let global_indices_arrs = global_indices_arrs
                .into_iter()
                .map(|arr| cast_index_dtype(arr, global_dict_len))
                .collect::<Vec<_>>();
            let mut encode_to_local_chunks =
                |arrs: &[ArrayRef]| -> std::result::Result<_, fff_core::errors::Error> {
                    let mut local_encoder = EncoderDictColEncoder::new(
                        self.column_chunk_size,
                        self.wasm_context.clone(),
                        true,
                        fb::CompressionType::Uncompressed,
                    );
                    let mut local_counter = EncodingCounter::default();
                    let local_chunks = arrs
                        .iter()
                        .map(|arr| Some(arr.clone()))
                        .chain(std::iter::once(None))
                        .map(|arr| -> Result<_, fff_core::errors::Error> {
                            if let Some(arr) = arr {
                                Ok(local_encoder
                                    .encode(arr, &mut local_counter, shared_dict_ctx)?
                                    .into_iter())
                            } else {
                                Ok(local_encoder
                                    .finish(&mut local_counter, shared_dict_ctx)?
                                    .into_iter())
                            }
                        })
                        .flatten_ok()
                        .collect::<Result<Vec<_>>>()?;
                    Ok((local_counter, local_chunks))
                };
            if let Some((sample_ratio, sample_len)) = self.sample_size {
                if (buffered_array_len as f64 * sample_ratio) as usize > sample_len {
                    let sample_count =
                        (buffered_array_len as f64 * sample_ratio) as usize / sample_len;
                    let sample_origs = (0..buffered_arrs.len())
                        .choose_multiple(&mut rand::thread_rng(), sample_count)
                        .iter()
                        .map(|i| buffered_arrs[*i].clone())
                        .collect::<Vec<_>>();
                    let (sample_local_counter, _) = encode_to_local_chunks(&sample_origs)?;
                    let local_est = (sample_local_counter.total_size()
                        * buffered_array_len as usize) as f64
                        / (sample_len * sample_count) as f64;
                    let global_index_est = self.estimate_arrays_encoded_size(
                        &global_indices_arrs,
                        buffered_array_len as usize,
                        sample_count,
                    )?;
                    let global_dict_est = self.estimate_arr_encoded_size(
                        global_dict.peek_dict()?,
                        sample_len,
                        sample_count,
                    )?;
                    if local_est <= global_index_est + global_dict_est {
                        // Use local
                        let (local_counter, local_chunks) = encode_to_local_chunks(&buffered_arrs)?;
                        *counter = local_counter;
                        return Ok(local_chunks);
                    } else {
                        // Use global
                        let global_dict_id = shared_dict_ctx.add_dictionary(global_dict);
                        return self.encode_shared_to_chunks(
                            &global_indices_arrs,
                            counter,
                            Some(global_dict_id),
                        );
                    }
                }
            }
            let (local_counter, local_chunks) = encode_to_local_chunks(&buffered_arrs)?;
            let mut global_counter = EncodingCounter::default();
            let peek_global_dict_id = shared_dict_ctx.peek_dict_id();
            let global_index_chunks = self.encode_shared_to_chunks(
                &global_indices_arrs,
                &mut global_counter,
                Some(peek_global_dict_id),
            )?;
            // TODO: avoid encoding the shared dict twice
            let _global_dict_chunks = self.encode_shared_to_chunks(
                &[global_dict.peek_dict()?],
                &mut global_counter,
                None,
            )?;
            if local_counter.total_size() <= global_counter.total_size() {
                *counter = local_counter;
                Ok(local_chunks)
            } else {
                global_counter.dict_size = 0; // defer it at global dictionary encoding time
                let global_dict_id = shared_dict_ctx.add_dictionary(global_dict);
                assert_eq!(global_dict_id, peek_global_dict_id);
                *counter = global_counter;
                Ok(global_index_chunks)
            }
        }
    }

    fn submit_dict(&mut self, _shared_dict_ctx: &mut SharedDictionaryContext) -> Result<()> {
        Ok(())
    }
}

// /// A specific encoder containing the encoding of both validity and offsets for a List Array.
// pub struct ListOffsetEncoder {
//     accumulated_chunk: EncodedColumnChunk,
//     accumulated_size: u64,
//     offsets_encoder: Box<dyn PhysicalColEncoder>,
// }

// impl ListOffsetEncoder {
//     pub fn try_new(
//         column_chunk_size: u64,
//         nullable: bool,
//         offset_sample: ArrayRef,
//     ) -> Result<Self> {
//         assert!(
//             offset_sample.data_type() == &DataType::Int32
//                 || offset_sample.data_type() == &DataType::Int64
//         );
//         Ok(Self {
//             accumulated_chunk: EncodedColumnChunk::default(),
//             accumulated_size: 0,
//             offsets_encoder: create_physical_encoder(
//                 Arc::clone(&offset_sample).data_type(),
//                 column_chunk_size,
//                 nullable,
//                 offset_sample,
//             )?,
//         })
//     }
// }

// impl PhysicalColEncoder for ListOffsetEncoder {
//     fn encode(&mut self, _array: ArrayRef) -> Result<Option<EncodedColumnChunk>> {
//         // TODO: separately encode offsets and validity.
//         todo!("ListOffsetEncoder::encode not implemented yet.")
//     }

//     fn finish(&mut self) -> Result<Option<EncodedColumnChunk>> {
//         // TODO: separately encode offsets and validity.
//         todo!("ListOffsetEncoder::encode not implemented yet.")
//     }
// }

// /// Should only be useful for Struct fields
// pub struct ValidityEncoder {
//     accumulated_chunk: EncodedColumnChunk,
//     accumulated_size: u64,
//     /// The desired encoded column chunk size, should match I/O unit size (e.g., 8MB on S3)
//     column_chunk_size: u64,
// }

// impl ValidityEncoder {
//     pub fn try_new(column_chunk_size: u64) -> Result<Self> {
//         Ok(Self {
//             accumulated_chunk: EncodedColumnChunk::default(),
//             accumulated_size: 0,
//             column_chunk_size,
//         })
//     }
// }

// impl PhysicalColEncoder for ValidityEncoder {
//     fn encode(
//     &mut self,
//     _array: ArrayRef,
//     _counter: &mut EncodingCounter,
//     _shared_dict_ctx: &mut SharedDictionaryContext,
// ) -> Result<Vec<EncodedColumnChunk>> {
//         // TODO: separately encode offsets and validity.
//         todo!("ValidityEncoder::encode not implemented yet.")
//     }

//     fn finish(
//         &mut self,
//         _counter: &mut EncodingCounter,
//         _shared_dict_ctx: &mut SharedDictionaryContext,
//     ) -> Result<Vec<EncodedColumnChunk>> {
//         // TODO: separately encode offsets and validity.
//         todo!("ValidityEncoder::encode not implemented yet.")
//     }

//     fn submit_dict(&mut self, _shared_dict_ctx: &mut SharedDictionaryContext) -> Result<()> {
//         Ok(())
//     }
// }

pub fn create_physical_encoder(
    data_type: &DataType,
    max_chunk_size: u64,
    _nullable: bool, // We use Vortex and its null info is embedded in the EncUnit.
    wasm_context: Arc<WASMWritingContext>,
    dictionary_type: DictionaryTypeOptions,
    compression_type: fb::CompressionType,
) -> Result<Box<dyn PhysicalColEncoder>> {
    match *data_type {
        non_nest_types!() => match dictionary_type {
            DictionaryTypeOptions::NoDictionary => Ok(Box::new(EncoderDictColEncoder::new(
                max_chunk_size,
                wasm_context,
                false,
                compression_type,
            ))),
            DictionaryTypeOptions::EncoderDictionary => Ok(Box::new(EncoderDictColEncoder::new(
                max_chunk_size,
                wasm_context,
                true,
                compression_type,
            ))),
            DictionaryTypeOptions::LocalDictionary => Ok(Box::new(DictColEncoder::new(
                max_chunk_size,
                wasm_context,
                compression_type,
            ))),
            DictionaryTypeOptions::GlobalDictionary
            | DictionaryTypeOptions::GlobalDictionaryMultiColSharing => Ok(Box::new(
                SharedDictColEncoder::new(u64::MAX, max_chunk_size, wasm_context, compression_type),
            )),
            DictionaryTypeOptions::FixedScopeDictionary(scope) => Ok(Box::new(
                SharedDictColEncoder::new(scope, max_chunk_size, wasm_context, compression_type),
            )),
            DictionaryTypeOptions::GLBest(sample_size) => Ok(Box::new(GLBestEncoder::new(
                sample_size,
                max_chunk_size,
                wasm_context,
                compression_type,
            ))),
        },
        DataType::List(_) | DataType::LargeList(_) => Ok(Box::new(EncoderDictColEncoder::new(
            max_chunk_size,
            wasm_context,
            true,
            compression_type,
        ))),
        _ => todo!("Other data types not supported"),
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        counter::EncodingCounter,
        dict::{shared_dictionary_context::SharedDictionaryContext, DictionaryTypeOptions},
    };

    #[test]
    fn test_plain() {
        use crate::context::WASMWritingContext;
        use fff_format::File::fff::flatbuf as fb;

        use super::PhysicalColEncoder;
        use crate::decoder::encunit::{EncUnitDecoder, VortexEncUnitDecoder};
        use crate::options::DEFAULT_IOUNIT_SIZE;
        use arrow_array::{Array, Int32Array};
        use std::sync::Arc;

        let mut encoder = super::EncoderDictColEncoder::new(
            DEFAULT_IOUNIT_SIZE,
            WASMWritingContext::empty().into(),
            true,
            fb::CompressionType::Uncompressed,
        );
        let a =
            Arc::new(arrow_array::Int32Array::from(vec![Some(1), None, Some(3)])) as Arc<dyn Array>;
        let mut counter = EncodingCounter::default();
        let mut shared_dict_ctx = SharedDictionaryContext::default();
        let _res = encoder
            .encode(a.clone(), &mut counter, &mut shared_dict_ctx)
            .unwrap();
        let res = encoder.finish(&mut counter, &mut shared_dict_ctx).unwrap();
        assert!(!res.is_empty());
        let res = res.first().unwrap();
        let encunit = res.encunits[0].clone();
        assert_eq!(
            encunit.encoding().encoding_type(),
            fb::EncodingType::CASCADE
        );
        let encunit_decoder = VortexEncUnitDecoder::new(encunit.bytes(), a.data_type().clone());
        let res_array = encunit_decoder.decode().unwrap();
        let res_array = res_array.as_any().downcast_ref::<Int32Array>().unwrap();
        assert_eq!(res_array.len(), 3);
        assert_eq!(res_array.value(0), 1);
        assert_eq!(res_array.value(2), 3);
        assert_eq!(counter.dict_type, DictionaryTypeOptions::EncoderDictionary);
    }

    #[test]
    fn test_dict() {
        use crate::context::WASMWritingContext;
        use fff_format::File::fff::flatbuf as fb;

        use super::PhysicalColEncoder;
        use crate::decoder::encunit::{EncUnitDecoder, VortexEncUnitDecoder};
        use crate::options::DEFAULT_IOUNIT_SIZE;
        use arrow_array::{Array, Int32Array, UInt8Array};
        use std::sync::Arc;

        let mut encoder = super::DictColEncoder::new(
            DEFAULT_IOUNIT_SIZE,
            WASMWritingContext::empty().into(),
            fb::CompressionType::Uncompressed,
        );
        let a = Arc::new(arrow_array::Int32Array::from(vec![
            Some(1),
            None,
            Some(3),
            Some(2),
            Some(3),
        ])) as Arc<dyn Array>;
        let mut counter = EncodingCounter::default();
        let mut shared_dict_ctx = SharedDictionaryContext::default();
        let _res = encoder
            .encode(a.clone(), &mut counter, &mut shared_dict_ctx)
            .unwrap();
        let res = encoder.finish(&mut counter, &mut shared_dict_ctx).unwrap();
        assert!(_res.is_empty());
        assert!(!res.is_empty());
        let res = res.first().unwrap();
        let dict_encunit = res.encunits[0].clone();
        assert_eq!(
            dict_encunit.encoding().encoding_type(),
            fb::EncodingType::CASCADE
        );
        let dict_encunit_decoder =
            VortexEncUnitDecoder::new(dict_encunit.bytes(), a.data_type().clone());
        let dict_array = dict_encunit_decoder.decode().unwrap();
        let dict_array = dict_array.as_any().downcast_ref::<Int32Array>().unwrap();
        assert_eq!(dict_array.len(), 3);
        assert_eq!(dict_array.value(0), 1);
        assert_eq!(dict_array.value(1), 3);
        assert_eq!(dict_array.value(2), 2);

        let index_encunit = res.encunits[1].clone();
        assert_eq!(
            index_encunit.encoding().encoding_type(),
            fb::EncodingType::CASCADE
        );
        let index_encunit_decoder =
            VortexEncUnitDecoder::new(index_encunit.bytes(), arrow_schema::DataType::Int64);
        let index_array = index_encunit_decoder.decode().unwrap();
        let index_array = index_array.as_any().downcast_ref::<UInt8Array>().unwrap();
        assert_eq!(index_array.len(), 5);
        assert_eq!(index_array.value(0), 0);
        assert_eq!(index_array.value(2), 1);
        assert_eq!(index_array.value(3), 2);
        assert_eq!(index_array.value(4), 1);
        assert_eq!(counter.dict_type, DictionaryTypeOptions::LocalDictionary);
    }
}
