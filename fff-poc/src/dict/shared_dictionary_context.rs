use std::{collections::HashMap, sync::Arc};

use arrow_array::ArrayRef;
use arrow_schema::DataType;
use fff_core::errors::Error;
use fff_encoding::schemes::encode_to_bytes;
use fff_format::File::fff::flatbuf as fb;

use crate::{
    context::WASMWritingContext,
    counter::EncodingCounter,
    encoder::{
        encoded_column_chunk::{EncodedColumnChunk, SerializedEncUnit},
        encunit::create_encunit_encoder,
    },
    file::footer::{self, WASMEncoding},
    options::{DEFAULT_ENCODING_UNIT_LEN, DEFAULT_IOUNIT_SIZE},
};

use super::{bottom_k_sketch::BottomKSketch, Dictionary};

// Threshold of Jaccard similarity for merging two dicts
const MERGE_THRESHOLD: f64 = 0.01;
const OVERLAP_THRESHOLD: f64 = 0.99;
const INTERSECTION_LEN_THRESHOLD: f64 = 1024.0;

/// This struct manages shared dictionaries for writer
pub struct SharedDictionaryContext {
    dictionaries: Vec<Dictionary>,
    _encoding_unit_size: u64,
    _column_chunk_size: u64,
    is_multi_col_sharing: bool,
    merge_result: Vec<Option<(usize, usize)>>,
    compression_type: fb::CompressionType,
}

impl Default for SharedDictionaryContext {
    fn default() -> Self {
        Self {
            dictionaries: vec![],
            _encoding_unit_size: DEFAULT_ENCODING_UNIT_LEN,
            _column_chunk_size: DEFAULT_IOUNIT_SIZE,
            is_multi_col_sharing: false,
            merge_result: vec![],
            compression_type: fb::CompressionType::Uncompressed,
        }
    }
}

impl SharedDictionaryContext {
    pub fn new(
        encoding_unit_size: u64,
        column_chunk_size: u64,
        is_multi_col_sharing: bool,
        compression_type: fb::CompressionType,
    ) -> Self {
        Self {
            dictionaries: vec![],
            _encoding_unit_size: encoding_unit_size,
            _column_chunk_size: column_chunk_size,
            is_multi_col_sharing,
            merge_result: vec![],
            compression_type,
        }
    }

    pub fn new_dictionary(&mut self, dtype: DataType) -> Result<u32, Error> {
        let dictionary = Dictionary::try_new(dtype)?;
        self.dictionaries.push(dictionary);
        Ok(self.dictionaries.len() as u32 - 1)
    }

    // TODO: this may cause TOCTOU race in multithreading scenario,
    // if the dictionary ID is used for index chunks before the dict is added
    pub fn peek_dict_id(&self) -> u32 {
        self.dictionaries.len() as u32
    }

    pub fn add_dictionary(&mut self, dictionary: Dictionary) -> u32 {
        self.dictionaries.push(dictionary);
        self.dictionaries.len() as u32 - 1
    }

    pub fn extend_and_get_index(
        &mut self,
        dict_idx: u32,
        arr: ArrayRef,
    ) -> Result<ArrayRef, Error> {
        let dictionary = &mut self.dictionaries[dict_idx as usize];
        dictionary.extend_and_get_index(arr)
    }

    pub fn merge_dicts(&mut self) -> Result<(), Error> {
        // Do not change the IDs of dicts, but change the chunks
        // Use hash functions to determine whether two dictionaries are "similar"
        // And merge similar ones
        let dicts = &mut self.dictionaries;
        let mut dtype_to_sketches = HashMap::<DataType, Vec<(usize, BottomKSketch)>>::new();
        for (idx, dict) in dicts.iter().enumerate() {
            if dict.len()? == 0 {
                continue;
            }
            let mut sketch = BottomKSketch::new();
            dict.dict_hash_iter()?.for_each(|val| sketch.add_hash(val));
            sketch.finish();
            let dtype = &dict.datatype;
            if dtype_to_sketches.contains_key(dtype) {
                dtype_to_sketches
                    .get_mut(dtype)
                    .unwrap()
                    .push((idx, sketch));
            } else {
                dtype_to_sketches.insert(dtype.clone(), vec![(idx, sketch)]);
            }
        }
        let merge_res = &mut self.merge_result;
        merge_res.resize(dicts.len(), None);
        for sketch_group in dtype_to_sketches.values() {
            if sketch_group.len() < 2 {
                continue;
            }
            let mut similarity_edges = vec![];
            for i in 1..sketch_group.len() {
                for j in 0..i {
                    similarity_edges.push((
                        sketch_group[i].1.estimate_jaccard(&sketch_group[j].1),
                        sketch_group[i].0,
                        sketch_group[j].0,
                    ))
                }
            }
            similarity_edges.sort_unstable_by(|x, y| {
                x.0.partial_cmp(&y.0)
                    .unwrap()
                    .reverse()
                    .then(x.1.cmp(&y.1))
                    .then(x.2.cmp(&y.2))
            });
            for similarity_edge in similarity_edges {
                if similarity_edge.0 < MERGE_THRESHOLD {
                    break;
                }
                let dict_i = similarity_edge.1.min(similarity_edge.2);
                let dict_j = similarity_edge.1.max(similarity_edge.2);
                assert!(dict_i < dict_j);
                if merge_res[dict_i].is_some() || merge_res[dict_j].is_some() {
                    continue;
                }
                let est_intersection_len = (dicts[dict_i].len()? + dicts[dict_j].len()?) as f64
                    * similarity_edge.0
                    / (similarity_edge.0 + 1f64);
                if est_intersection_len < INTERSECTION_LEN_THRESHOLD
                    && similarity_edge.0 < OVERLAP_THRESHOLD
                {
                    continue;
                }
                let (mut_i_slice, mut_j_slice) = dicts.split_at_mut(dict_j);
                let merge_len = mut_i_slice[dict_i].merge_with(&mut mut_j_slice[0])?;
                if merge_len > 0 {
                    merge_res[dict_i] = Some((dict_j, merge_len));
                    merge_res[dict_j] = Some((dict_i, merge_len));
                }
            }
        }
        Ok(())
    }

