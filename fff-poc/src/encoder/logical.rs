use std::{ops::Not, sync::Arc};

use super::{
    encoded_column_chunk::EncodedColumnChunk,
    physical::{self, create_physical_encoder, PhysicalColEncoder},
};
use crate::{
    common::ColumnIndexSequence,
    context::WASMWritingContext,
    counter::EncodingCounter,
    dict::{shared_dictionary_context::SharedDictionaryContext, DictionaryTypeOptions},
};
use arrow_array::cast::AsArray;
use arrow_array::Array;
use arrow_array::ArrayRef;
use arrow_array::{BooleanArray, Int32Array, Int64Array};
use arrow_buffer::BooleanBuffer;
use arrow_schema::{DataType, FieldRef};
use fff_core::{errors::Result, non_nest_types};
use fff_format::{File::fff::flatbuf as fb, ToFlatBuffer};
use flatbuffers::{FlatBufferBuilder, WIPOffset};

/// This maps to each logical column (i.e, field) in the Arrow schema.
/// It emits multiple of EncodedColumnChunk because the logical column may map to multiple physical columns.
pub trait LogicalColEncoder {
    fn encode(
        &mut self,
        array: ArrayRef,
        counter: &mut EncodingCounter,
        shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Option<Vec<EncodedColumnChunk>>>;

    fn memory_size(&self) -> usize;

    fn finish(
        &mut self,
        counter: &mut EncodingCounter,
        shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Option<Vec<EncodedColumnChunk>>>;

    fn submit_dict(&mut self, shared_dict_ctx: &mut SharedDictionaryContext) -> Result<()>;
}

#[derive(Debug)]
pub struct LogicalTree {
    id: fb::LogicalId,
    children: Vec<LogicalTree>,
}

impl LogicalTree {
    pub fn new(id: fb::LogicalId, children: Vec<LogicalTree>) -> Self {
        Self { id, children }
    }
}

impl ToFlatBuffer for LogicalTree {
    type Target<'a> = fb::LogicalTree<'a>;

    fn to_fb<'fb>(&self, fbb: &mut FlatBufferBuilder<'fb>) -> WIPOffset<Self::Target<'fb>> {
        let children: Vec<_> = self.children.iter().map(|child| child.to_fb(fbb)).collect();
        let children = fbb.create_vector(&children);
        fb::LogicalTree::create(
            fbb,
            &fb::LogicalTreeArgs {
                id: self.id,
                children: Some(children),
            },
        )
    }
}

/// For data types that are not List or Struct.
pub struct FlatColEncoder {
    data_encoder: Box<dyn PhysicalColEncoder>,
    column_index: u32,
}

