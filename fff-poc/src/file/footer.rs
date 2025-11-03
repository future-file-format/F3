use fff_format::File::fff::flatbuf::root_as_footer;
use fff_format::ToFlatBuffer;
use fff_format::POSTSCRIPT_SIZE;
use flatbuffers::{FlatBufferBuilder, WIPOffset};
use semver::Version;
use std::collections::HashMap;
use std::io::Seek;
use std::io::Write;
use std::sync::LazyLock;

use arrow_ipc::convert::fb_to_schema;
use arrow_ipc::root_as_message;
use arrow_schema::Schema;
use arrow_schema::SchemaRef;
use fff_format::File::fff::flatbuf as fb;

use crate::common::checksum::Checksum;
use crate::common::checksum::ChecksumType;
use crate::reader::RowGroupCntNPointer;
use fff_core::errors::{Error, Result};

/// Default encoding versions map
pub(crate) static DEFAULT_ENCODING_VERSIONS: LazyLock<HashMap<fb::EncodingType, Version>> =
    LazyLock::new(|| {
        HashMap::from([
            // (fb::EncodingType::PLAIN, Version::parse("0.1.0").unwrap()),
            // (fb::EncodingType::NULLABLE, Version::parse("0.1.0").unwrap()),
            (fb::EncodingType::CASCADE, Version::parse("0.21.0").unwrap()),
            (
                fb::EncodingType::CUSTOM_WASM,
                Version::parse("1.0.0").unwrap(),
            ),
        ])
    });

pub struct PostScript {
    pub metadata_size: u32,
    pub footer_size: u32,
    // TODO: probably not need compression for footer.
    pub compression: fb::CompressionType,
    pub checksum_type: ChecksumType,
    pub data_checksum: u64,
    pub schema_checksum: u64,
    pub major_version: u16,
    pub minor_version: u16,
}

/// Maps an encoding type to its semantic version
#[derive(Clone, Debug)]
pub struct EncodingVersion {
    encoding_type: fb::EncodingType,
    version: Version,
}

impl EncodingVersion {
    pub fn new(encoding_type: fb::EncodingType, version: Version) -> Self {
        Self {
            encoding_type,
            version,
        }
    }
}

impl ToFlatBuffer for EncodingVersion {
    type Target<'a> = fb::EncodingVersion<'a>;

    fn to_fb<'fb>(&self, fbb: &mut FlatBufferBuilder<'fb>) -> WIPOffset<Self::Target<'fb>> {
        let sem_ver = fb::SemVer::create(
            fbb,
            &fb::SemVerArgs {
                major: self.version.major,
                minor: self.version.minor,
                patch: self.version.patch,
            },
        );
        fb::EncodingVersion::create(
            fbb,
            &fb::EncodingVersionArgs {
                encoding_type: self.encoding_type,
                version: Some(sem_ver),
            },
        )
    }
}

/// Helper function to create encoding versions for all supported encoding types
pub(crate) fn create_default_encoding_versions() -> Result<Vec<EncodingVersion>> {
    let mut encoding_versions = Vec::new();

    for (&encoding_type, version) in DEFAULT_ENCODING_VERSIONS.iter() {
        encoding_versions.push(EncodingVersion::new(encoding_type, version.clone()));
    }

    Ok(encoding_versions)
}

#[derive(Clone, Default)]
pub(crate) enum DictionaryEncoding {
    #[default]
    NoDictionary,
    /// Stores EncBlockIndex of dictionary chunks
    Dictionary(Vec<u32>),
    /// Store the index of shared dictionary
    SharedDictionary(u32),
}

/// ColumnMetadata (direct) for writer to use.
/// Reader should use [fb::ColumnMetadata](fff_format::File::fff::flatbuf::ColumnMetadata) directly.
#[derive(Default, Clone)]
pub struct ColumnMetadata {
    column_chunks: Vec<Chunk>,
}

// impl From<&fb::ColumnMetadata<'_>> for ColumnMetadata {
//     fn from(column_metadata: &fb::ColumnMetadata) -> Self {
//         Self {
//             column_chunks: column_metadata
//                 .column_chunks()
//                 .into_iter()
//                 .flatten()
//                 .map(|x| Chunk::from(&x))
//                 .collect(),
//         }
//     }
// }

impl ColumnMetadata {
    pub fn add_chunk(&mut self, chunk: Chunk) {
        self.column_chunks.push(chunk);
    }
}

