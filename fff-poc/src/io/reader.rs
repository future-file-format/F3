use bytes::Bytes;
use fff_core::errors::Result;
use futures::executor::block_on;
use lazy_static::lazy_static;
use object_store::path::Path;
use object_store::ObjectStore;
use parquet::file::reader::{ChunkReader, Length};
use std::io::Read;
use std::sync::{Arc, OnceLock};
use std::{fs::File, os::unix::fs::FileExt};

lazy_static! {
    static ref RUNTIME: tokio::runtime::Runtime = tokio::runtime::Runtime::new().unwrap();
}

/// Read Trait for abstraction over local files and S3.
pub trait Reader {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> Result<()>;
    fn size(&self) -> Result<u64>;
}

impl Reader for File {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> Result<()> {
        // println!("read size: {:?}", buf.len());
        FileExt::read_exact_at(self, buf, offset).map_err(Into::into)
    }

    fn size(&self) -> Result<u64> {
        File::metadata(self).map(|m| m.len()).map_err(Into::into)
    }
}

impl Reader for Arc<File> {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> Result<()> {
        Reader::read_exact_at(self.as_ref(), buf, offset)
    }

    fn size(&self) -> Result<u64> {
        Reader::size(self.as_ref())
    }
}

impl Reader for [u8] {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> Result<()> {
        buf.copy_from_slice(&self[offset as usize..(offset as usize + buf.len())]);
        Ok(())
    }

    fn size(&self) -> Result<u64> {
        Ok(self.len() as u64)
    }
}

#[derive(Clone)]
pub struct ObjectStoreReadAt {
    object_store: Arc<dyn ObjectStore>,
    location: Arc<Path>,
    /// CAUTION: here we have the assumption that the file size won't change accross read requests.
    /// This is simply to allow Parquet readers to have less overhead on multiple reads.
    cache_size: OnceLock<u64>,
}

impl ObjectStoreReadAt {
    pub fn new(object_store: Arc<dyn ObjectStore>, location: Arc<Path>) -> Self {
        Self {
            object_store,
            location,
            cache_size: OnceLock::new(),
        }
    }
}

impl Reader for ObjectStoreReadAt {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> Result<()> {
        // let start = std::time::Instant::now();
        let start_range = offset as usize;

        let object_store = Arc::clone(&self.object_store);
        let location = self.location.clone();
        let len = buf.len();
        let head_result = block_on(async move {
            RUNTIME
                .spawn(async move {
                    object_store
                        .get_range(&location, start_range..(start_range + len))
                        .await
                })
                .await
                .unwrap()
        });

        let bytes = head_result.map_err(fff_core::errors::Error::ObjectStore)?;
        buf.copy_from_slice(bytes.as_ref());
        // println!("read {:?}", start.elapsed());
        Ok(())
    }

    fn size(&self) -> Result<u64> {
        Ok(*self.cache_size.get_or_init(|| {
            // let start = std::time::Instant::now();
            let object_store = Arc::clone(&self.object_store);
            let location = self.location.clone();
            let head_result = block_on(async move {
                RUNTIME
                    .spawn(async move { object_store.head(&location).await })
                    .await
                    .unwrap()
            });
            // println!("size {:?}", start.elapsed());
            head_result
                .map_err(fff_core::errors::Error::ObjectStore)
                .map(|o| o.size as u64)
                .unwrap()
        }))
    }
}

impl Reader for Arc<ObjectStoreReadAt> {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> Result<()> {
        Reader::read_exact_at(self.as_ref(), buf, offset)
    }

    fn size(&self) -> Result<u64> {
        Reader::size(self.as_ref())
    }
}

impl Length for ObjectStoreReadAt {
    fn len(&self) -> u64 {
        self.size().unwrap()
    }
}

pub struct ObjectStoreRead {
    read_at: ObjectStoreReadAt,
    offset: usize,
}

impl Read for ObjectStoreRead {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        self.read_at.read_exact_at(buf, self.offset as u64).unwrap();
        self.offset += buf.len();
        Ok(buf.len())
    }
}

/// NOTE(Deprecated): This is no longer useful because it is sub-optimal to read Parquet.
/// We directly use async parquet reader now.
impl ChunkReader for ObjectStoreReadAt {
    type T = ObjectStoreRead;

    fn get_read(&self, start: u64) -> parquet::errors::Result<Self::T> {
        Ok(ObjectStoreRead {
            read_at: self.clone(),
            offset: start as usize,
        })
    }

    fn get_bytes(&self, start: u64, length: usize) -> parquet::errors::Result<Bytes> {
        // let t = std::time::Instant::now();
        let start_range = start as usize;

        let object_store = Arc::clone(&self.object_store);
        let location = self.location.clone();
        let head_result = block_on(async move {
            RUNTIME
                .spawn(async move {
                    object_store
                        .get_range(&location, start_range..(start_range + length))
                        .await
                })
                .await
                .unwrap()
        });
        // println!("pq random access {:?}", t.elapsed());

        head_result.map_err(|err| parquet::errors::ParquetError::External(err.into()))
    }
}
