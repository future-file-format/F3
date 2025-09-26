use arrow_array::ArrayRef;
use arrow_buffer::Buffer;
use arrow_schema::DataType;
use bytes::Bytes;

use crate::enc_unit::MINIBLOCK_SIZE;
use crate::schemes::ALIGNMENT;
use fff_core::util::bit_util::ceil;

use super::{Decoder, EncUnitMetadata, Encoder, Encoding, NonNullDecoderState};

use super::EncUnit;
use fff_core::errors::{Error, Result};

/// PlainEncoding means zero-copy to Arrow. For example, validity is stored as bitmap, the same as Arrow.
/// Leaf nodes.
pub struct PlainEncoder;

impl PlainEncoder {
    fn primitive_encode(&self, arr: ArrayRef) -> Result<EncUnit> {
        debug_assert!(arr.to_data().buffers().len() == 1);
        let primitive_width = match arr.data_type() {
            DataType::List(_) => 32,
            DataType::LargeList(_) => 64,
            _ => arr.data_type().primitive_width().unwrap(),
        };
        let metadata = EncUnitMetadata {
            num_values: arr.len() as u32,
            mini_blocks_offsets: {
                (0..arr.len())
                    .step_by(MINIBLOCK_SIZE)
                    .map(|i| -> u32 { (i * primitive_width).try_into().unwrap() })
                    .collect()
            },
            metadata: None,
        };
        let metadata = Bytes::from(
            rkyv::to_bytes::<_, 256>(&metadata)
                .unwrap()
                .into_boxed_slice(),
        );
        let metadata_size = metadata.len() as u32;
        let metadata_size = Bytes::from(metadata_size.to_le_bytes().to_vec());
        let data = arr.to_data().buffers()[0].to_vec().into(); // TODO: avoid copy?
        let padding = Bytes::from(vec![
            0u8;
            (metadata_size.len() + metadata.len())
                .next_multiple_of(ALIGNMENT)
                - (metadata_size.len() + metadata.len())
        ]);
        let buffers = vec![metadata_size, metadata, padding, data];
        Ok(EncUnit::new(buffers, Encoding::Plain, vec![]))
    }

    fn boolean_encode(&self, arr: ArrayRef) -> Result<EncUnit> {
        debug_assert!(arr.to_data().buffers().len() == 1);
        let metadata = EncUnitMetadata {
            num_values: arr.len() as u32,
            mini_blocks_offsets: {
                (0..arr.len())
                    .step_by(MINIBLOCK_SIZE)
                    .map(|i| -> u32 { ceil(i, 8) as u32 })
                    .collect()
            },
            metadata: None,
        };
        let metadata = Bytes::from(
            rkyv::to_bytes::<_, 256>(&metadata)
                .unwrap()
                .into_boxed_slice(),
        );
        let metadata_size = metadata.len() as u32;
        let metadata_size = Bytes::from(metadata_size.to_le_bytes().to_vec());
        let data = arr.to_data().buffers()[0].to_vec().into(); // TODO: avoid copy?
        let padding = Bytes::from(vec![
            0u8;
            (metadata_size.len() + metadata.len())
                .next_multiple_of(ALIGNMENT)
                - (metadata_size.len() + metadata.len())
        ]);
        let buffers = vec![metadata_size, metadata, padding, data];
        Ok(EncUnit::new(buffers, Encoding::Plain, vec![]))
    }
}

impl Encoder for PlainEncoder {
    fn encode(&self, arr: ArrayRef) -> Result<EncUnit> {
        match arr.data_type() {
            // List is an exception because its nulls are stored in the validity buffer. We directly treat it as a primitive type.
            DataType::List(_) | DataType::LargeList(_) => self.primitive_encode(arr),
            t if t.is_primitive() => self.primitive_encode(arr),
            DataType::Boolean => self.boolean_encode(arr),
            _ => unimplemented!(),
        }
    }
    fn encoding_type(&self) -> Encoding {
        Encoding::Plain
    }
}

pub struct PlainDecoder {
    state: NonNullDecoderState,
}

impl PlainDecoder {
    pub fn new(encblock: Bytes) -> Self {
        Self {
            state: NonNullDecoderState::new(encblock),
        }
    }
}

impl Decoder for PlainDecoder {
    fn decode_all(&mut self) -> Result<Vec<Buffer>> {
        if self.state.data.is_empty() {
            return Err(Error::General("no data to decode".to_string()));
        }
        Ok(vec![Buffer::from_vec(Vec::<u8>::from(std::mem::take(
            &mut self.state.data,
        )))])
    }

    fn decode_a_vector(&mut self) -> Result<Option<Vec<Buffer>>> {
        if self.state.data.is_empty() {
            return Err(Error::General("no data to decode".to_string()));
        }
        let start = *self
            .state
            .metadata()
            .mini_blocks_offsets
            .get(self.state.vector_index - 1)
            .ok_or_else(|| Error::General("invalid start miniblock offset".to_string()))?
            as usize;
        let end = self
            .state
            .metadata()
            .mini_blocks_offsets
            .get(self.state.vector_index);
        match (end, self.state.vector_index) {
            (None, i) if i == self.state.metadata().mini_blocks_offsets.len() => {
                return Ok(Some(vec![Buffer::from_vec(Vec::<u8>::from(
                    &self.state.data[start..],
                ))]))
            }
            (None, _) => return Ok(None),
            (Some(size), _) => {
                let res = Some(vec![Buffer::from_vec(Vec::<u8>::from(
                    &self.state.data[start..*size as usize],
                ))]);
                self.state.vector_index += 1;
                Ok(res)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::sync::Arc;

    use crate::enc_unit::FlatEncUnit;
    use crate::schemes::encode_to_bytes;
    use fff_core::util::buffer_to_array::primitive_array_from_arrow_buffers;

    #[test]
    fn test_plain_encoding() {
        use super::*;
        use arrow::array::Int32Array;
        use {PlainDecoder, PlainEncoder};
        // create 64k vector, value from 1 to 64k
        let arr = Int32Array::from((1..=64 * 1024).collect::<Vec<i32>>());
        let arr = Arc::new(arr) as ArrayRef;
        let enc = Rc::new(PlainEncoder {}) as Rc<dyn Encoder>;
        let bytes = encode_to_bytes(enc, arr.clone());
        let plain_buffer = FlatEncUnit::try_deserialize(bytes).unwrap().buffers()[0]
            .try_to_dense()
            .unwrap();
        let mut dec = PlainDecoder::new(plain_buffer.clone());
        let mut res = vec![Buffer::from_vec::<u8>(vec![])];
        res.extend(dec.decode_all().unwrap());

        let output = primitive_array_from_arrow_buffers(arr.data_type(), res, 64 * 1024).unwrap();
        assert_eq!(*arr, *output);
        dec = PlainDecoder::new(plain_buffer);
        for (_, row_id) in (0..64 * 1024).step_by(MINIBLOCK_SIZE).enumerate() {
            let mut res = vec![Buffer::from_vec::<u8>(vec![])];
            res.extend(dec.decode_a_vector().unwrap().unwrap());
            let output =
                primitive_array_from_arrow_buffers(arr.data_type(), res, MINIBLOCK_SIZE as u64)
                    .unwrap();
            assert_eq!(*arr.slice(row_id, MINIBLOCK_SIZE), *output);
        }
    }
}
