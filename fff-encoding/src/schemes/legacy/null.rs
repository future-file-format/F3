use std::sync::Arc;

use super::{Decoder, Encoder, Encoding};
use arrow_array::{ArrayRef, BooleanArray};
use arrow_buffer::{BooleanBuffer, Buffer};
use arrow_schema::DataType;
use bytes::Bytes;

use super::EncUnit;
use fff_core::errors::Result;

/// The following comments are for the deprecated layout!
/// Non-leaf nodes.
/// 4 bytes: null child encoding size (X)
/// X bytes: null child encoding bytes
/// Y byes: non-null child encoding bytes
pub struct NullableEncoder {
    validity_encoder: Box<dyn Encoder>,
    data_encoder: Box<dyn Encoder>,
}

impl NullableEncoder {
    pub fn new(validity_encoder: Box<dyn Encoder>, data_encoder: Box<dyn Encoder>) -> Self {
        Self {
            validity_encoder,
            data_encoder,
        }
    }

    fn primitive_encode(&self, arr: ArrayRef) -> Result<EncUnit> {
        // debug_assert!(arr.data_type().is_primitive());
        debug_assert!(arr.to_data().buffers().len() == 1);
        let validity_array = if let Some(nulls) = arr.nulls() {
            Arc::new(BooleanArray::new(nulls.inner().clone(), None)) as ArrayRef
        } else {
            let buff = BooleanBuffer::new_set(arr.len());
            Arc::new(BooleanArray::new(buff, None)) as ArrayRef
        };
        let validity_block = self.validity_encoder.encode(validity_array.clone())?;
        // FIXME: if DataType is List, the array len should + 1.
        let data_block = self.data_encoder.encode(arr.clone())?;
        Ok(EncUnit::new(
            vec![],
            Encoding::Nullable,
            vec![validity_block, data_block],
        ))
    }
}

impl Encoder for NullableEncoder {
    fn encode(&self, arr: ArrayRef) -> Result<EncUnit> {
        match arr.data_type() {
            // List is an exception because its nulls are stored in the validity buffer. We directly treat it as a primitive type.
            DataType::List(_) | DataType::LargeList(_) => self.primitive_encode(arr),
            t if t.is_primitive() => self.primitive_encode(arr),
            _ => unimplemented!(),
        }
    }
    fn encoding_type(&self) -> Encoding {
        Encoding::Nullable
    }
}

pub struct NullableDecoder {
    validity_encoder: Box<dyn Decoder>,
    data_encoder: Box<dyn Decoder>,
}

impl NullableDecoder {
    pub fn new(validity: Bytes, data: Bytes) -> Self {
        use super::plain::PlainDecoder;
        // let validity_size = encblock
        //     .split_to(4)
        //     .as_ref()
        //     .read_u32::<LittleEndian>()
        //     .unwrap();
        // let validity = encblock.split_to(validity_size as usize);
        // let data = encblock;
        Self {
            validity_encoder: Box::new(PlainDecoder::new(validity)),
            data_encoder: Box::new(PlainDecoder::new(data)),
        }
    }
}

impl Decoder for NullableDecoder {
    fn decode_all(&mut self) -> Result<Vec<Buffer>> {
        Ok(self
            .validity_encoder
            .decode_all()?
            .into_iter()
            .chain(self.data_encoder.decode_all()?)
            .collect())
    }

    fn decode_a_vector(&mut self) -> Result<Option<Vec<Buffer>>> {
        let validity = self.validity_encoder.decode_a_vector()?;
        let data = self.data_encoder.decode_a_vector()?;
        match (validity, data) {
            (Some(validity), Some(data)) => Ok(Some(validity.into_iter().chain(data).collect())),
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use crate::enc_unit::{FlatEncUnit, MINIBLOCK_SIZE};
    use crate::schemes::{encode_to_bytes, plain::PlainEncoder};
    use fff_core::util::buffer_to_array::primitive_array_from_arrow_buffers;

    #[test]
    fn test_nullable_encoding() {
        use super::*;
        use arrow::array::Int32Array;
        // create 64k vector
        let mut vec: Vec<Option<i32>> = (1..=64 * 1024).map(Some).collect();
        vec[13] = None;
        let arr = Int32Array::from(vec);
        let arr = Arc::new(arr) as ArrayRef;
        let enc = Rc::new(NullableEncoder::new(
            Box::new(PlainEncoder {}),
            Box::new(PlainEncoder {}),
        )) as Rc<dyn Encoder>;
        let bytes = encode_to_bytes(enc, arr.clone());
        let flat_enc_unit = FlatEncUnit::try_deserialize(bytes).unwrap();
        let mut dec = NullableDecoder::new(
            flat_enc_unit.buffers()[0].try_to_dense().unwrap(),
            flat_enc_unit.buffers()[1].try_to_dense().unwrap(),
        );
        let res = dec.decode_all().unwrap();

        let output = primitive_array_from_arrow_buffers(arr.data_type(), res, 64 * 1024).unwrap();
        assert_eq!(*arr, *output);
        let mut dec = NullableDecoder::new(
            flat_enc_unit.buffers()[0].try_to_dense().unwrap(),
            flat_enc_unit.buffers()[1].try_to_dense().unwrap(),
        );
        for (_, row_id) in (0..64 * 1024).step_by(MINIBLOCK_SIZE).enumerate() {
            let res = dec.decode_a_vector().unwrap().unwrap();
            let output =
                primitive_array_from_arrow_buffers(arr.data_type(), res, MINIBLOCK_SIZE as u64)
                    .unwrap();
            assert_eq!(*arr.slice(row_id, MINIBLOCK_SIZE), *output);
        }
    }
}
