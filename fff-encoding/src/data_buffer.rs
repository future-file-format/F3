use bytes::Bytes;
use fff_core::errors::{Error, Result};

/// At writing time, a node of encoding tree may contain multiple `Bytes`,
/// e.g., Plain encoding contains the buffer of [metadata_size, metadata, padding, data]. namely Sparse Buffer.
/// they will be written together as a single Dense Buffer during writing.
/// At decoding time, they are deserialized into a single Dense `Bytes`.
pub enum DataBuffer {
    Sparse(Vec<Bytes>),
    Dense(Bytes),
}

impl From<Bytes> for DataBuffer {
    fn from(bytes: Bytes) -> Self {
        DataBuffer::Dense(bytes)
    }
}

impl From<Vec<Bytes>> for DataBuffer {
    fn from(buffers: Vec<Bytes>) -> Self {
        DataBuffer::Sparse(buffers)
    }
}

pub(crate) struct DataBuffersIter<'a> {
    buffer: &'a DataBuffer,
    index: usize,
}

impl Iterator for DataBuffersIter<'_> {
    type Item = Bytes;

    fn next(&mut self) -> Option<Self::Item> {
        match self.buffer {
            DataBuffer::Sparse(vec) => {
                if self.index < vec.len() {
                    let item = vec[self.index].clone();
                    self.index += 1;
                    Some(item)
                } else {
                    None
                }
            }
            DataBuffer::Dense(bytes) => {
                if self.index == 0 {
                    self.index = 1;
                    Some(bytes.clone())
                } else {
                    None
                }
            }
        }
    }
}

impl DataBuffer {
    pub(crate) fn iter(&self) -> DataBuffersIter {
        DataBuffersIter {
            buffer: self,
            index: 0,
        }
    }

    pub fn try_to_dense(&self) -> Result<Bytes> {
        match self {
            DataBuffer::Dense(bytes) => Ok(bytes.clone()),
            DataBuffer::Sparse(buffers) => Err(Error::General(format!(
                "Cannot convert sparse buffer to dense. Found {} buffers",
                buffers.len()
            ))),
        }
    }
}