impl ToFlatBuffer for ColumnMetadata {
    type Target<'a> = fb::ColumnMetadata<'a>;

    fn to_fb<'fb>(&self, fbb: &mut FlatBufferBuilder<'fb>) -> WIPOffset<Self::Target<'fb>> {
        // All column chunks are written to the buffer in fbb
        // Then the WIPOffsets (just uints) are collected and created as a Vector
        // The copy cost is not significant
        let chunks = &self
            .column_chunks
            .iter()
            .map(|x| x.to_fb(fbb))
            .collect::<Vec<_>>();
        let chunks = fbb.create_vector(chunks);
        fb::ColumnMetadata::create(
            fbb,
            &fb::ColumnMetadataArgs {
                column_chunks: Some(chunks),
            },
        )
    }
}

/// Chunk for writer to use.
/// Reader should use [fb::Chunk](fff_format::File::fff::flatbuf::Chunk) directly.
#[derive(Default, Clone)]
pub struct Chunk {
    offset: u64,
    size: u32,
    num_rows: u64,
    encoding: DictionaryEncoding,
    blocks: Vec<EncUnit>,
    checksum: Option<u64>,
}
// impl From<&fb::Chunk<'_>> for Chunk {
//     fn from(chunk: &fb::Chunk) -> Self {
//         Self {
//             offset: chunk.offset(),
//             size: chunk.size_(),
//             num_rows: chunk.num_rows(),
//             encoding: match chunk.encoding_type() {
//                 fb::DictionaryEncoding::NoDictionary => DictionaryEncoding::NoDictionary,
//                 fb::DictionaryEncoding::LocalDictionary => DictionaryEncoding::Dictionary(
//                     chunk
//                         .encoding_as_local_dictionary()
//                         .unwrap()
//                         .dictionary_encblock_idxs()
//                         .unwrap()
//                         .into_iter()
//                         .map(|x| x as u32)
//                         .collect(),
//                 ),
//                 fb::DictionaryEncoding::SharedDictionary => DictionaryEncoding::SharedDictionary(
//                     chunk
//                         .encoding_as_shared_dictionary()
//                         .unwrap()
//                         .column_chunks()
//                         .unwrap()
//                         .into_iter()
//                         .map(|x| Chunk::from(&x))
//                         .collect(),
//                 ),
//                 _ => DictionaryEncoding::NoDictionary,
//             },
//             blocks: chunk
//                 .blocks()
//                 .into_iter()
//                 .flatten()
//                 .map(|x| NullableBlock::from(&x))
//                 .collect(),
//         }
//     }
// }

impl Chunk {
    pub(crate) fn new(
        offset: u64,
        size: u32,
        num_rows: u64,
        encoding: DictionaryEncoding,
        blocks: Vec<EncUnit>,
        checksum: Option<u64>,
    ) -> Self {
        Self {
            offset,
            size,
            num_rows,
            encoding,
            blocks,
            checksum,
        }
    }

    pub fn offset(&self) -> u64 {
        self.offset
    }
}

impl ToFlatBuffer for Chunk {
    type Target<'a> = fb::Chunk<'a>;

    fn to_fb<'fb>(&self, fbb: &mut FlatBufferBuilder<'fb>) -> WIPOffset<Self::Target<'fb>> {
        let encunits = {
            let blocks = self.blocks.iter().map(|y| y.to_fb(fbb)).collect::<Vec<_>>();
            let res = fbb.create_vector(&blocks);
            Some(res)
        };
        let (encoding_type, encoding) = match &self.encoding {
            DictionaryEncoding::NoDictionary => (
                fb::DictionaryEncoding::NoDictionary,
                Some(fb::NoDictionary::create(fbb, &fb::NoDictionaryArgs {}).as_union_value()),
            ),
            DictionaryEncoding::Dictionary(idxs) => {
                let dictionary_encunit_idxs = Some(fbb.create_vector(idxs));
                (
                    fb::DictionaryEncoding::LocalDictionary,
                    Some(
                        fb::LocalDictionary::create(
                            fbb,
                            &fb::LocalDictionaryArgs {
                                dictionary_encunit_idxs,
                            },
                        )
                        .as_union_value(),
                    ),
                )
            }
            DictionaryEncoding::SharedDictionary(shared_idx) => (
                fb::DictionaryEncoding::SharedDictionary,
                Some(
                    fb::SharedDictionary::create(
                        fbb,
                        &fb::SharedDictionaryArgs {
                            shared_dictionary_idx: *shared_idx,
                        },
                    )
                    .as_union_value(),
                ),
            ),
        };
        fb::Chunk::create(
            fbb,
            &fb::ChunkArgs {
                offset: self.offset,
                size_: self.size,
                num_rows: self.num_rows,
                encoding_type,
                encoding,
                encunits,
                checksum: self.checksum,
            },
        )
    }
}

