use std::collections::HashMap;
use std::io::{BufWriter, Seek, Write};
use std::iter::once;
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_ipc::writer::IpcWriteOptions;
use arrow_ipc::writer::{DictionaryTracker, IpcDataGenerator};
use arrow_schema::Schema;
use arrow_schema::SchemaRef;
use fff_format::File::fff::flatbuf as fb;
use fff_format::ToFlatBuffer;
use fff_format::{File::fff::flatbuf::CompressionType, MAGIC, MAJOR_VERSION, MINOR_VERSION};
use flatbuffers::FlatBufferBuilder;

use crate::common::checksum::create_checksum;
use crate::common::checksum::Checksum;
use crate::common::checksum::ChecksumType;
use crate::common::ColumnIndexSequence;
use crate::context::WASMWritingContext;
use crate::counter::EncodingCounter;
use crate::dict::shared_dictionary::SharedDictionaryTable;
use crate::dict::shared_dictionary_context::SharedDictionaryContext;
use crate::dict::DictionaryTypeOptions;
use crate::encoder::encoded_column_chunk::EncodedColumnChunk;
use crate::encoder::logical::LogicalColEncoder;
use crate::encoder::logical::{create_logical_encoder, LogicalTree};
use crate::file::footer::create_default_encoding_versions;
use crate::file::footer::{self, Chunk, ColumnMetadata, RowGroupMetadata, RowGroupsTable};
use crate::options::FileWriterOptions;

use fff_core::{errors::Result, nyi_err};

struct FileWriteState<W: Write + Seek> {
    writer: BufWriter<W>,
    row_groups_table: RowGroupsTable,
    num_rows_in_file: u32,
    num_physical_columns: usize,
    data_checksum: Box<dyn Checksum>,
    column_counters: Vec<EncodingCounter>,
    enable_io_unit_checksum: bool,
    /// Metadata for the current row group.
    column_metadatas_in_cur_row_group: Vec<ColumnMetadata>,
    start_offset_of_cur_row_group: u64,
    num_rows_in_cur_row_group: u32,
}

