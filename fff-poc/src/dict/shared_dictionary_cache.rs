use std::sync::Arc;

use arrow::compute::concat;
use arrow_array::ArrayRef;
use arrow_ipc::{convert::fb_to_schema, root_as_message};
use bytes::BytesMut;
use fff_core::{errors::Error, nyi_err};
use fff_format::File::fff::flatbuf as fb;

use crate::{
    context::WASMReadingContext, decoder::physical::create_physical_decoder, io::reader::Reader,
};

pub struct SharedDictionaryCache {
    // shared_dictionary_table: fb::SharedDictionaryTable<'a>,
    dictionaries: Vec<Option<ArrayRef>>,
    dictionary_compressed_sizes: Vec<usize>,
    dictionary_chunk_sizes: Vec<usize>,
    dictionary_chunk_references: Vec<Vec<usize>>,
}

impl SharedDictionaryCache {
    pub fn try_new_read_all<R: Reader>(
        reader: R,
        shared_dictionary_table: fb::SharedDictionaryTable,
        wasm_context: Option<Arc<WASMReadingContext<R>>>,
    ) -> Result<Self, Error> {
        // TODO: remove these unwrap
        let positions = shared_dictionary_table
            .dictionary_positions()
            .unwrap()
            .iter()
            .map(|v| {
                v.chunk_ids()
                    .unwrap()
                    .iter()
                    .map(|x| x as usize)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let chunks = shared_dictionary_table.dictionary_chunks().unwrap();
        let dictionary_chunk_sizes = chunks
            .iter()
            .map(|chunk_meta| chunk_meta.size_() as usize)
            .collect::<Vec<_>>();
        let dict_schema = shared_dictionary_table
            .dictionary_schema()
            .ok_or_else(|| Error::ParseError("Shared dictionary schema not found".to_string()))?;
        let message = root_as_message(dict_schema.bytes())
            .map_err(|err| Error::ParseError(format!("Unable to get root as message: {err:?}")))?;
        let ipc_schema = message
            .header_as_schema()
            .ok_or_else(|| Error::ParseError("Unable to read IPC message as schema".to_string()))?;
        let dict_schema = fb_to_schema(ipc_schema);
        let mut dict_sizes = vec![]; // TODO: do not use mutable vec to modify
        let dictionaries = positions
            .iter()
            .enumerate()
            .map(|(i, chunk_ids)| -> Result<Option<ArrayRef>, Error> {
                let datatype = dict_schema.field(i).data_type();
                let mut dict_size = 0;
                let dict_arrs = chunk_ids
                    .iter()
                    .map(|chunk_id| {
                        let chunk_meta = chunks.get(*chunk_id);
                        dict_size += chunk_meta.size_() as usize;
                        let mut encoded_chunk_buf = BytesMut::zeroed(chunk_meta.size_() as usize);
                        reader.read_exact_at(&mut encoded_chunk_buf, chunk_meta.offset())?;
                        let mut decoder = create_physical_decoder::<R>(
                            chunk_meta
                                .encunits()
                                .ok_or_else(|| {
                                    Error::General("No chunks in column meta".to_string())
                                })?
                                .iter(),
                            chunk_meta.encoding_type(),
                            None,
                            datatype,
                            encoded_chunk_buf,
                            wasm_context
                                .as_ref()
                                .map(Arc::clone),
                            None,
                        )?;
                        let mut arrays = vec![];
                        if chunk_meta.num_rows() == 0 {
                            arrays.push(Arc::new(arrow_array::Int32Array::new_null(1)) as ArrayRef);
                        } else {
                            while let Some(array) = decoder.decode_batch()? {
                                arrays.push(array);
                            }
                        }
                        if arrays.len() != 1 {
                            nyi_err!(
                            "Now we only handle the case where each dictionary chunk has a single EncUnit"
                        )
                        } else {
                            Ok(arrays[0].clone())
                        }
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                dict_sizes.push(dict_size);
                if dict_arrs.len() == 1 {
                    Ok(Some(dict_arrs[0].clone()))
                } else if dict_arrs.len() == 2 {
                    assert_eq!(dict_arrs[0].data_type(), dict_arrs[1].data_type());
                    Ok(Some(concat(&[&dict_arrs[0], &dict_arrs[1]])?))
                } else {
                    Err(Error::General("Now we only handle the case where each dictionary has <=2 chunks".to_owned()))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            dictionaries,
            dictionary_compressed_sizes: dict_sizes,
            dictionary_chunk_sizes,
            dictionary_chunk_references: positions,
        })
    }

    pub fn get_dict(&self, index: usize) -> Option<ArrayRef> {
        self.dictionaries.get(index).cloned().flatten()
    }

    pub fn get_dict_size(&self, index: usize) -> Option<usize> {
        self.dictionary_compressed_sizes.get(index).cloned()
    }

    pub fn get_dict_chunk_sizes(&self) -> &[usize] {
        &self.dictionary_chunk_sizes
    }

    pub fn get_dict_references(&self) -> &Vec<Vec<usize>> {
        &self.dictionary_chunk_references
    }
}