/// WASMEncoding for writer to use.
/// Reader should use fff_format::File::fff::flatbuf::WASMEncoding directly.
#[derive(Clone)]
pub struct WASMEncoding {
    wasm_id: u32,
    mini_encunit_sizes: Vec<u32>,
}

impl WASMEncoding {
    pub fn new(wasm_id: u32, mini_encunit_sizes: Vec<u32>) -> Self {
        Self {
            wasm_id,
            mini_encunit_sizes,
        }
    }
}

impl From<&fb::WASMEncoding<'_>> for WASMEncoding {
    fn from(fb: &fb::WASMEncoding) -> Self {
        Self {
            wasm_id: fb.wasm_id(),
            mini_encunit_sizes: fb.mini_encunit_sizes().unwrap().into_iter().collect(),
        }
    }
}

impl ToFlatBuffer for WASMEncoding {
    type Target<'a> = fb::WASMEncoding<'a>;

    fn to_fb<'fb>(&self, fbb: &mut FlatBufferBuilder<'fb>) -> WIPOffset<Self::Target<'fb>> {
        let mini_encunit_sizes = fbb.create_vector(&self.mini_encunit_sizes);
        fb::WASMEncoding::create(
            fbb,
            &fb::WASMEncodingArgs {
                wasm_id: self.wasm_id,
                mini_encunit_sizes: Some(mini_encunit_sizes),
            },
        )
    }
}

/// Encoding for writer to use.
/// Reader should use fff_format::File::fff::flatbuf::Encoding directly.
#[derive(Clone)]
pub struct Encoding {
    encoding_type: fb::EncodingType,
    /// WASM binary location and minipage sizes
    wasm_encoding: Option<WASMEncoding>,
}

// impl From<fff_encoding::enc_unit::Encoding> for Encoding {
//     fn from(encoding: fff_encoding::enc_unit::Encoding) -> Self {
//         let encoding_type = match encoding {
//             fff_encoding::enc_unit::Encoding::Plain => fb::EncodingType::PLAIN,
//             fff_encoding::enc_unit::Encoding::Nullable => fb::EncodingType::NULLABLE,
//             _ => fb::EncodingType::CASCADE,
//             // TODO: WASM
//         };
//         Self {
//             encoding_type,
//             wasm_encoding: None,
//         }
//     }
// }

impl Default for Encoding {
    fn default() -> Self {
        Self {
            encoding_type: fb::EncodingType::CASCADE,
            wasm_encoding: None,
        }
    }
}

impl From<&fb::Encoding<'_>> for Encoding {
    fn from(fb: &fb::Encoding) -> Self {
        Self {
            encoding_type: fb.type_(),
            wasm_encoding: fb
                .wasm_encoding()
                .map(|fb_wasm_encoding| WASMEncoding::from(&fb_wasm_encoding)),
        }
    }
}

impl Encoding {
    pub fn try_new(
        encoding_type: fb::EncodingType,
        wasm_encoding: Option<WASMEncoding>,
    ) -> Result<Self> {
        Ok(Self {
            encoding_type,
            wasm_encoding,
        })
    }

    pub fn encoding_type(&self) -> fb::EncodingType {
        self.encoding_type
    }

    pub fn wasm_encoding(&self) -> Option<&WASMEncoding> {
        self.wasm_encoding.as_ref()
    }
}

impl ToFlatBuffer for Encoding {
    type Target<'a> = fb::Encoding<'a>;

    fn to_fb<'fb>(&self, fbb: &mut FlatBufferBuilder<'fb>) -> WIPOffset<Self::Target<'fb>> {
        let wasm_encoding = self
            .wasm_encoding
            .as_ref()
            .map(|wasm_encoding| wasm_encoding.to_fb(fbb));
        fb::Encoding::create(
            fbb,
            &fb::EncodingArgs {
                type_: self.encoding_type,
                wasm_encoding,
            },
        )
    }
}

/// EncUnit metadata for writer.
/// Reader should use [EncUnitFBS](fff_format::File::fff::flatbuf::EncUnit) directly.
#[derive(Default, Clone)]
pub struct EncUnit {
    size: u32,
    num_rows: u32,
    encoding: Encoding,
    compression: fb::CompressionType,
}