impl LogicalColEncoder for FlatColEncoder {
    fn encode(
        &mut self,
        array: ArrayRef,
        counter: &mut EncodingCounter,
        shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Option<Vec<EncodedColumnChunk>>> {
        let mut res = vec![];
        for data_chunk in self.data_encoder.encode(array, counter, shared_dict_ctx)? {
            res.push(data_chunk.update_column_index(self.column_index));
        }
        Ok((!res.is_empty()).then_some(res))
    }

    fn memory_size(&self) -> usize {
        self.data_encoder.memory_size()
    }

    fn finish(
        &mut self,
        counter: &mut EncodingCounter,
        shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Option<Vec<EncodedColumnChunk>>> {
        let mut res = vec![];
        for data_chunk in self.data_encoder.finish(counter, shared_dict_ctx)? {
            res.push(data_chunk.update_column_index(self.column_index));
        }
        Ok((!res.is_empty()).then_some(res))
    }

    fn submit_dict(&mut self, shared_dict_ctx: &mut SharedDictionaryContext) -> Result<()> {
        self.data_encoder.submit_dict(shared_dict_ctx)
    }
}

pub struct ListColEncoder {
    /// validity is stored inside offsets_encoder
    offsets_encoder: Box<dyn PhysicalColEncoder>,
    /// This column index is for offsets column.
    column_index: u32,
    values_encoder: Box<dyn LogicalColEncoder>,
}

impl LogicalColEncoder for ListColEncoder {
    fn encode(
        &mut self,
        array: ArrayRef,
        counter: &mut EncodingCounter,
        shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Option<Vec<EncodedColumnChunk>>> {
        let mut res = vec![];
        // simply encode the validity and offsets buffers in this Array.
        for offsets_chunk in
            self.offsets_encoder
                .encode(Arc::clone(&array), counter, shared_dict_ctx)?
        {
            res.push(offsets_chunk.update_column_index(self.column_index));
        }
        if let Some(values_chunks) =
            self.values_encoder
                .encode(extract_items(&array), counter, shared_dict_ctx)?
        {
            res.extend(values_chunks);
        }
        Ok((!res.is_empty()).then_some(res))
    }

    fn memory_size(&self) -> usize {
        self.offsets_encoder.memory_size() + self.values_encoder.memory_size()
    }

    fn finish(
        &mut self,
        counter: &mut EncodingCounter,
        shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Option<Vec<EncodedColumnChunk>>> {
        let mut res = vec![];
        for offsets_chunk in self.offsets_encoder.finish(counter, shared_dict_ctx)? {
            res.push(offsets_chunk.update_column_index(self.column_index));
        }
        if let Some(values_chunks) = self.values_encoder.finish(counter, shared_dict_ctx)? {
            res.extend(values_chunks);
        }
        Ok((!res.is_empty()).then_some(res))
    }

    fn submit_dict(&mut self, shared_dict_ctx: &mut SharedDictionaryContext) -> Result<()> {
        self.offsets_encoder.submit_dict(shared_dict_ctx)?;
        self.values_encoder.submit_dict(shared_dict_ctx)
    }
}

pub struct ListOfStructOfPrimitiveColEncoder {
    /// List offsets and validity are pushdowned to Struct subfields in this encoder.
    fields_encoders: Vec<super::physical::ListOfStructColEncoder>,
    column_indexes: Vec<u32>,
}

impl LogicalColEncoder for ListOfStructOfPrimitiveColEncoder {
    fn encode(
        &mut self,
        array: ArrayRef,
        _counter: &mut EncodingCounter,
        _shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Option<Vec<EncodedColumnChunk>>> {
        let mut res = vec![];
        match array.data_type() {
            DataType::List(_) => {
                for ((field_encoder, field_array), col_idx) in self
                    .fields_encoders
                    .iter_mut()
                    .zip(array.as_list::<i32>().values().as_struct().columns())
                    .zip(&self.column_indexes)
                {
                    if let Some(field_chunk) =
                        field_encoder.encode(Arc::clone(&array), Arc::clone(field_array))?
                    {
                        res.push(field_chunk.update_column_index(*col_idx));
                    }
                }
            }
            DataType::LargeList(_) => {
                for ((field_encoder, field_array), col_idx) in self
                    .fields_encoders
                    .iter_mut()
                    .zip(array.as_list::<i64>().values().as_struct().columns())
                    .zip(&self.column_indexes)
                {
                    if let Some(field_chunk) =
                        field_encoder.encode(Arc::clone(&array), Arc::clone(field_array))?
                    {
                        res.push(field_chunk.update_column_index(*col_idx));
                    }
                }
            }
            _ => panic!("Expecting List or LargeList data type"),
        }
        Ok((!res.is_empty()).then_some(res))
    }

    fn memory_size(&self) -> usize {
        self.fields_encoders.iter().fold(0, |mut acc, x| {
            acc += x.memory_size();
            acc
        })
    }

    fn finish(
        &mut self,
        _counter: &mut EncodingCounter,
        _shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Option<Vec<EncodedColumnChunk>>> {
        let mut res = vec![];
        for (field_encoder, col_idx) in self.fields_encoders.iter_mut().zip(&self.column_indexes) {
            if let Some(field_chunk) = field_encoder.finish()? {
                res.push(field_chunk.update_column_index(*col_idx));
            }
        }
        Ok((!res.is_empty()).then_some(res))
    }

    fn submit_dict(&mut self, _shared_dict_ctx: &mut SharedDictionaryContext) -> Result<()> {
        Ok(())
    }
}

pub struct StructColEncoder {
    validity_encoder: Box<dyn PhysicalColEncoder>,
    /// This column index is for validity column.
    column_index: u32,
    fields_encoders: Vec<Box<dyn LogicalColEncoder>>,
}

impl LogicalColEncoder for StructColEncoder {
    fn encode(
        &mut self,
        array: ArrayRef,
        counter: &mut EncodingCounter,
        shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Option<Vec<EncodedColumnChunk>>> {
        let mut res = vec![];
        for validity_chunk in
            self.validity_encoder
                .encode(extract_validity(&array), counter, shared_dict_ctx)?
        {
            res.push(validity_chunk.update_column_index(self.column_index));
        }
        for (field_encoder, field_array) in self
            .fields_encoders
            .iter_mut()
            .zip(array.as_struct().columns())
        {
            if let Some(field_chunks) =
                field_encoder.encode(Arc::clone(field_array), counter, shared_dict_ctx)?
            {
                res.extend(field_chunks);
            }
        }
        Ok(res.is_empty().not().then_some(res))
    }

    fn memory_size(&self) -> usize {
        self.validity_encoder.memory_size()
            + self
                .fields_encoders
                .iter()
                .map(|e| e.memory_size())
                .sum::<usize>()
    }

    fn finish(
        &mut self,
        counter: &mut EncodingCounter,
        shared_dict_ctx: &mut SharedDictionaryContext,
    ) -> Result<Option<Vec<EncodedColumnChunk>>> {
        let mut res = vec![];
        for validity_chunk in self.validity_encoder.finish(counter, shared_dict_ctx)? {
            res.push(validity_chunk.update_column_index(self.column_index));
        }
        for field_encoder in self.fields_encoders.iter_mut() {
            if let Some(field_chunks) = field_encoder.finish(counter, shared_dict_ctx)? {
                res.extend(field_chunks);
            }
        }
        Ok((!res.is_empty()).then_some(res))
    }

    fn submit_dict(&mut self, shared_dict_ctx: &mut SharedDictionaryContext) -> Result<()> {
        self.validity_encoder.submit_dict(shared_dict_ctx)?;
        for field_encoder in self.fields_encoders.iter_mut() {
            field_encoder.submit_dict(shared_dict_ctx)?;
        }
        Ok(())
    }
}

#[allow(clippy::only_used_in_recursion)]
pub fn create_logical_encoder(
    field: FieldRef,
    field_id: i32,
    max_chunk_size: u64,
    column_idx: &mut ColumnIndexSequence,
    wasm_context: Arc<WASMWritingContext>,
    dictionary_type: DictionaryTypeOptions,
    compression_type: fb::CompressionType,
) -> Result<(Box<dyn LogicalColEncoder>, LogicalTree)> {
    match field.data_type() {
        non_nest_types!() => Ok((
            Box::new(FlatColEncoder {
                data_encoder: create_physical_encoder(
                    field.data_type(),
                    max_chunk_size,
                    field.is_nullable(),
                    wasm_context,
                    dictionary_type,
                    compression_type,
                )?,
                column_index: column_idx.next_column_index(),
            }),
            LogicalTree::new(fb::LogicalId::FLAT, vec![]),
        )),
        DataType::List(child) | DataType::LargeList(child) => {
            match child.data_type() {
                // Pushingdown List offsets only works for List(Struct(non_nest_type!()))
                // TODO: support more complex nested types. Don't forget to modify decoder.
                DataType::Struct(fields)
                    if fields
                        .iter()
                        .all(|f| matches!(f.data_type(), non_nest_types!()))
                        && cfg!(feature = "list-offsets-pushdown") =>
                {
                    Ok((
                        Box::new(ListOfStructOfPrimitiveColEncoder {
                            fields_encoders: fields
                                .iter()
                                .map(|_| {
                                    physical::ListOfStructColEncoder::new(
                                        max_chunk_size,
                                        wasm_context.clone(),
                                        compression_type,
                                    )
                                })
                                .collect(),
                            column_indexes: (0..fields.len())
                                .map(|_| column_idx.next_column_index())
                                .collect(),
                        }),
                        LogicalTree::new(fb::LogicalId::LIST_OF_STRUCT_OF_PRIMITIVE, vec![]),
                    ))
                }
                _ => {
                    // Validity and Offsets in List are encode together, and a physical encoder is created for them.
                    let offsets_validity_index = column_idx.next_column_index();
                    let offsets_encoder = create_physical_encoder(
                        field.data_type(),
                        max_chunk_size,
                        field.is_nullable(),
                        wasm_context.clone(),
                        dictionary_type,
                        compression_type,
                    )?;
                    let (values_encoder, child_tree) = create_logical_encoder(
                        Arc::clone(child),
                        field_id,
                        max_chunk_size,
                        column_idx,
                        wasm_context,
                        dictionary_type,
                        compression_type,
                    )?;
                    Ok((
                        Box::new(ListColEncoder {
                            offsets_encoder,
                            column_index: offsets_validity_index,
                            values_encoder,
                        }),
                        LogicalTree::new(fb::LogicalId::LIST, vec![child_tree]),
                    ))
                }
            }
        }
        DataType::Struct(child_fields) => {
            let validity_index = column_idx.next_column_index();
            let mut fields_encoders = vec![];
            let mut child_trees = vec![];
            for child_field in child_fields.iter() {
                let (enc, child_tree) = create_logical_encoder(
                    Arc::clone(child_field),
                    field_id,
                    max_chunk_size,
                    column_idx,
                    wasm_context.clone(),
                    dictionary_type,
                    compression_type,
                )?;
                fields_encoders.push(enc);
                child_trees.push(child_tree);
            }
            Ok((
                Box::new(StructColEncoder {
                    validity_encoder: create_physical_encoder(
                        &DataType::Boolean,
                        max_chunk_size,
                        false,
                        wasm_context.clone(),
                        dictionary_type,
                        compression_type,
                    )?,
                    column_index: validity_index,
                    fields_encoders,
                }),
                LogicalTree::new(fb::LogicalId::STRUCT, child_trees),
            ))
        }
        DataType::Map(_, _) => {
            todo!("implement map")
        }
        _ => {
            todo!("Implement logical encoding for field {}", field)
        }
    }
}

fn extract_items(list_arr: &dyn Array) -> ArrayRef {
    match list_arr.data_type() {
        DataType::List(_) => {
            let list_arr = list_arr.as_list::<i32>();
            let items_start = list_arr.value_offsets()[list_arr.offset()] as usize;
            let items_end = list_arr.value_offsets()[list_arr.offset() + list_arr.len()] as usize;
            list_arr
                .values()
                .slice(items_start, items_end - items_start)
        }
        DataType::LargeList(_) => {
            let list_arr = list_arr.as_list::<i64>();
            let items_start = list_arr.value_offsets()[list_arr.offset()] as usize;
            let items_end = list_arr.value_offsets()[list_arr.offset() + list_arr.len()] as usize;
            list_arr
                .values()
                .slice(items_start, items_end - items_start)
        }
        _ => panic!(),
    }
}

/// Note: From Lance
/// Given a list array, return the offsets as a standalone ArrayRef (either an Int32Array or Int64Array)
fn _extract_offsets_and_validity(list_arr: &dyn Array) -> ArrayRef {
    match list_arr.data_type() {
        DataType::List(_) => {
            let offsets = list_arr.as_list::<i32>().offsets().clone();
            let nulls = list_arr.nulls().cloned();
            Arc::new(Int32Array::new(offsets.into_inner(), nulls))
        }
        DataType::LargeList(_) => {
            let offsets = list_arr.as_list::<i64>().offsets().clone();
            let nulls = list_arr.nulls().cloned();
            Arc::new(Int64Array::new(offsets.into_inner(), nulls))
        }
        _ => panic!(),
    }
}

/// Note: From Lance
/// Converts the validity of a list/struct array into a boolean array.  If there is no validity information
/// then this is an all-valid boolean array.
fn extract_validity(list_arr: &dyn Array) -> ArrayRef {
    if let Some(validity) = list_arr.nulls() {
        Arc::new(BooleanArray::new(validity.inner().clone(), None))
    } else {
        // If there is no validity information, then this is an all-valid boolean array.
        // Use empty Array here will cause wrong num_rows in EncUnit metadata in flatbuffer,
        // which causes bug in row skipping logic.
        Arc::new(BooleanArray::new(
            BooleanBuffer::new_set(list_arr.len()),
            None,
        ))
    }
}

mod tests {
    #[test]
    #[allow(clippy::arc_with_non_send_sync)]
    fn test_list_encoder() {
        use arrow::array::{Int32Builder, ListBuilder};
        use arrow_array::RecordBatch;

        use super::*;
        use arrow_schema::Field;
        use arrow_schema::Schema;
        use std::sync::Arc;

        let schema = Arc::new(Schema::new(vec![Field::new(
            "a",
            DataType::List(Arc::new(Field::new("item", DataType::Int32, true))),
            true,
        )]));
        let mut builder = ListBuilder::new(Int32Builder::new());
        builder.append_value([Some(1), Some(2), Some(3)]);
        builder.append_value([Some(4), Some(5)]);
        builder.append_value([None]);
        builder.append_value([Some(6), Some(7), Some(8), Some(9)]);
        let a = builder.finish();
        let input_batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(a)]).unwrap();
        let mut encoder = create_logical_encoder(
            Arc::new(input_batch.schema_ref().field(0).clone()),
            0,
            1024 * 1024,
            &mut ColumnIndexSequence::default(),
            Arc::new(WASMWritingContext::empty()),
            DictionaryTypeOptions::EncoderDictionary,
            fb::CompressionType::Uncompressed,
        )
        .unwrap()
        .0;
        let mut counter = EncodingCounter::default();
        let mut shared_dict_ctx = SharedDictionaryContext::default();
        let mut chunks = encoder
            .encode(
                input_batch.column(0).clone(),
                &mut counter,
                &mut shared_dict_ctx,
            )
            .unwrap()
            .unwrap_or(vec![]);
        chunks.extend(
            encoder
                .finish(&mut counter, &mut shared_dict_ctx)
                .unwrap()
                .unwrap(),
        );

        assert_eq!(chunks.len(), 2);
        let offsets_chunk = &chunks[0];
        let values_chunk = &chunks[1];
        assert_eq!(offsets_chunk.encunits.len(), 1);
        assert_eq!(values_chunk.encunits.len(), 1);
    }
}
