use std::collections::HashMap;

use arrow_schema::DataType;
use fff_format::File::fff::flatbuf::CompressionType;

pub use crate::dict::DictionaryTypeOptions;
use crate::{
    common::checksum::ChecksumType,
    context::{WASMId, WASMWritingContext, WasmLib},
};

pub const DEFAULT_IOUNIT_SIZE: u64 = 8 * 1024 * 1024; // in bytes
pub const DEFAULT_ENCODING_UNIT_LEN: u64 = 64 * 1024; // in number of rows
pub const DEFAULT_CHECKSUM_TYPE: ChecksumType = ChecksumType::XxHash;

#[derive(Clone)]
pub struct FileWriterOptions {
    /// The size of an IOUnit in bytes. 8MB by default.
    iounit_size: u64,
    /// The length of an encoding unit in dictionary. 64Ki rows by default.
    encoding_unit_len: u64,
    /// The type of the checksum for data and schema. xxhash by defalt.
    checksum_type: ChecksumType,
    /// Always set the encoding of EncUnit metadata tobe CUSTOM_WASM. Write built-in Wasm to the file.
    /// In the meantime, disallow extension Wasms.
    write_built_in_wasm: bool,
    /// Mapping between root-level column id and custom encunit len (num of rows)
    custom_encunit_len: HashMap<usize, usize>,
    /// The size of a row group in number of rows. Infinite by default.
    row_group_size: u64,
    /// Custom encoding options, include the encoder dylib and decoder wasm lib
    /// FIXME: cannot be used together with write_built_in_wasm
    custom_encoding_options: CustomEncodingOptions,
    /// The type of dictionary to use
    dictionary_type: DictionaryTypeOptions,
    /// Enable per-IOUnit checksum
    enable_io_unit_checksum: bool,
    /// The type of compression to use for EncUnits
    compression_type: CompressionType,
}

impl Default for FileWriterOptions {
    fn default() -> Self {
        Self::builder().build()
    }
}

impl FileWriterOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn builder() -> FileWriterOptionsBuilder {
        FileWriterOptionsBuilder::with_defaults()
    }

    pub fn iounit_size(&self) -> u64 {
        self.iounit_size
    }

    pub fn encoding_unit_len(&self) -> u64 {
        self.encoding_unit_len
    }

    pub fn checksum_type(&self) -> ChecksumType {
        self.checksum_type
    }

    pub fn write_built_in_wasm(&self) -> bool {
        self.write_built_in_wasm
    }

    pub fn custom_encunit_len(&self) -> &HashMap<usize, usize> {
        &self.custom_encunit_len
    }

    pub fn row_group_size(&self) -> u64 {
        self.row_group_size
    }

    pub fn custom_encoding_options(&self) -> &CustomEncodingOptions {
        &self.custom_encoding_options
    }

    pub fn take_custom_encoding_options(&mut self) -> CustomEncodingOptions {
        std::mem::take(&mut self.custom_encoding_options)
    }

    pub fn dictionary_type(&self) -> DictionaryTypeOptions {
        self.dictionary_type
    }

    pub fn enable_io_unit_checksum(&self) -> bool {
        self.enable_io_unit_checksum
    }

    pub fn compression_type(&self) -> CompressionType {
        self.compression_type
    }
}

pub struct FileWriterOptionsBuilder {
    /// The size of an IOUnit in bytes. 8MB by default.
    iounit_size: u64,
    /// The length of an encoding unit in dictionary. 64Ki rows by default.
    encoding_unit_len: u64,
    /// The type of the checksum for data and schema. xxhash by defalt.
    checksum_type: ChecksumType,
    /// Always set the encoding of EncUnit metadata to be CUSTOM_WASM. Write built-in Wasm to the file.
    /// In the meantime, disallow extension Wasms.
    write_built_in_wasm: bool,
    /// Mapping between root-level column id and custom encunit len (num of rows)
    /// TODO: not correctly implement yet. Check FileWriter::write_batch
    custom_encunit_len: HashMap<usize, usize>,
    /// The size of a row group in number of rows. Infinite by default.
    /// This is a threshold. E.g., if row_group_size is 1000 and we already wrote 900 rows,
    /// and then we write a batch of 200 rows, the row group will be 1100 rows.
    /// Check FileWriter::write_batch
    row_group_size: u64,
    /// Custom encoding options, include the encoder dylib and decoder wasm lib
    /// FIXME: cannot be used together with write_built_in_wasm
    custom_encoding_options: CustomEncodingOptions,
    dictionary_type: DictionaryTypeOptions,
    /// Enable per-IOUnit checksum
    enable_io_unit_checksum: bool,
    /// The type of compression to use for EncUnits
    compression_type: CompressionType,
}