// impl From<&fb::EncBlock<'_>> for EncBlock {
//     fn from(page: &fb::EncBlock) -> Self {
//         Self {
//             size: page.size_(),
//             encoding: Encoding::from(&page.encoding().unwrap()),
//         }
//     }
// }

impl EncUnit {
    pub fn new(
        size: u32,
        num_rows: u32,
        encoding: Encoding,
        compression: fb::CompressionType,
    ) -> Self {
        Self {
            size,
            num_rows,
            encoding,
            compression,
        }
    }
}

impl ToFlatBuffer for EncUnit {
    type Target<'a> = fb::EncUnit<'a>;

    fn to_fb<'fb>(&self, fbb: &mut FlatBufferBuilder<'fb>) -> WIPOffset<Self::Target<'fb>> {
        let encoding = self.encoding.to_fb(fbb);
        fb::EncUnit::create(
            fbb,
            &fb::EncUnitArgs {
                size_: self.size,
                num_rows: self.num_rows,
                encoding: Some(encoding),
                compression: self.compression,
            },
        )
    }
}

/// Metadata section pointer for writer.
/// Reader should use [MetadataSectionFBS](fff_format::File::fff::flatbuf::MetadataSection) directly.
#[derive(Clone)]
pub struct MetadataSection {
    pub offset: u64,
    pub size: u32,
    pub compression_type: fb::CompressionType,
}

impl From<&fb::MetadataSection<'_>> for MetadataSection {
    fn from(metadata_sec: &fb::MetadataSection) -> Self {
        Self {
            offset: metadata_sec.offset(),
            size: metadata_sec.size_(),
            compression_type: metadata_sec.compression_type(),
        }
    }
}

impl ToFlatBuffer for MetadataSection {
    type Target<'a> = fb::MetadataSection<'a>;

    fn to_fb<'fb>(&self, fbb: &mut FlatBufferBuilder<'fb>) -> WIPOffset<Self::Target<'fb>> {
        fb::MetadataSection::create(
            fbb,
            &fb::MetadataSectionArgs {
                offset: self.offset,
                size_: self.size,
                compression_type: self.compression_type,
            },
        )
    }
}

/// Row group metadata storing indirect column metadata sections for writer.
/// Reader should use [RowGroupMetadataFBS](fff_format::File::fff::flatbuf::RowGroupMetadata) directly.
#[derive(Default)]
pub struct IndirectRowGroupMetadata {
    col_metadatas: Vec<MetadataSection>,
}

impl IndirectRowGroupMetadata {
    pub fn add_col_meta(&mut self, col_meta: MetadataSection) {
        self.col_metadatas.push(col_meta);
    }
}

/// To be used at writing time.
#[derive(Default)]
pub struct RowGroupMetadata {
    col_metadatas: Vec<ColumnMetadata>,
}

impl RowGroupMetadata {
    pub fn new(metadata_sec: Vec<ColumnMetadata>) -> Self {
        Self {
            col_metadatas: metadata_sec,
        }
    }

    pub fn col_metadatas(&self) -> &[ColumnMetadata] {
        &self.col_metadatas
    }
}

impl From<&fb::RowGroupMetadata<'_>> for IndirectRowGroupMetadata {
    fn from(row_group_metadata: &fb::RowGroupMetadata) -> Self {
        Self {
            col_metadatas: row_group_metadata
                .col_metadatas()
                .into_iter()
                .flatten()
                .map(|x| MetadataSection::from(&x))
                .collect(),
        }
    }
}

impl IndirectRowGroupMetadata {
    pub fn new(metadata_sec: Vec<MetadataSection>) -> Self {
        Self {
            col_metadatas: metadata_sec,
        }
    }
}

impl ToFlatBuffer for IndirectRowGroupMetadata {
    type Target<'a> = fb::RowGroupMetadata<'a>;

    fn to_fb<'fb>(&self, fbb: &mut FlatBufferBuilder<'fb>) -> WIPOffset<Self::Target<'fb>> {
        let col_metadatas = self
            .col_metadatas
            .iter()
            .map(|x: &MetadataSection| x.to_fb(fbb))
            .collect::<Vec<_>>();
        let col_metadatas = fbb.create_vector(&col_metadatas);
        fb::RowGroupMetadata::create(
            fbb,
            &fb::RowGroupMetadataArgs {
                col_metadatas: Some(col_metadatas),
            },
        )
    }
}

