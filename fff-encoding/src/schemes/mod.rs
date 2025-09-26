use std::{io::Cursor, rc::Rc};

use arrow_array::ArrayRef;
use arrow_buffer::Buffer;
use byteorder::{LittleEndian, ReadBytesExt};
use bytes::Bytes;
use fff_core::{errors::Result, nyi_err};
use rkyv::{Archive, Deserialize as rkyvDe, Serialize as rkyvSer};

use crate::enc_unit::{EncUnit, Encoding, ALIGNMENT};

pub mod bp;
pub mod vortex;

pub trait Encoder {
    fn encode(&self, arr: ArrayRef) -> Result<EncUnit>;
    fn encoding_type(&self) -> Encoding;
}

pub trait Decoder {
    /// Results may be multiple buffers according to logical types (null or not, varbinary, etc.)
    /// We use Arrow's buffer here since it is automatically aligned on 64B.
    fn decode_all(&mut self) -> Result<Vec<Buffer>> {
        nyi_err!("decode_all")
    }
    fn decode_all_as_array(&mut self) -> Result<ArrayRef> {
        nyi_err!("decode_all_as_array")
    }

    fn slice(&mut self, _start: usize, _stop: usize) -> Result<ArrayRef> {
        nyi_err!("slice")
    }

    fn decode_a_vector(&mut self) -> Result<Option<Vec<Buffer>>> {
        nyi_err!("decode_a_vector")
    }
}

/// A unified layout for non-null encodings.
/// | metadata_size: u32 | mini_block_offsets: [uint32] | OptionalMetadata | padding to align | data: [ubyte] |
#[derive(Archive, rkyvDe, rkyvSer, Debug, PartialEq)]
#[archive(
    // This will generate a PartialEq impl between our unarchived and archived
    // types:
    compare(PartialEq),
    // bytecheck can be used to validate your data if you want. To use the safe
    // API, you have to derive CheckBytes for the archived type:
    check_bytes,
)]
// Derives can be passed through to the generated type:
#[archive_attr(derive(Debug))]
pub(crate) struct EncUnitMetadata {
    num_values: u32,
    mini_blocks_offsets: Vec<u32>,
    metadata: Option<Vec<u8>>,
}

pub fn encode_to_bytes(encoder: Rc<dyn Encoder>, arr: ArrayRef) -> Bytes {
    let encblock = encoder.encode(arr).unwrap();
    let mut buffer = Vec::new();
    let mut cursor = Cursor::new(&mut buffer);
    encblock.try_serialize(&mut cursor).unwrap();
    buffer.into()
}
pub(crate) struct NonNullDecoderState {
    _vector_index: usize,
    metadata_bytes: Bytes,
    data: Bytes,
}

/// TODO: Move metadata to the root node of cascade encoding or simply remove it.
impl NonNullDecoderState {
    /// We need to ensure the input encblock is aligned on 64B.
    pub fn new(mut encblock: Bytes) -> Self {
        let metadata_size = encblock
            .split_to(4)
            .as_ref()
            .read_u32::<LittleEndian>()
            .unwrap();
        let metadata = encblock.split_to(metadata_size as usize);
        let _padding = encblock
            .split_to((4 + metadata.len()).next_multiple_of(ALIGNMENT) - (4 + metadata.len()));
        Self {
            _vector_index: 1,
            metadata_bytes: metadata,
            data: encblock,
        }
    }
    pub(crate) fn metadata(&self) -> &ArchivedEncUnitMetadata {
        // unsafe { rkyv::archived_root::<EncUnitMetadata>(&self.metadata_bytes) }
        rkyv::check_archived_root::<EncUnitMetadata>(&self.metadata_bytes).unwrap()
    }
}
