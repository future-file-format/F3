use std::sync::Arc;

use crate::common::checksum::{create_checksum, ChecksumType};
use crate::dict::shared_dictionary_cache::SharedDictionaryCache;
use crate::io::reader::Reader;
use crate::{common::ColumnIndexSequence, context::WASMReadingContext};
use arrow::array::AsArray;
use arrow_array::{Array, ArrayRef, LargeListArray, ListArray, StructArray};
use arrow_buffer::{NullBuffer, OffsetBuffer, OffsetBufferBuilder, ScalarBuffer};
use arrow_schema::{DataType, Field, FieldRef, Fields};
use bytes::BytesMut;
use fff_core::{
    errors::{Error, Result},
    general_error,
};
use fff_format::File::fff::flatbuf as fb;
use flatbuffers::{ForwardsUOffset, VectorIter};

use super::physical::{create_physical_decoder, ChunkDecoder};
use fff_core::non_nest_types;

/// This maps to each logical column in the top level Arrow schema stored in file footer.
pub trait LogicalColDecoder {
    /// Decode all the data of the column in current row group.
    /// Each page's data corresponds to one `ArrayRef`
    fn decode_batch(&mut self) -> Result<Vec<ArrayRef>>;
    /// Decode some rows out starting at row_id.
    fn decode_row_at(&mut self, row_id: usize, len: usize) -> Result<Vec<ArrayRef>>;
}

/// A specific trait for testing select+proj performance of different nested implementation.
pub trait LogicalListStructNonNestedColDecoder {
    fn decode_batch_at_with_proj(
        &mut self,
        project_idx: usize,
        row_id: usize,
        len: usize,
    ) -> Result<Vec<ArrayRef>>;
}

/// Decoder for a single physical column inside a file
/// Currently used by both pritimive types and list's (validity + offsets)
/// lifetime here is because data_encoder stores iter of FlatBuf which has lifetime 'a
pub struct PrimitiveColDecoder<'a, R> {
    r: &'a R,
    chunk_decoder: Option<Box<dyn ChunkDecoder + 'a>>,
    chunks_meta_iter: VectorIter<'a, ForwardsUOffset<fb::Chunk<'a>>>,
    primitive_type: DataType,
    wasm_context: Option<Arc<WASMReadingContext<R>>>,
    shared_dictionary_cache: &'a SharedDictionaryCache,
    /// if checksum is not None, we will verify the checksum of the chunk
    checksum_type: Option<ChecksumType>,
}

impl<R: Reader> PrimitiveColDecoder<'_, R> {
    /// Read a chunk from the reader
    /// IO and compute are sequential in this case. Separation is left for future work.
    fn read_chunk(&mut self, offset: u64, size: u32, checksum: Option<u64>) -> Result<BytesMut> {
        let mut buf = BytesMut::zeroed(size as usize);
        self.r.read_exact_at(&mut buf, offset)?;
        if let Some(checksum_type) = &mut self.checksum_type {
            let checksum = checksum.ok_or_else(|| {
                general_error!(format!(
                    "No checksum in column meta for chunk at offset {}",
                    offset
                ))
            })?;
            let computed_checksum = {
                let mut checksum = create_checksum(checksum_type);
                checksum.update(&buf);
                checksum.finalize()
            };
            if checksum != computed_checksum {
                return Err(Error::General("Checksum verification failed".to_string()));
            }
        }
        Ok(buf)
    }
}

