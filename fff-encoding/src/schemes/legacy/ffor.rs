use arrow_array::ArrayRef;
use arrow_buffer::Buffer;

use crate::encblock::{EncBlockMetadata, MINIBLOCK_SIZE};
use crate::errors::{Error, Result};
use bytes::BytesMut;

use super::{EncBlockEncoder, EncodedBlock, Encoding};
use fastlanes::FoR;

pub struct FFOREncoder {}

impl FFOREncoder {
    fn primitive_encode(&self, arr: ArrayRef) -> Result<EncodedBlock> {
        debug_assert!(arr.to_data().buffers().len() == 1);
        let metadata = EncBlockMetadata {
            mini_blocks_offsets: {
                (0..arr.len())
                    .step_by(MINIBLOCK_SIZE)
                    .map(|i| -> u32 {
                        (i * arr.data_type().primitive_width().unwrap())
                            .try_into()
                            .unwrap()
                    })
                    .collect()
            },
            metadata: None,
        };
        let mut s = flexbuffers::FlexbufferSerializer::new();
        metadata.serialize(&mut s).unwrap();
        let metadata = s.take_buffer().into();
        let metadata_size = metadata.len() as u32;
        let metadata_size = Buffer::from(metadata_size.to_le_bytes().to_vec());
        let data = arr.to_data().buffers()[0].clone();
        let buffers = vec![metadata_size, metadata, data];
        Ok(EncodedBlock {
            buffers,
            encoding: Encoding::Plain,
        })
    }
}

impl EncBlockEncoder for FFOREncoder {
    fn encode(&self, arr: ArrayRef) -> Result<EncodedBlock> {
        match arr.data_type() {
            t if t.is_primitive() => self.primitive_encode(arr),
            _ => unimplemented!(),
        }
    }
    fn encoding_type(&self) -> Encoding {
        Encoding::Plain
    }
}

pub struct PlainDecoder {
    vector_index: usize,
    metadata: EncBlockMetadata,
    data: BytesMut,
}

impl PlainDecoder {
    pub fn new(mut encblock: BytesMut) -> Self {
        let metadata_size = encblock
            .split_to(4)
            .as_ref()
            .read_u32::<LittleEndian>()
            .unwrap();
        let metadata = encblock.split_to(metadata_size as usize);
        let r = flexbuffers::Reader::get_root(metadata.as_ref()).unwrap();
        Self {
            vector_index: 1,
            metadata: EncBlockMetadata::deserialize(r).unwrap(),
            data: encblock,
        }
    }
}

impl EncBlockDecoder for PlainDecoder {
    fn decode_all(&mut self) -> Result<Vec<BytesMut>> {
        Ok(vec![self.data.clone()])
    }

    fn decode_a_vector(&mut self) -> Result<Option<Vec<BytesMut>>> {
        let start = *self
            .metadata
            .mini_blocks_offsets
            .get(self.vector_index - 1)
            .ok_or_else(|| Error::General("invalid start miniblock offset".to_string()))?
            as usize;
        let end = self.metadata.mini_blocks_offsets.get(self.vector_index);
        match (end, self.vector_index) {
            (None, i) if i == self.metadata.mini_blocks_offsets.len() => {
                return Ok(Some(vec![BytesMut::from(&self.data[start..])]))
            }
            (None, _) => return Ok(None),
            (Some(size), _) => {
                let res = Some(vec![BytesMut::from(&self.data[start..*size as usize])]);
                self.vector_index += 1;
                Ok(res)
            }
        }
    }
}
