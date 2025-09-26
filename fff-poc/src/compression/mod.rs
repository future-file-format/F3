/// Block compression is not recommended because it is both compute-heavy and hinder random access.
use bytes::Bytes;
use fff_core::errors::{Error, Result};
use fff_format::File::fff::flatbuf as fb;

/// Compress data based on the compression type
pub fn compress_data(data: Bytes, compression_type: fb::CompressionType) -> Result<Bytes> {
    match compression_type {
        fb::CompressionType::Uncompressed => Ok(data),
        fb::CompressionType::Lz4 => {
            let compressed = lz4_flex::compress_prepend_size(data.as_ref());
            Ok(Bytes::from(compressed))
        }
        fb::CompressionType::Zstd => {
            let compressed = zstd::stream::encode_all(data.as_ref(), 0)?;
            Ok(Bytes::from(compressed))
        }
        _ => Err(fff_core::errors::Error::General(
            "Unsupported compression type".to_string(),
        )),
    }
}

/// Decompress data based on the compression type
pub fn decompress_data(data: Bytes, compression_type: fb::CompressionType) -> Result<Bytes> {
    match compression_type {
        fb::CompressionType::Uncompressed => Ok(data),
        fb::CompressionType::Lz4 => Ok(Bytes::from(
            lz4_flex::decompress_size_prepended(data.as_ref())
                .map_err(|e| Error::External(Box::new(e)))?,
        )),
        fb::CompressionType::Zstd => Ok(Bytes::from(zstd::stream::decode_all(data.as_ref())?)),
        _ => Err(Error::General(format!(
            "Unsupported compression type: {:?}",
            compression_type
        ))),
    }
}