impl<R: Reader> LogicalColDecoder for PrimitiveColDecoder<'_, R> {
    fn decode_batch(&mut self) -> Result<Vec<ArrayRef>> {
        let mut arrays = vec![];
        while let Some(chunk_meta) = self.chunks_meta_iter.next() {
            let encoded_chunk_buf = self.read_chunk(
                chunk_meta.offset(),
                chunk_meta.size_(),
                chunk_meta.checksum(),
            )?;
            self.chunk_decoder = Some(create_physical_decoder::<R>(
                chunk_meta
                    .encunits()
                    .ok_or_else(|| general_error!("No chunks in column meta"))?
                    .iter(),
                chunk_meta.encoding_type(),
                chunk_meta.encoding_as_shared_dictionary(),
                &self.primitive_type,
                encoded_chunk_buf,
                self.wasm_context.as_ref().map(Arc::clone),
                Some(self.shared_dictionary_cache),
            )?);
            while let Some(array) = self.chunk_decoder.as_mut().unwrap().decode_batch()? {
                arrays.push(array);
            }
        }
        Ok(arrays)
    }
    fn decode_row_at(&mut self, row_id: usize, len: usize) -> Result<Vec<ArrayRef>> {
        let mut arrays = vec![];
        let mut cur_row = 0; // FIXME: Not correct if we have muliple row groups
        let mut remaining = len;
        while let Some(chunk_meta) = self.chunks_meta_iter.next() {
            if remaining == 0 {
                break;
            }
            if cur_row + chunk_meta.num_rows() as usize <= row_id {
                cur_row += chunk_meta.num_rows() as usize;
                continue;
            }
            let mut to_decode = std::cmp::min(
                chunk_meta.num_rows() as usize - (row_id - cur_row),
                remaining,
            );

            let encoded_chunk_buf = self.read_chunk(
                chunk_meta.offset(),
                chunk_meta.size_(),
                chunk_meta.checksum(),
            )?;
            // println!(
            //     "read chunk at offset {} with size {}",
            //     chunk_meta.offset(),
            //     chunk_meta.size_()
            // );
            self.chunk_decoder = Some(create_physical_decoder::<R>(
                chunk_meta
                    .encunits()
                    .ok_or_else(|| general_error!("No chunks in column meta"))?
                    .iter(),
                chunk_meta.encoding_type(),
                chunk_meta.encoding_as_shared_dictionary(),
                &self.primitive_type,
                encoded_chunk_buf,
                self.wasm_context.as_ref().map(Arc::clone),
                Some(self.shared_dictionary_cache),
            )?);
            let mut decoded = 0;
            while let Some(array) = self
                .chunk_decoder
                .as_mut()
                .unwrap()
                .decode_row_at(row_id - cur_row, to_decode)?
            {
                to_decode -= array.len();
                decoded += array.len();
                arrays.push(array);
                if to_decode == 0 {
                    break;
                }
            }
            remaining -= decoded;
        }
        Ok(arrays)
    }
}

/// Decoder for List column
/// validity_offsets_decoder will output a List Array but only validity and offsetes are useful.
/// values_decoder is recursively decided by the child field of the List field.
pub struct ListColDecoder<'a, R> {
    field: FieldRef,
    validity_offsets_decoder: PrimitiveColDecoder<'a, R>,
    values_decoder: Box<dyn LogicalColDecoder + 'a>,
}

impl<R: Reader> LogicalColDecoder for ListColDecoder<'_, R> {
    fn decode_batch(&mut self) -> Result<Vec<ArrayRef>> {
        let mut res = vec![];
        let validity_offsets = self.validity_offsets_decoder.decode_batch()?;
        let values = self.values_decoder.decode_batch()?;
        for (v_o, val) in validity_offsets.into_iter().zip(values.into_iter()) {
            match self.field.data_type() {
                DataType::List(child) => {
                    let arr = v_o.as_list::<i32>();
                    let offsets: ScalarBuffer<i32> = arr.to_data().buffers()[0].clone().into();
                    let offsets = OffsetBuffer::new(offsets);
                    let nulls = v_o.as_list::<i32>().nulls().cloned();
                    res.push(Arc::new(ListArray::new(
                        // always return byte view array because of the change of underlining vortex.
                        field_to_view(child.clone()),
                        offsets,
                        val,
                        nulls,
                    )) as Arc<dyn Array>);
                }
                DataType::LargeList(child) => {
                    let offsets: ScalarBuffer<i64> =
                        v_o.as_list::<i64>().to_data().buffers()[0].clone().into();
                    let offsets = OffsetBuffer::new(offsets);
                    let nulls = v_o.as_list::<i64>().nulls().cloned();
                    res.push(Arc::new(LargeListArray::new(
                        field_to_view(child.clone()),
                        offsets,
                        val,
                        nulls,
                    )) as Arc<dyn Array>);
                }
                _ => unreachable!(),
            }
        }
        Ok(res)
    }

    fn decode_row_at(&mut self, _row_id: usize, _len: usize) -> Result<Vec<ArrayRef>> {
        todo!()
    }
}

/// A custom experimental ListStruct(non_nest) decoder with Offsets pushdown for List.
/// Will only be enabled with feature = "list-offsets-pushdown"
pub struct OffsetPushdownListStructColDecoder<'a, R> {
    children: Vec<PrimitiveColDecoder<'a, R>>,
}

