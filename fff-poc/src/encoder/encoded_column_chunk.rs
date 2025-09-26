use bytes::Bytes;
use fff_format::File::fff::flatbuf::CompressionType;

use crate::file::footer;

/// Only used in `EncodedColumnChunk`
#[derive(Clone)]
pub struct SerializedEncUnit {
    bytes: Bytes,
    num_rows: u32,
    encoding: footer::Encoding,
    compression_type: CompressionType,
}

impl SerializedEncUnit {
    pub fn new(
        bytes: Bytes,
        num_rows: u32,
        encoding: footer::Encoding,
        compression_type: CompressionType,
    ) -> Self {
        Self {
            bytes,
            num_rows,
            encoding,
            compression_type,
        }
    }

    pub fn bytes(&self) -> Bytes {
        self.bytes.clone()
    }

    pub fn num_rows(&self) -> u32 {
        self.num_rows
    }

    pub fn encoding(&self) -> &footer::Encoding {
        &self.encoding
    }

    pub fn compression_type(&self) -> CompressionType {
        self.compression_type
    }
}

/// An encoded ColumnChunk, serves as an IO unit.
/// Analogy to footer::Chunk and fb::Chunk, but contains data.
pub struct EncodedColumnChunk {
    /// Each EncUnit is a buffer of encoded data, fully serialized.
    pub encunits: Vec<SerializedEncUnit>,
    pub num_rows: usize,
    pub dict_encoding: footer::DictionaryEncoding,
    /// The physical column index
    pub column_index: u32,
}

impl Default for EncodedColumnChunk {
    fn default() -> Self {
        Self::builder().build()
    }
}

impl EncodedColumnChunk {
    pub fn builder() -> EncodedColumnChunkBuilder {
        EncodedColumnChunkBuilder::with_defaults()
    }
}

#[derive(Default)]
pub struct EncodedColumnChunkBuilder {
    pub encunits: Vec<SerializedEncUnit>,
    pub num_rows: usize,
    pub dict_encoding: footer::DictionaryEncoding,
    /// The physical column index
    pub column_index: u32,
}

impl EncodedColumnChunkBuilder {
    /// Returns default state of the builder.
    fn with_defaults() -> Self {
        Self::default()
    }

    /// Finalizes the configuration and returns immutable writer properties struct.
    pub fn build(self) -> EncodedColumnChunk {
        EncodedColumnChunk {
            encunits: self.encunits,
            num_rows: self.num_rows,
            dict_encoding: self.dict_encoding,
            column_index: self.column_index,
        }
    }

    pub fn set_dict_encoding(mut self, dict_encoding: footer::DictionaryEncoding) -> Self {
        self.dict_encoding = dict_encoding;
        self
    }
}

impl EncodedColumnChunk {
    pub fn update_column_index(self, column_index: u32) -> Self {
        Self {
            column_index,
            ..self
        }
    }
}