impl<W> FileWriteState<W>
where
    W: Write + Seek,
{
    pub fn flush_chunk(&mut self, chunk: EncodedColumnChunk) -> Result<()> {
        let column_index = chunk.column_index;
        let chunk_meta = self.flush_chunk_and_get_metadata(chunk)?;
        // use chunk.column_index to let the metadata knows which physical column does this chunk belong to
        self.column_metadatas_in_cur_row_group[column_index as usize].add_chunk(chunk_meta);
        Ok(())
    }

    fn write_and_update_file_level_checksum(&mut self, buf: &[u8]) -> Result<()> {
        self.writer.write_all(buf)?;
        self.data_checksum.update(buf);
        Ok(())
    }

    pub fn flush_chunk_and_get_metadata(&mut self, chunk: EncodedColumnChunk) -> Result<Chunk> {
        // println!("flush chunk with index {}", chunk.column_index);
        let offset = self.writer.stream_position()?;
        let mut iounit_checksum = self
            .enable_io_unit_checksum
            .then_some(create_checksum(&ChecksumType::XxHash));
        let encunit_metas = chunk
            .encunits
            .into_iter()
            .map(|unit| {
                let buf = unit.bytes();
                self.write_and_update_file_level_checksum(buf.as_ref())
                    .unwrap();
                if let Some(checksum) = &mut iounit_checksum {
                    checksum.update(buf.as_ref());
                }
                footer::EncUnit::new(
                    buf.len() as u32,
                    unit.num_rows(),
                    unit.encoding().clone(),
                    unit.compression_type(),
                )
            })
            .collect();
        let size: u64 = self.writer.stream_position()? - offset;
        // use chunk.column_index to let the metadata knows which physical column does this chunk belong to
        Ok(Chunk::new(
            offset,
            size as u32,
            chunk.num_rows as u64,
            chunk.dict_encoding,
            encunit_metas,
            iounit_checksum.map(|c| c.finalize()),
        ))
    }

    /// Finish the current row group and add it to the row groups table.
    pub fn finish_row_group(&mut self) -> Result<()> {
        self.row_groups_table.add_meta(
            self.num_rows_in_cur_row_group,
            self.start_offset_of_cur_row_group,
            (self.writer.stream_position()? - self.start_offset_of_cur_row_group) as u32,
            RowGroupMetadata::new(std::mem::replace(
                &mut self.column_metadatas_in_cur_row_group,
                vec![ColumnMetadata::default(); self.num_physical_columns],
            )),
        );
        self.num_rows_in_cur_row_group = 0;
        self.start_offset_of_cur_row_group = self.writer.stream_position()?;
        Ok(())
    }

    // Deprecated flush logic with null info
    // pub fn flush_chunk(&mut self, chunk: EncodedColumnChunk) -> Result<()> {
    //     let offset = self.writer.stream_position()?;
    //     let blocks = chunk
    //         .encunits
    //         .into_iter()
    //         .map(|block| {
    //             let null_info: Option<footer::NullInfo> = if let Some(info) = block.null_info {
    //                 let vblock: Option<footer::EncUnit> =
    //                     if let Some(encblock) = info.validity_block {
    //                         let buf = encblock.buffer;
    //                         self.writer.write_all(buf.as_slice()).unwrap();
    //                         self.data_checksum.update(buf.as_slice());
    //                         Some(footer::EncUnit::new(buf.len() as u32, encblock.encoding))
    //                     } else {
    //                         None
    //                     };
    //                 Some(footer::NullInfo::new(info.null_type, vblock))
    //             } else {
    //                 None
    //             };
    //             let data_blocks: Option<Vec<footer::EncUnit>> =
    //                 if let Some(encblock) = block.data_blocks {
    //                     Some(
    //                         encblock
    //                             .into_iter()
    //                             .map(|b| {
    //                                 let buf = b.buffer;
    //                                 self.writer.write_all(buf.as_slice()).unwrap();
    //                                 self.data_checksum.update(buf.as_slice());
    //                                 footer::EncUnit::new(buf.len() as u32, b.encoding)
    //                             })
    //                             .collect(),
    //                     )
    //                 } else {
    //                     None
    //                 };
    //             footer::Block::new(block.num_rows, null_info, data_blocks)
    //         })
    //         .collect();
    //     let size: u64 = self.writer.stream_position()? - offset;
    //     // use chunk.column_index to let the metadata knows which physical column does this chunk belong to
    //     self.column_metadatas[chunk.column_index as usize].add_chunk(Chunk::new(
    //         offset,
    //         size as u32,
    //         chunk.num_rows as u64,
    //         chunk.dict_encoding,
    //         blocks,
    //     ));
    //     Ok(())
    // }
}

pub struct FileWriter<W: Write + Seek> {
    schema: Schema,
    column_encoders: Vec<Box<dyn LogicalColEncoder>>,
    logical_tree: LogicalTree,
    state: FileWriteState<W>,
    schema_checksum: Box<dyn Checksum>,
    wasm_context: Arc<WASMWritingContext>,
    custom_encunit_len: HashMap<usize, usize>,
    row_group_size: u64,
    shared_dictionary_context: SharedDictionaryContext,
}