impl<R: Reader> LogicalListStructNonNestedColDecoder for OffsetPushdownListStructColDecoder<'_, R> {
    fn decode_batch_at_with_proj(
        &mut self,
        project_idx: usize,
        row_id: usize,
        len: usize,
    ) -> Result<Vec<ArrayRef>> {
        let res = self.children[project_idx].decode_row_at(row_id, len)?;
        Ok(res)
    }
}

/// A custom experimental Struct(non_nest) decoder *without* Offsets pushdown for List.
pub struct StructOfNonNestColDecoder<'a, R> {
    fields: Fields,
    struct_validity_decoder: PrimitiveColDecoder<'a, R>,
    children: Vec<PrimitiveColDecoder<'a, R>>,
}

impl<R: Reader> LogicalListStructNonNestedColDecoder for StructOfNonNestColDecoder<'_, R> {
    fn decode_batch_at_with_proj(
        &mut self,
        project_idx: usize,
        row_id: usize,
        len: usize,
    ) -> Result<Vec<ArrayRef>> {
        let mut res = vec![];
        let validity = self.struct_validity_decoder.decode_row_at(row_id, len)?;
        let values = self.children[project_idx].decode_row_at(row_id, len)?;
        for (v, val) in validity.into_iter().zip(values.into_iter()) {
            let bool_array = v.as_boolean();
            let nulls =
                (!bool_array.is_empty()).then(|| NullBuffer::new(bool_array.values().clone()));
            res.push(Arc::new(StructArray::new(
                [field_to_view(self.fields[project_idx].clone())].into(),
                vec![val],
                nulls,
            )) as Arc<dyn Array>);
        }
        Ok(res)
    }
}

/// A custom experimental ListStruct(non_nest) decoder *without* Offsets pushdown for List.
/// CAUTION: its API may only work in a few test cases to justify the experiments
pub struct ListStructColDecoder<'a, R> {
    field: FieldRef,
    validity_offsets_decoder: PrimitiveColDecoder<'a, R>,
    children: StructOfNonNestColDecoder<'a, R>,
}

impl<R: Reader> LogicalListStructNonNestedColDecoder for ListStructColDecoder<'_, R> {
    fn decode_batch_at_with_proj(
        &mut self,
        project_idx: usize,
        row_id: usize,
        len: usize,
    ) -> Result<Vec<ArrayRef>> {
        let mut res = vec![];
        // Because children's cardinality is larget than the List's v_o, we need to prefix sum the
        // offsets to get the correct row id for children.
        // let mut value_row_id = 0;
        // let mut prefix_sum = 0;
        // while row_id > prefix_sum {
        //     let offsets: OffsetBuffer<i32> = OffsetBuffer::new(
        //         self.validity_offsets_decoder.decode_batch()?[0]
        //             .as_list::<i32>()
        //             .to_data()
        //             .buffers()[0]
        //             .clone()
        //             .into(),
        //     );
        //     prefix_sum += offsets[offsets.len() - 1] as usize;
        // }
        let v_o = self.validity_offsets_decoder.decode_row_at(row_id, len)?;
        // TODO: concat is not efficient
        let v_o = arrow::compute::concat(
            v_o.iter()
                .map(|a| a.as_ref())
                .collect::<Vec<_>>()
                .as_slice(),
        )?;
        match self.field.data_type() {
            DataType::List(child) => {
                match child.data_type() {
                    DataType::Struct(struct_fields) => {
                        let arr = v_o.as_list::<i32>();
                        let offsets: ScalarBuffer<i32> = arr.to_data().buffers()[0].clone().into();
                        let offsets: OffsetBuffer<i32> = OffsetBuffer::new(offsets);
                        // We only decode part of the child Array out, so we need to slice the offsets
                        let mut builder = OffsetBufferBuilder::<i32>::new(len);
                        for i in 0..len {
                            builder.push_length((offsets[i + 1] - offsets[i]) as usize);
                        }
                        let offsets_sliced = builder.finish();
                        // Children are sliced.
                        let val = self.children.decode_batch_at_with_proj(
                            project_idx,
                            offsets[0] as usize,
                            (offsets[len] - offsets[0]) as usize,
                        )?;
                        let nulls = v_o.as_list::<i32>().nulls().cloned();
                        res.push(Arc::new(ListArray::new(
                            Field::new_struct(
                                child.name(),
                                vec![field_to_view(struct_fields[project_idx].clone())],
                                child.is_nullable(),
                            )
                            .into(),
                            offsets_sliced,
                            arrow::compute::concat(
                                val.iter()
                                    .map(|a| a.as_ref())
                                    .collect::<Vec<_>>()
                                    .as_slice(),
                            )?,
                            nulls,
                        )) as Arc<dyn Array>);
                    }
                    _ => unreachable!("wrong type in ListStructColDecoder"),
                }
            }
            DataType::LargeList(_child) => {
                todo!();
                // let offsets: ScalarBuffer<i64> =
                //     v_o.as_list::<i64>().to_data().buffers()[0].clone().into();
                // let offsets = OffsetBuffer::new(offsets);
                // let nulls = v_o.as_list::<i64>().nulls().map(|x| x.clone());
                // res.push(
                //     Arc::new(LargeListArray::new(Arc::clone(&child), offsets, val, nulls))
                //         as Arc<dyn Array>,
                // );
            }
            _ => unreachable!(),
        }
        Ok(res)
    }
}

