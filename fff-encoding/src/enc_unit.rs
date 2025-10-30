use super::data_buffer::DataBuffer;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use bytes::{Bytes, BytesMut};
use fff_core::errors::Result;
use fff_core::util::bit_util::padding_size;
use fff_format::File::fff::flatbuf as fb;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    io::{Seek, Write},
};

/// Analogy to footer::EncUnit and fb::EncUnit, but contains (non-serialized) data.
///
/// EncUnit (64k) is an encoded Arrow Array that should be physically stored together
/// Example: nullable int32 array's validity and data
/// Example: nullable String array's validity, offsets and data
///
/// EncUnit should allow fine-grained access whenever the encoding allows it. e.g., access 2k values
pub struct EncUnit {
    /// Leaf nodes contains the actual encoded data.
    buffers: Vec<Bytes>,
    /// Deprecated after using Vortex.
    _encoding: Encoding,
    /// Deprecated after using Vortex.
    /// Any non-leaf nodes must have children.
    _children: Vec<EncUnit>,
}

impl EncUnit {
    pub fn new(buffers: Vec<Bytes>, encoding: Encoding, children: Vec<EncUnit>) -> Self {
        Self {
            buffers,
            _encoding: encoding,
            _children: children,
        }
    }

    /// Deprecated after using Vortex.
    /// Flatten the EncUnit into data buffers, buffers offsets, and an encoding tree.
    fn _into_flat(self) -> FlatEncUnit {
        let mut buffers: Vec<DataBuffer> = vec![];
        let mut buffer_sizes = vec![];
        if !self.buffers.is_empty() {
            buffer_sizes.push(self.buffers.iter().map(|b| b.len() as u32).sum());
            buffers.push(self.buffers.into());
        }
        let mut encoding_tree = EncodingTree {
            root: self._encoding,
            children: vec![],
        };
        for child in self._children.into_iter() {
            let child_flat = child._into_flat();
            buffers.extend(child_flat.buffers);
            buffer_sizes.extend(child_flat.buffer_sizes.unwrap());
            encoding_tree.children.push(child_flat.encoding_tree);
        }
        FlatEncUnit {
            buffers,
            buffer_sizes: Some(buffer_sizes),
            encoding_tree,
        }
    }

    /// (Deprecated) Serialize the EncUnit into a byte buffer, via writting to `W`.
    /// Now directly write all the buffers.
    pub fn try_serialize<W: Write + Seek>(self, mut write: W) -> Result<W> {
        for buf in self.buffers.iter() {
            write.write_all(buf.as_ref())?;
        }
        Ok(write)
        // self.into_flat().serialize(write)
    }
}

#[derive(Copy, Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
pub enum Encoding {
    BP, // Deprecated after using Vortex.
    Vortex,
    /// User-provided dylib encoder
    Custom,
}

impl Encoding {
    pub fn to_fbs_encoding(&self) -> fb::EncodingType {
        match self {
            Encoding::Vortex => fb::EncodingType::CASCADE,
            Encoding::Custom => fb::EncodingType::CUSTOM_WASM,
            _ => unimplemented!(),
        }
    }
}

impl From<fb::EncodingType> for Encoding {
    fn from(encoding: fb::EncodingType) -> Self {
        match encoding {
            fb::EncodingType::CASCADE => Encoding::Vortex,
            fb::EncodingType::CUSTOM_WASM => Encoding::Custom,
            _ => unimplemented!(),
        }
    }
}

pub(crate) const MINIBLOCK_SIZE: usize = 1024;
pub(crate) const ALIGNMENT: usize = 4;
pub(crate) const ZEROS: [u8; 512] = [0u8; 512];

#[derive(Serialize, Deserialize)]
struct EncodingTree {
    root: Encoding,
    children: Vec<EncodingTree>,
}

pub struct FlatEncUnit {
    encoding_tree: EncodingTree,
    // For serialize, we need to know the size of each buffer.
    buffer_sizes: Option<Vec<u32>>,
    /// Contains buffers of each node
    buffers: Vec<DataBuffer>,
}