impl<W: Write + Seek> FileWriter<W> {
    pub fn try_new(schema: SchemaRef, writer: W, mut options: FileWriterOptions) -> Result<Self> {
        let checksum_type = options.checksum_type();
        let mut column_idx = ColumnIndexSequence::default();
        let wasm_context = Arc::new(
            match (
                options.write_built_in_wasm(),
                options.custom_encoding_options().len() > 0,
            ) {
                (true, false) => WASMWritingContext::default_with_always_set_custom_wasm(),
                (false, true) => options.take_custom_encoding_options().into_context(),
                (false, false) => WASMWritingContext::empty(),
                _ => todo!("Cleanup this stupid code"),
            },
        );
        let mut column_encoders = vec![];
        let mut child_trees = vec![];
        let shared_dictionary_context = SharedDictionaryContext::new(
            options.encoding_unit_len(),
            options.iounit_size(),
            options.dictionary_type() == DictionaryTypeOptions::GlobalDictionaryMultiColSharing,
            options.compression_type(),
        );
        for (field_id, field) in schema.fields().iter().enumerate() {
            let (encoder, child_tree) = create_logical_encoder(
                Arc::clone(field),
                field_id as i32,
                options.iounit_size(),
                &mut column_idx,
                wasm_context.clone(),
                options.dictionary_type(),
                options.compression_type(),
            )?;
            column_encoders.push(encoder);
            child_trees.push(child_tree);
        }
        let num_physical_columns = column_idx.get_current_index() as usize;
        Ok(Self {
            schema: schema.as_ref().clone(),
            column_encoders,
            logical_tree: LogicalTree::new(fb::LogicalId::STRUCT, child_trees),
            state: FileWriteState {
                writer: BufWriter::new(writer),
                column_metadatas_in_cur_row_group: vec![
                    ColumnMetadata::default();
                    num_physical_columns
                ],
                row_groups_table: RowGroupsTable::default(),
                start_offset_of_cur_row_group: 0,
                num_rows_in_file: 0,
                num_physical_columns,
                num_rows_in_cur_row_group: 0,
                data_checksum: create_checksum(&checksum_type),
                column_counters: vec![EncodingCounter::default(); num_physical_columns],
                enable_io_unit_checksum: options.enable_io_unit_checksum(),
            },
            schema_checksum: create_checksum(&checksum_type),
            wasm_context,
            custom_encunit_len: options.custom_encunit_len().clone(),
            row_group_size: options.row_group_size(),
            shared_dictionary_context,
        })
    }

    pub fn write_batch(&mut self, batch: &RecordBatch) -> Result<()> {
        // push each array into the column writer
        // the logic of metadata should also be in the column writer
        for (i, col) in batch.columns().iter().enumerate() {
            let encoder = self.column_encoders[i].as_mut();
            // TODO: currently this is for research experiments.
            // A detailed API similar to Parquet's write_batch with correct internal buffer should be added.
            // Currently requires the input batch size to be the multiple of the custom encunit size.
            if let Some(encunit_len) = self.custom_encunit_len.get(&i) {
                if batch.num_rows() < *encunit_len {
                    return nyi_err!("Batch size should be larger than the custom encunit size");
                }
                for j in 0..(batch.num_rows() / encunit_len) {
                    // slice col to the correct range, then encode
                    let col_sliced = col.slice(
                        j * encunit_len,
                        std::cmp::min(*encunit_len, col.len() - j * encunit_len),
                    );
                    if let Some(res) = encoder.encode(
                        col_sliced.clone(),
                        &mut self.state.column_counters[i],
                        &mut self.shared_dictionary_context,
                    )? {
                        res.into_iter()
                            .try_for_each(|chunk| self.state.flush_chunk(chunk))?;
                    };
                }
            } else {
                if let Some(res) = encoder.encode(
                    col.clone(),
                    &mut self.state.column_counters[i],
                    &mut self.shared_dictionary_context,
                )? {
                    res.into_iter()
                        .try_for_each(|chunk| self.state.flush_chunk(chunk))?;
                };
            }
        }
        self.state.num_rows_in_file += batch.num_rows() as u32;
        self.state.num_rows_in_cur_row_group += batch.num_rows() as u32;
        if self.state.num_rows_in_cur_row_group as u64 >= self.row_group_size {
            self.flush_pending_chunks()?;
            self.state.finish_row_group()?;
        }
        Ok(())
    }

    pub fn memory_size(&self) -> usize {
        self.column_encoders.iter().map(|e| e.memory_size()).sum()
    }