/// Decoder for Struct column
pub struct StructColDecoder<'a, R> {
    fields: Fields,
    validity_decoder: PrimitiveColDecoder<'a, R>,
    children: Vec<Box<dyn LogicalColDecoder + 'a>>,
}

impl<R: Reader> LogicalColDecoder for StructColDecoder<'_, R> {
    fn decode_batch(&mut self) -> Result<Vec<ArrayRef>> {
        let validity = self.validity_decoder.decode_batch()?;
        let children: Vec<Vec<ArrayRef>> = self
            .children
            .iter_mut()
            .map(|c| c.decode_batch())
            .collect::<Result<Vec<_>>>()?;
        fn transpose<T>(v: Vec<Vec<T>>) -> Vec<Vec<T>> {
            assert!(!v.is_empty());
            let len = v[0].len();
            let mut iters: Vec<_> = v.into_iter().map(|n| n.into_iter()).collect();
            (0..len)
                .map(|_| {
                    iters
                        .iter_mut()
                        .map(|n| n.next().unwrap())
                        .collect::<Vec<T>>()
                })
                .collect()
        }
        let children = transpose(children);

        let res: Vec<ArrayRef> = validity
            .into_iter()
            .zip(children)
            .map(|(v, cs)| {
                // recover NullBuffer from BooleanArray
                let bool_array = v.as_boolean();
                let nulls =
                    (!bool_array.is_empty()).then(|| NullBuffer::new(bool_array.values().clone()));
                let res = Arc::new(StructArray::new(
                    self.fields
                        .iter()
                        .map(|f| {
                            // always return byte view array because of the change of underlining vortex.
                            field_to_view(f.clone())
                        })
                        .collect(),
                    cs,
                    nulls,
                )) as ArrayRef;
                res
            })
            .collect();
        Ok(res)
    }

    fn decode_row_at(&mut self, _row_id: usize, _len: usize) -> Result<Vec<ArrayRef>> {
        todo!()
    }
}