/// To be used at writing time only.
#[derive(Default)]
pub struct RowGroupsTable {
    row_counts: Vec<u32>,
    offsets: Vec<u64>,
    sizes: Vec<u32>,
    indirect_row_group_metadata: Vec<IndirectRowGroupMetadata>,
    row_group_metadata: Vec<RowGroupMetadata>,
}

impl RowGroupsTable {
    pub fn add_meta(
        &mut self,
        row_count: u32,
        offset: u64,
        size: u32,
        row_group_metadata: RowGroupMetadata,
    ) {
        self.row_counts.push(row_count);
        self.offsets.push(offset);
        self.sizes.push(size);
        self.row_group_metadata.push(row_group_metadata);
    }

    pub fn row_counts(&self) -> &[u32] {
        &self.row_counts
    }

    pub fn offsets(&self) -> &[u64] {
        &self.offsets
    }

    pub fn sizes(&self) -> &[u32] {
        &self.sizes
    }

    pub fn indirect_row_group_metadata(&self) -> &[IndirectRowGroupMetadata] {
        &self.indirect_row_group_metadata
    }

    /// Write ColumnMetadata as FBS to file and update indirect_row_group_metadata
    /// Returns the start offset of the very first ColumnMetadata
    pub fn to_indirect_and_flush<W: Write + Seek>(
        &mut self,
        writer: &mut W,
        checksum: &mut dyn Checksum,
    ) -> Result<u64> {
        let start_offset = writer.stream_position()?;
        for row_group in &self.row_group_metadata {
            let mut indirect_row_group_metadata = IndirectRowGroupMetadata::default();
            for col_meta in row_group.col_metadatas() {
                let mut fbb = FlatBufferBuilder::new();
                let fbs = col_meta.to_fb(&mut fbb);
                fbb.finish(fbs, None);
                let data = fbb.finished_data();
                let offset = writer.stream_position()?;
                writer.write_all(data)?;
                checksum.update(data);
                let size = data.len() as u32;
                indirect_row_group_metadata.add_col_meta(MetadataSection {
                    offset,
                    size,
                    compression_type: fb::CompressionType::Uncompressed,
                });
            }
            self.indirect_row_group_metadata
                .push(indirect_row_group_metadata);
        }
        Ok(start_offset)
    }
}

/// Footer for reader to use. Basically group all the column metadata flatbuffers
#[derive(Clone)]
pub struct Footer<'a> {
    schema: SchemaRef,
    // row_groups_fbs: fb::RowGroups<'a>,
    row_group_metadatas: Vec<GroupedColumnMetadata<'a>>,
    // TODO: statistics
}
/// Grouped according to row group. Each fb::ColumnMetadata is a separate root flatbuffer.
/// Offset and size are indicated by MetadataSection.
#[derive(Clone)]
pub struct GroupedColumnMetadata<'a> {
    pub column_metadatas: Vec<fb::ColumnMetadata<'a>>,
    pub row_count: u32,
    pub _offset: u64,
    pub _size: u32,
}