    /// For testing memory usage if we correctly implement row groups
    pub fn flush_pending_chunks(&mut self) -> Result<()> {
        for (i, encoder) in self.column_encoders.iter_mut().enumerate() {
            if let Some(res) = encoder.finish(
                &mut self.state.column_counters[i],
                &mut self.shared_dictionary_context,
            )? {
                res.into_iter()
                    .try_for_each(|chunk| self.state.flush_chunk(chunk))?;
            };
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<Vec<EncodingCounter>> {
        // if dictionary mode is global with sharing, first submit all values to dictionary context
        if self.shared_dictionary_context.is_multi_col_sharing() {
            for encoder in self.column_encoders.iter_mut() {
                encoder.submit_dict(&mut self.shared_dictionary_context)?;
            }
            self.shared_dictionary_context.merge_dicts()?;
        }

        // flush pendding data in encoders
        self.flush_pending_chunks()?;

        // Make sure flushed pending data added to row group metadata
        self.state.finish_row_group()?;

        // flush shared dictionary
        let (dict_chunks, merge_peers, dict_dtypes) = self
            .shared_dictionary_context
            .finish_and_flush(self.wasm_context.clone(), &mut self.state.column_counters)?;
        let dict_chunks = dict_chunks
            .into_iter()
            .map(|chunks| -> Result<Vec<Chunk>> {
                chunks
                    .into_iter()
                    .map(|chunk| self.state.flush_chunk_and_get_metadata(chunk))
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut fbb = FlatBufferBuilder::new();
        // write WASM binaries.
        let wasms: Vec<_> = self
            .wasm_context
            .get_sorted_wasms()
            .into_iter()
            .map(|wasm| {
                let offset = self.state.writer.stream_position()?;
                self.state.write_and_update_file_level_checksum(wasm)?;
                let size = self.state.writer.stream_position()? - offset;
                let mut b = fb::MetadataSectionBuilder::new(&mut fbb);
                b.add_offset(offset as u64);
                b.add_size_(size as u32);
                b.add_compression_type(CompressionType::Uncompressed);
                Ok(b.finish())
            })
            .collect::<Result<Vec<flatbuffers::WIPOffset<fb::MetadataSection>>>>()?;
        // write wasm binaries locations as an optional metadata section
        let wasms = fbb.create_vector(&wasms);
        let mut wasm_b_builder = fb::WASMBinariesBuilder::new(&mut fbb);
        wasm_b_builder.add_wasm_binaries(wasms);
        let wasms = wasm_b_builder.finish();
        fbb.finish(wasms, None);
        let wasms = fbb.finished_data();
        let wasm_meta_start = self.state.writer.stream_position()?;
        self.state.write_and_update_file_level_checksum(wasms)?;
        let wasm_meta_size = self.state.writer.stream_position()? - wasm_meta_start;

        // write ColumnMetadata and update indirect_row_group_metadata
        let metadata_start = self
            .state
            .row_groups_table
            .to_indirect_and_flush(&mut self.state.writer, self.state.data_checksum.as_mut())?;

        // write RowGroups fbs table to file
        let mut fbb = FlatBufferBuilder::new();
        let row_counts = fbb.create_vector(self.state.row_groups_table.row_counts());
        let offsets = fbb.create_vector(self.state.row_groups_table.offsets());
        let sizes = fbb.create_vector(self.state.row_groups_table.sizes());

        let row_group_metadatas = self
            .state
            .row_groups_table
            .indirect_row_group_metadata()
            .iter()
            .map(|x| x.to_fb(&mut fbb))
            .collect::<Vec<_>>();
        let row_group_metadatas = fbb.create_vector(&row_group_metadatas);
        let row_groups = {
            let mut row_group_builder = fb::RowGroupsBuilder::new(&mut fbb);
            row_group_builder.add_row_counts(row_counts);
            row_group_builder.add_offsets(offsets);
            row_group_builder.add_sizes(sizes);
            row_group_builder.add_row_group_metadatas(row_group_metadatas);
            row_group_builder.finish()
        };

        // write shared dictionary table
        let dict_start_idx = dict_chunks
            .iter()
            .map(|v| v.len())
            .scan(0, |acc, x| {
                let begin = *acc;
                *acc += x;
                Some(begin)
            })
            .collect::<Vec<_>>();
        let dict_positions = dict_start_idx
            .iter()
            .enumerate()
            .map(|(i, begin)| {
                if let Some(Some(peer)) = merge_peers.get(i) {
                    once(dict_start_idx[*peer] as u32)
                        .chain((*begin..*begin + dict_chunks[i].len()).map(|x| x as u32))
                        .collect::<Vec<_>>()
                } else {
                    (*begin..*begin + dict_chunks[i].len())
                        .map(|x| x as u32)
                        .collect::<Vec<_>>()
                }
            })
            .collect::<Vec<_>>();
        let dict_chunks = dict_chunks.into_iter().flatten().collect::<Vec<_>>();
        let shared_dict_table =
            SharedDictionaryTable::new(dict_chunks, dict_positions, dict_dtypes);
        let shared_dict_table = shared_dict_table.to_fb(&mut fbb);

        // TODO: write Statistics to file

        // write Footer to file
        let data_gen = IpcDataGenerator {};
        let write_options = IpcWriteOptions::default();
        // This is how Parquet encodes Arrow schema: https://github.com/apache/arrow-rs/blob/24a6bff6769a5c6062aafe52b6702459086d3b94/parquet/src/arrow/schema/mod.rs#L173
        let mut dictionary_tracker =
            DictionaryTracker::new_with_preserve_dict_id(true, write_options.preserve_dict_id());
        let schema = data_gen
            .schema_to_bytes_with_dictionary_tracker(
                &self.schema,
                &mut dictionary_tracker,
                &write_options,
            )
            .ipc_message;
        let schema_checksum = {
            self.schema_checksum.update(&schema);
            self.schema_checksum.finalize()
        };
        let schema = fbb.create_vector(&schema);
        let logical_tree = self.logical_tree.to_fb(&mut fbb);

        let optional_metadata_section = {
            let name = fbb.create_string("WASMBinaries");
            let names = fbb.create_vector(&[name]);
            let offsets = fbb.create_vector(&[wasm_meta_start as u64]);
            let sizes = fbb.create_vector(&[wasm_meta_size as u32]);
            let compression_types = fbb.create_vector(&[CompressionType::Uncompressed]);
            let mut builder = fb::OptionalMetadataSectionsBuilder::new(&mut fbb);
            builder.add_names(names);
            builder.add_offsets(offsets);
            builder.add_sizes(sizes);
            builder.add_compression_types(compression_types);
            builder.finish()
        };

        // Create encoding versions
        let encoding_versions = create_default_encoding_versions()?;
        let encoding_versions_fb = encoding_versions
            .iter()
            .map(|ev| ev.to_fb(&mut fbb))
            .collect::<Vec<_>>();
        let encoding_versions_fb = fbb.create_vector(&encoding_versions_fb);

        let footer = {
            let mut footer_builder = fb::FooterBuilder::new(&mut fbb);
            footer_builder.add_schema(schema);
            footer_builder.add_logical_tree(logical_tree);
            footer_builder.add_row_groups(row_groups);
            footer_builder.add_optional_sections(optional_metadata_section);
            footer_builder.add_shared_dictionary_table(shared_dict_table);
            footer_builder.add_encoding_versions(encoding_versions_fb);
            footer_builder.finish()
        };
        fbb.finish(footer, None);
        let footer_data = fbb.finished_data();
        self.state
            .write_and_update_file_level_checksum(footer_data)?;

        // write postscript to file
        let writer = &mut self.state.writer;
        let metadata_size = (writer.stream_position()? - metadata_start) as u32;
        writer.write_all(metadata_size.to_le_bytes().as_ref())?;
        let footer_size = footer_data.len() as u32;
        writer.write_all(footer_size.to_le_bytes().as_ref())?;
        let footer_compression = CompressionType::Uncompressed;
        writer.write_all(u8::from(footer_compression).to_le_bytes().as_ref())?;
        writer.write_all((ChecksumType::XxHash as u8).to_le_bytes().as_ref())?;
        writer.write_all(self.state.data_checksum.finalize().to_le_bytes().as_ref())?;
        writer.write_all(schema_checksum.to_le_bytes().as_ref())?;
        writer.write_all(MAJOR_VERSION.to_le_bytes().as_ref())?;
        writer.write_all(MINOR_VERSION.to_le_bytes().as_ref())?;
        writer.write_all(MAGIC)?;
        writer.flush()?;
        Ok(self.state.column_counters)
    }
}