/// Create a LogicalListStructNonNestedColDecoder
/// Whether it is OffsetPushdown or not depends on the feature flag "list-offsets-pushdown"
pub fn create_list_struct_decoder<'a, R: Reader>(
    r: &'a R,
    field: FieldRef,
    column_metas: &Vec<fb::ColumnMetadata<'a>>,
    column_idx: &mut ColumnIndexSequence,
    wasm_context: Option<Arc<WASMReadingContext<R>>>,
    shared_dictionary_cache: &'a SharedDictionaryCache,
) -> Result<Box<dyn LogicalListStructNonNestedColDecoder + 'a>> {
    let mut column_index = column_idx.next_column_index();
    let mut column_meta = column_metas.get(column_index as usize).unwrap();
    let mut chunks_meta_iter = column_meta
        .column_chunks()
        .ok_or_else(|| Error::General("No chunks in column meta".to_string()))?
        .iter();
    match field.data_type() {
        DataType::List(child) | DataType::LargeList(child) => match child.data_type() {
            DataType::Struct(fields)
                if fields
                    .iter()
                    .all(|f| matches!(f.data_type(), non_nest_types!())) =>
            {
                if cfg!(feature = "list-offsets-pushdown") {
                    // create offset-pushdown list struct decoder
                    let mut children = vec![];
                    let mut i = 0;
                    if i < fields.len() {
                        loop {
                            // println!("create decoder for index {}", column_index);
                            children.push(PrimitiveColDecoder {
                                r,
                                chunk_decoder: None,
                                chunks_meta_iter,
                                primitive_type: {
                                    match field.data_type() {
                                        DataType::List(child) => DataType::List(
                                            Field::new_struct(
                                                child.name(),
                                                vec![fields[i].clone()],
                                                child.is_nullable(),
                                            )
                                            .into(),
                                        ),
                                        DataType::LargeList(child) => DataType::LargeList(
                                            Field::new_struct(
                                                child.name(),
                                                vec![fields[i].clone()],
                                                child.is_nullable(),
                                            )
                                            .into(),
                                        ),
                                        _ => unreachable!(),
                                    }
                                },
                                wasm_context: wasm_context.as_ref().map(Arc::clone),
                                shared_dictionary_cache,
                                checksum_type: None,
                            });
                            i += 1;
                            if i == fields.len() {
                                break;
                            }
                            column_index = column_idx.next_column_index();
                            column_meta = column_metas.get(column_index as usize).unwrap();
                            chunks_meta_iter = column_meta
                                .column_chunks()
                                .ok_or_else(|| {
                                    Error::General("No chunks in column meta".to_string())
                                })?
                                .iter();
                        }
                    }
                    Ok(Box::new(OffsetPushdownListStructColDecoder { children }))
                } else {
                    // create ordinary list struct decoder
                    Ok(Box::new(ListStructColDecoder {
                        field: Arc::clone(&field),
                        validity_offsets_decoder: PrimitiveColDecoder {
                            r,
                            chunk_decoder: None,
                            chunks_meta_iter,
                            primitive_type: field.data_type().clone(),
                            wasm_context: wasm_context.as_ref().map(Arc::clone),
                            shared_dictionary_cache,
                            checksum_type: None,
                        },
                        children: StructOfNonNestColDecoder {
                            fields: fields.clone(),
                            struct_validity_decoder: PrimitiveColDecoder {
                                r,
                                chunk_decoder: None,
                                chunks_meta_iter: {
                                    column_index = column_idx.next_column_index();
                                    column_meta = column_metas.get(column_index as usize).unwrap();
                                    let chunks_meta_iter = column_meta
                                        .column_chunks()
                                        .ok_or_else(|| {
                                            Error::General("No chunks in column meta".to_string())
                                        })?
                                        .iter();
                                    chunks_meta_iter
                                },
                                primitive_type: DataType::Boolean,
                                wasm_context: wasm_context.as_ref().map(Arc::clone),
                                shared_dictionary_cache,
                                checksum_type: None,
                            },
                            children: fields
                                .iter()
                                .map(|f| PrimitiveColDecoder {
                                    r,
                                    chunk_decoder: None,
                                    chunks_meta_iter: {
                                        column_index = column_idx.next_column_index();
                                        column_meta =
                                            column_metas.get(column_index as usize).unwrap();
                                        let chunks_meta_iter = column_meta
                                            .column_chunks()
                                            .ok_or_else(|| {
                                                Error::General(
                                                    "No chunks in column meta".to_string(),
                                                )
                                            })
                                            .unwrap()
                                            .iter();
                                        chunks_meta_iter
                                    },
                                    primitive_type: f.data_type().clone(),
                                    wasm_context: wasm_context.as_ref().map(Arc::clone),
                                    shared_dictionary_cache,
                                    checksum_type: None,
                                })
                                .collect(),
                        },
                    }))
                }
            }
            _ => panic!("wrong type in create_list_struct_decoder"),
        },
        _ => panic!("wrong type in create_list_struct_decoder"),
    }
}