impl FileWriterOptionsBuilder {
    /// Returns default state of the builder.
    pub fn with_defaults() -> Self {
        Self {
            iounit_size: DEFAULT_IOUNIT_SIZE,
            encoding_unit_len: DEFAULT_ENCODING_UNIT_LEN,
            checksum_type: DEFAULT_CHECKSUM_TYPE,
            write_built_in_wasm: false,
            custom_encunit_len: Default::default(),
            row_group_size: u64::MAX, // By default, only one row group per file.
            custom_encoding_options: Default::default(),
            dictionary_type: DictionaryTypeOptions::EncoderDictionary,
            enable_io_unit_checksum: false,
            compression_type: CompressionType::Uncompressed,
        }
    }

    /// Finalizes the configuration and returns immutable writer properties struct.
    pub fn build(self) -> FileWriterOptions {
        // TODO: better way of separting built-in wasm and custom extension wasm
        assert!(!self.write_built_in_wasm || self.custom_encoding_options.is_empty());
        FileWriterOptions {
            iounit_size: self.iounit_size,
            encoding_unit_len: self.encoding_unit_len,
            checksum_type: self.checksum_type,
            write_built_in_wasm: self.write_built_in_wasm,
            custom_encunit_len: self.custom_encunit_len,
            row_group_size: self.row_group_size,
            custom_encoding_options: self.custom_encoding_options,
            dictionary_type: self.dictionary_type,
            enable_io_unit_checksum: self.enable_io_unit_checksum,
            compression_type: self.compression_type,
        }
    }

    pub fn set_iounit_size(mut self, iounit_size: u64) -> Self {
        self.iounit_size = iounit_size;
        self
    }

    pub fn set_encoding_unit_len(mut self, encoding_unit_len: u64) -> Self {
        self.encoding_unit_len = encoding_unit_len;
        self
    }

    pub fn set_checksum_type(mut self, checksum_type: ChecksumType) -> Self {
        self.checksum_type = checksum_type;
        self
    }

    pub fn write_built_in_wasm(mut self, write_built_in_wasm: bool) -> Self {
        self.write_built_in_wasm = write_built_in_wasm;
        self
    }

    pub fn set_custom_encunit_len(mut self, custom_encunit_len: HashMap<usize, usize>) -> Self {
        self.custom_encunit_len = custom_encunit_len;
        self
    }

    pub fn set_row_group_size(mut self, row_group_size: u64) -> Self {
        self.row_group_size = row_group_size;
        self
    }

    pub fn set_custom_encoding_options(
        mut self,
        custom_encoding_options: CustomEncodingOptions,
    ) -> Self {
        self.custom_encoding_options = custom_encoding_options;
        self
    }

    pub fn set_dictionary_type(mut self, dictionary_type: DictionaryTypeOptions) -> Self {
        self.dictionary_type = dictionary_type;
        self
    }

    pub fn enable_io_unit_checksum(mut self, enable_io_unit_checksum: bool) -> Self {
        self.enable_io_unit_checksum = enable_io_unit_checksum;
        self
    }

    pub fn set_compression_type(mut self, compression_type: CompressionType) -> Self {
        self.compression_type = compression_type;
        self
    }
}

#[derive(Clone, Default)]
pub struct CustomEncodingOptions {
    /// mapping between DataType and (path to encoding lib, path to wasm decoding lib)
    /// WASMId to its binaries
    wasms: HashMap<WASMId, WasmLib>,
    /// DataType to its WASMId
    data_type_to_wasm_id: HashMap<DataType, WASMId>,
}

impl CustomEncodingOptions {
    pub fn new(
        wasms: HashMap<WASMId, WasmLib>,
        data_type_to_wasm_id: HashMap<DataType, WASMId>,
    ) -> Self {
        Self {
            wasms,
            data_type_to_wasm_id,
        }
    }

    pub fn len(&self) -> usize {
        self.wasms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.wasms.len() == 0
    }

    pub fn into_context(self) -> WASMWritingContext {
        WASMWritingContext::with_custom_wasms(self.wasms, self.data_type_to_wasm_id)
    }
}