impl FlatEncUnit {
    pub fn buffers(&self) -> &[DataBuffer] {
        self.buffers.as_ref()
    }
    /// num_of_buffers: u32
    /// buffer_sizes: num_of_buffers * u32
    /// size of serialized encoding tree: u32
    /// serialized encoding tree
    /// padding to 4B alignment
    /// buffers: num_of_buffers * [u8]
    /// padding to 4B alignment -> this is to ensure the next EncUnit is aligned at 4B.
    pub fn serialize<W: Write + Seek>(&self, mut write: W) -> Result<W> {
        let start = write.stream_position()?;
        write
            .write_u32::<LittleEndian>(self.buffers.len() as u32)
            .unwrap();
        for size in self.buffer_sizes.as_ref().unwrap().iter() {
            write.write_u32::<LittleEndian>(*size)?;
        }
        // TODO: find the best serializer impl.
        let mut s = flexbuffers::FlexbufferSerializer::new();
        self.encoding_tree.serialize(&mut s).unwrap();
        let encoding_tree = s.take_buffer();
        write.write_u32::<LittleEndian>(encoding_tree.len() as u32)?;
        write.write_all(&encoding_tree)?;
        let written_len =
            4 + self.buffer_sizes.as_ref().unwrap().len() * 4 + 4 + encoding_tree.len();
        write.write_all(&ZEROS[..padding_size(written_len, ALIGNMENT)])?;
        for buf in self.buffers.iter() {
            for buffer in buf.iter() {
                write.write_all(buffer.as_ref())?;
            }
        }
        let written_len = write.stream_position()? - start;
        write.write_all(&ZEROS[..padding_size(written_len as usize, ALIGNMENT)])?;
        Ok(write)
    }

    pub fn try_deserialize(mut bytes: Bytes) -> Result<Self> {
        let num_buffers = bytes
            .split_to(4)
            .as_ref()
            .read_u32::<LittleEndian>()
            .unwrap();
        let buffer_sizes_bytes = bytes.split_to(num_buffers as usize * 4);
        let buffer_sizes: &[u32] = bytemuck::try_cast_slice(buffer_sizes_bytes.as_ref())?;
        let size_encoding_tree = bytes
            .split_to(4)
            .as_ref()
            .read_u32::<LittleEndian>()
            .unwrap();
        let encoding_tree = <EncodingTree as Deserialize>::deserialize(
            flexbuffers::Reader::get_root(bytes.split_to(size_encoding_tree as usize).as_ref())
                .unwrap(),
        )
        .unwrap();
        let written_len = 4 + size_encoding_tree as usize + 4 + num_buffers as usize * 4;
        let _padding = bytes.split_to(padding_size(written_len, ALIGNMENT));
        let mut buffers = vec![];
        for size in buffer_sizes.iter() {
            let buffer = bytes.split_to(*size as usize);
            buffers.push(buffer);
        }
        Ok(Self {
            encoding_tree,
            buffer_sizes: None,
            buffers: buffers.into_iter().map(DataBuffer::Dense).collect(),
        })
    }

    /// WARNING: This function is only for testing purposes.
    pub fn read_first_buffer(bytes: Bytes) -> Result<Bytes> {
        Self::try_deserialize(bytes).unwrap().buffers[0].try_to_dense()
    }

    /// No use for now. Written as an attempt to zero-copy Vortex decoding.
    pub fn try_deserialize_first_bytesmut(mut bytes: BytesMut) -> Result<BytesMut> {
        let num_buffers = bytes
            .split_to(4)
            .as_ref()
            .read_u32::<LittleEndian>()
            .unwrap();
        let buffer_sizes_bytes = bytes.split_to(num_buffers as usize * 4);
        let buffer_sizes: &[u32] = bytemuck::try_cast_slice(buffer_sizes_bytes.as_ref())?;
        let size_encoding_tree = bytes
            .split_to(4)
            .as_ref()
            .read_u32::<LittleEndian>()
            .unwrap();
        let written_len = 4 + size_encoding_tree as usize + 4 + num_buffers as usize * 4;
        let _padding = bytes.split_to(padding_size(written_len, ALIGNMENT));
        let mut buffers: VecDeque<BytesMut> = vec![].into();
        for size in buffer_sizes.iter() {
            let buffer = bytes.split_to(*size as usize);
            buffers.push_back(buffer);
        }
        Ok(buffers.pop_front().unwrap())
    }
}