pub fn create_logical_decoder<'a, R: Reader>(
    r: &'a R,
    field: FieldRef,
    column_metas: &Vec<fb::ColumnMetadata<'a>>,
    column_idx: &mut ColumnIndexSequence,
    wasm_context: Option<Arc<WASMReadingContext<R>>>,
    shared_dictionary_cache: &'a SharedDictionaryCache,
    checksum_type: Option<ChecksumType>,
) -> Result<Box<dyn LogicalColDecoder + 'a>> {
    // match field.data_type() {
    //     DataType::List(child) | DataType::LargeList(child)
    //         if match child.data_type() {
    //             DataType::Struct(fields)
    //                 if fields
    //                     .iter()
    //                     .map(|f| match f.data_type() {
    //                         non_nest_types!() => true,
    //                         _ => false,
    //                     })
    //                     .fold(true, |acc, x| acc && x)
    //                     && cfg!(feature = "list-offsets-pushdown") =>
    //             {
    //                 true
    //             }
    //             _ => false,
    //         } =>
    //     {
    //         return Ok(Box::new(create_list_struct_decoder(
    //             r,
    //             field,
    //             field_id,
    //             column_metas,
    //             column_idx,
    //             wasm_context,
    //         )?))
    //     }
    //     _ => (),
    // }
    let column_index = column_idx.next_column_index();
    let column_meta = column_metas.get(column_index as usize).unwrap();
    let chunks_meta_iter = column_meta
        .column_chunks()
        .ok_or_else(|| Error::General("No chunks in column meta".to_string()))?
        .iter();
    match field.data_type() {
        non_nest_types!() => {
            let data_type = field.data_type().clone();
            Ok(Box::new(PrimitiveColDecoder {
                r,
                chunk_decoder: None,
                chunks_meta_iter,
                primitive_type: data_type,
                wasm_context: wasm_context.map(|wasm_context| Arc::clone(&wasm_context)),
                shared_dictionary_cache,
                checksum_type,
            }))
        }
        DataType::List(child) | DataType::LargeList(child) => {
            Ok(Box::new(ListColDecoder {
                field: Arc::clone(&field),
                validity_offsets_decoder: PrimitiveColDecoder {
                    r,
                    chunk_decoder: None,
                    chunks_meta_iter,
                    // CAUTION: here we create a list primitive decoder but only output validity and offsets.
                    primitive_type: field.data_type().clone(),
                    wasm_context: wasm_context.as_ref().map(Arc::clone),
                    shared_dictionary_cache,
                    checksum_type,
                },
                values_decoder: create_logical_decoder(
                    r,
                    Arc::clone(child),
                    column_metas,
                    column_idx,
                    wasm_context.map(|wasm_context| Arc::clone(&wasm_context)),
                    shared_dictionary_cache,
                    checksum_type,
                )?,
            }))
        }
        DataType::Struct(child_fields) => Ok(Box::new(StructColDecoder {
            fields: child_fields.clone(),
            // validity decoder for struct is a primitive decoder for Boolean
            validity_decoder: PrimitiveColDecoder {
                r,
                chunk_decoder: None,
                chunks_meta_iter,
                primitive_type: DataType::Boolean,
                wasm_context: wasm_context.as_ref().map(Arc::clone),
                shared_dictionary_cache,
                checksum_type,
            },
            children: child_fields
                .iter()
                .map(|f| {
                    create_logical_decoder(
                        r,
                        Arc::clone(f),
                        column_metas,
                        column_idx,
                        wasm_context.as_ref().map(Arc::clone),
                        shared_dictionary_cache,
                        checksum_type,
                    )
                })
                .collect::<Result<Vec<_>>>()?,
        })),
        _ => todo!("Implement logical encoding for field {}", field),
    }
}

pub fn advance_column_index(field: FieldRef, column_idx: &mut ColumnIndexSequence) -> Result<()> {
    match field.data_type() {
        non_nest_types!() => {
            let _column_index = column_idx.next_column_index();
            Ok(())
        }
        DataType::List(_child) | DataType::LargeList(_child) => {
            let _column_index = column_idx.next_column_index();
            Ok(())
        }
        DataType::Struct(_child_fields) => {
            todo!("Implement logical decoding for field {}", field)
        }
        _ => todo!("Implement logical encoding for field {}", field),
    }
}

fn field_to_view(field: FieldRef) -> FieldRef {
    match field.data_type() {
        DataType::Utf8 | DataType::LargeUtf8 => {
            Field::new(field.name(), DataType::Utf8View, field.is_nullable()).into()
        }
        DataType::Binary | DataType::LargeBinary => {
            Field::new(field.name(), DataType::BinaryView, field.is_nullable()).into()
        }
        _ => field,
    }
}