impl<'a> Footer<'a> {
    /// With projection, this function requires each column metadata's buffer is read ahead.
    pub(crate) fn try_new_with_projection(
        row_group_cnt_n_pointers: &[RowGroupCntNPointer],
        grouped_column_metadata_bufs: Vec<Vec<&'a [u8]>>,
        schema: SchemaRef,
    ) -> Result<Self> {
        let row_group_metadata = grouped_column_metadata_bufs
            .iter()
            .zip(row_group_cnt_n_pointers)
            .map(
                |(row_group_projected_col_bufs, row_group_cnt_n_pointer)| -> GroupedColumnMetadata {
                    let column_metadatas: Vec<fb::ColumnMetadata> = row_group_projected_col_bufs
                        .iter()
                        .map(|buf| {
                            flatbuffers::root::<fff_format::File::fff::flatbuf::ColumnMetadata>(buf)
                                .unwrap()
                        })
                        .collect();
                    GroupedColumnMetadata {
                        column_metadatas,
                        row_count: row_group_cnt_n_pointer.row_count,
                        _offset: row_group_cnt_n_pointer._offset,
                        _size: row_group_cnt_n_pointer._size,
                    }
                },
            )
            .collect();
        Ok(Self {
            schema,
            row_group_metadatas: row_group_metadata,
        })
    }
    /// This function reads the whole footer from the file, without column projection.
    /// buf is the preallocated buffer according to postscript
    pub fn try_new(buf: &'a [u8], file_size: usize, post_script: &PostScript) -> Result<Self> {
        let data_size = file_size - POSTSCRIPT_SIZE as usize - post_script.metadata_size as usize;
        let footer_fbs =
            root_as_footer(&buf[(post_script.metadata_size - post_script.footer_size) as usize..])
                .map_err(|e| Error::ParseError(format!("Unable to get root as footer: {e:?}")))?;
        // FIXME: use logical tree to know which logical encoding to use.
        let (schema, _logical_tree, row_groups_pointer, _shared_dict, _, _) =
            parse_footer(&footer_fbs)?;
        let row_group_metadata_fbs = row_groups_pointer
            .row_group_metadatas()
            .ok_or_else(|| Error::ParseError("Row group metadatas not found".to_string()))?;
        let row_counts = row_groups_pointer
            .row_counts()
            .ok_or_else(|| Error::ParseError("Row counts not found".to_string()))?;
        let offsets = row_groups_pointer
            .offsets()
            .ok_or_else(|| Error::ParseError("Offsets not found".to_string()))?;
        let sizes = row_groups_pointer
            .sizes()
            .ok_or_else(|| Error::ParseError("Sizes not found".to_string()))?;
        let row_group_metadata =
            itertools::izip!(row_group_metadata_fbs, row_counts, offsets, sizes)
                .map(
                    |(row_group_meta_fbs, row_count, offset, size)| -> GroupedColumnMetadata {
                        let column_metadatas: Vec<fb::ColumnMetadata> = row_group_meta_fbs
                            .col_metadatas()
                            .ok_or_else(|| {
                                Error::ParseError("Column metadatas not found".to_string())
                            })
                            .unwrap()
                            .into_iter()
                            .map(|meta_section| {
                                flatbuffers::root::<fff_format::File::fff::flatbuf::ColumnMetadata>(
                                    &buf[meta_section.offset() as usize - data_size
                                        ..meta_section.offset() as usize - data_size
                                            + meta_section.size_() as usize],
                                )
                                .unwrap()
                            })
                            .collect();
                        GroupedColumnMetadata {
                            column_metadatas,
                            row_count,
                            _offset: offset,
                            _size: size,
                        }
                    },
                )
                .collect();
        // let row_groups = row_groups_pointer;
        Ok(Self {
            schema: schema.into(),
            // row_groups_fbs: row_groups,
            row_group_metadatas: row_group_metadata,
        })
    }

    pub fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    // pub fn row_groups(&self) -> &fb::RowGroups<'a> {
    //     &self.row_groups_fbs
    // }

    pub fn row_group_metadatas(&self) -> &[GroupedColumnMetadata<'a>] {
        &self.row_group_metadatas
    }
}

#[allow(clippy::type_complexity)]
pub fn parse_footer<'a>(
    footer_fbs: &fb::Footer<'a>,
) -> Result<(
    Schema,
    fb::LogicalTree<'a>,
    fb::RowGroups<'a>,
    Option<fb::SharedDictionaryTable<'a>>,
    Option<fb::OptionalMetadataSections<'a>>,
    Option<HashMap<fb::EncodingType, Version>>,
)> {
    let schema_bytes = footer_fbs
        .schema()
        .ok_or_else(|| Error::ParseError("Schema not found".to_string()))?;
    let message = root_as_message(schema_bytes.bytes())
        .map_err(|err| Error::ParseError(format!("Unable to get root as message: {err:?}")))?;
    let ipc_schema = message
        .header_as_schema()
        .ok_or_else(|| Error::ParseError("Unable to read IPC message as schema".to_string()))?;
    let schema = fb_to_schema(ipc_schema);
    let logical_tree = footer_fbs.logical_tree().unwrap();
    let shared_dict = footer_fbs.shared_dictionary_table();
    let row_groups_pointer = footer_fbs
        .row_groups()
        .ok_or_else(|| Error::ParseError("Row groups not found in footer".to_string()))?;

    // Parse encoding versions if present
    let encoding_versions = footer_fbs.encoding_versions().map(|versions| {
        let mut map = HashMap::new();
        for version in versions.iter() {
            map.insert(
                version.encoding_type(),
                Version::new(
                    version.version().major(),
                    version.version().minor(),
                    version.version().patch(),
                ),
            );
        }
        map
    });

    Ok((
        schema,
        logical_tree,
        row_groups_pointer,
        shared_dict,
        footer_fbs.optional_sections(),
        encoding_versions,
    ))
}