    pub fn submit_values(&mut self, dict_idx: u32, values: ArrayRef) -> Result<(), Error> {
        self.dictionaries[dict_idx as usize].submit_values(values)
    }

    #[allow(clippy::type_complexity)]
    pub fn finish_and_flush(
        &mut self,
        wasm_context: Arc<WASMWritingContext>,
        _counters: &mut [EncodingCounter], // TODO: the dict size in counter cannot be updated here
    ) -> Result<
        (
            Vec<Vec<EncodedColumnChunk>>,
            Vec<Option<usize>>,
            Vec<DataType>,
        ),
        Error,
    > {
        let dict = std::mem::take(&mut self.dictionaries);
        let dict_dtypes = dict
            .iter()
            .map(|dict| dict.datatype.clone())
            .collect::<Vec<_>>();
        let (chunks, merge_peer) = dict
            .into_iter()
            .enumerate()
            .map(
                |(i, dict)| -> Result<(Vec<EncodedColumnChunk>, Option<usize>), Error> {
                    let (mut dict, _) = dict.finish()?;
                    let dict_dtype = dict.data_type().clone();
                    let mut dict_chunk = EncodedColumnChunk::builder()
                        .set_dict_encoding(footer::DictionaryEncoding::NoDictionary)
                        .build();
                    let mut opt_peer = None;
                    let mut dict_len = dict.len();
                    let mut ret_chunks = vec![];
                    let dict_encoder = create_encunit_encoder(
                        wasm_context.clone(),
                        dict.data_type().clone(),
                        false,
                    );
                    let write_slice = |slice, slice_len| -> Result<SerializedEncUnit, Error> {
                        let encoded_bytes = encode_to_bytes(dict_encoder.clone(), slice);
                        Ok(SerializedEncUnit::new(
                            encoded_bytes,
                            slice_len as u32,
                            {
                                let encoding_type = dict_encoder.encoding_type();
                                footer::Encoding::try_new(
                                    if wasm_context.always_set_custom_wasm_for_built_in() {
                                        fb::EncodingType::CUSTOM_WASM
                                    } else {
                                        encoding_type.to_fbs_encoding()
                                    },
                                    if wasm_context.always_set_custom_wasm_for_built_in() {
                                        wasm_context.builtin_wasm_id()
                                    } else {
                                        wasm_context.data_type_to_wasm_id(&dict_dtype)
                                    }
                                    .map(|id| WASMEncoding::new(id.0, Vec::new())),
                                )?
                            },
                            self.compression_type,
                        ))
                    };
                    if let Some(Some((peer, merge_len))) = self.merge_result.get(i) {
                        if i < *peer {
                            // Write common part at i
                            let mut common_dict_chunk = EncodedColumnChunk::builder()
                                .set_dict_encoding(footer::DictionaryEncoding::NoDictionary)
                                .build();
                            common_dict_chunk
                                .encunits
                                .push(write_slice(dict.slice(0, *merge_len), *merge_len)?);
                            common_dict_chunk.num_rows = *merge_len;
                            ret_chunks.push(common_dict_chunk);
                        } else {
                            // Reference the common part at peer
                            opt_peer = Some(*peer);
                        }
                        dict_len -= merge_len;
                        if dict_len == 0 {
                            return Ok((ret_chunks, opt_peer));
                        }
                        dict = dict.slice(*merge_len, dict_len);
                    }
                    dict_chunk.encunits.push(write_slice(dict, dict_len)?);
                    dict_chunk.num_rows = dict_len;
                    ret_chunks.push(dict_chunk);
                    Ok((ret_chunks, opt_peer))
                },
            )
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .unzip();
        Ok((chunks, merge_peer, dict_dtypes))
    }

    pub fn dict_len(&self, dict_idx: u32) -> Result<usize, Error> {
        self.dictionaries[dict_idx as usize].len()
    }

    pub fn is_multi_col_sharing(&self) -> bool {
        self.is_multi_col_sharing
    }
}
