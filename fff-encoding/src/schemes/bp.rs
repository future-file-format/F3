/// Deprecated after using Vortex.
/// Leave it here only for legacy usage.
use std::mem;

use crate::enc_unit::ALIGNMENT;
use crate::enc_unit::MINIBLOCK_SIZE;
use crate::schemes::{EncUnitMetadata, Encoding};
use arrow_array::{ArrayRef, UInt32Array};
use arrow_buffer::{Buffer, MutableBuffer};
use arrow_schema::DataType;
use bytes::Bytes;
use fastlanes::BitPacking;
use fff_core::{errors::Result, util::bit_util::ceil};

use super::{Decoder, EncUnit, Encoder, NonNullDecoderState};

/// Leaf nodes.
pub struct BPEncoder;

/// TODO: handle the case where num values is not multiple of 1024
impl BPEncoder {
    /// | metadata_size: u32 | mini_block_offsets: [uint32] | OptionalMetadata: bw_per_mini_block [u8] | .. padding to 64B | data: [ubyte] |
    fn encode(&self, arr: ArrayRef) -> Result<EncUnit> {
        debug_assert!(arr.to_data().buffers().len() == 1);
        debug_assert!(*arr.data_type() == DataType::UInt32);
        let data = arr.as_any().downcast_ref::<UInt32Array>().unwrap();
        let data = data.values().as_ref();
        let num_chunks = ceil(data.len(), MINIBLOCK_SIZE);
        let mut bw_per_mini_block: Vec<u8> = Vec::with_capacity(num_chunks);
        let mut mini_blocks_offsets: Vec<u32> = Vec::with_capacity(num_chunks);
        // Allocate a result byte array.
        let mut encoded_data: Vec<u32> = Vec::new();
        debug_assert!(data.len() % MINIBLOCK_SIZE == 0);
        for (start, end) in (0..data.len())
            .step_by(MINIBLOCK_SIZE)
            .map(|i| (i, i + MINIBLOCK_SIZE))
        {
            let end = std::cmp::min(end, data.len());
            let mini_block = &data[start..end];
            let bit_width = mini_block.iter().fold(0, |acc, &x| {
                std::cmp::max(acc, 32 - x.leading_zeros() as usize)
            });
            bw_per_mini_block.push(bit_width as u8);
            mini_blocks_offsets.push(encoded_data.len() as u32);
            let packed_len = 128 * bit_width / size_of::<u32>();
            // let packed_len = 128 * bit_width / size_of::<T>();
            encoded_data.reserve(packed_len);
            let output_len = encoded_data.len();
            unsafe {
                encoded_data.set_len(output_len + packed_len);
                BitPacking::unchecked_pack(
                    bit_width,
                    mini_block,
                    &mut encoded_data[output_len..][..packed_len],
                );
            }
        }
        mini_blocks_offsets.push(encoded_data.len() as u32);
        let metadata = EncUnitMetadata {
            num_values: arr.len() as u32,
            mini_blocks_offsets,
            metadata: Some(bw_per_mini_block),
        };
        // let mut s = flexbuffers::FlexbufferSerializer::new();
        // metadata.serialize(&mut s).unwrap();
        // let metadata: Bytes = s.take_buffer().into();
        let metadata = Bytes::from(
            rkyv::to_bytes::<_, 256>(&metadata)
                .unwrap()
                .into_boxed_slice(),
        );
        let metadata_size = metadata.len() as u32;
        let metadata_size = Bytes::from(metadata_size.to_le_bytes().to_vec());
        let encoded_data: Vec<u8> = unsafe {
            let ratio = mem::size_of::<u32>() / mem::size_of::<u8>();
            let length = encoded_data.len() * ratio;
            let capacity = encoded_data.capacity() * ratio;
            let ptr = encoded_data.as_mut_ptr() as *mut u8;
            // Don't run the destructor for vec32
            mem::forget(encoded_data);
            // Construct new Vec
            Vec::from_raw_parts(ptr, length, capacity)
        };
        let padding = Bytes::from(vec![
            0u8;
            (metadata_size.len() + metadata.len())
                .next_multiple_of(ALIGNMENT)
                - (metadata_size.len() + metadata.len())
        ]);
        let buffers = vec![metadata_size, metadata, padding, Bytes::from(encoded_data)];
        Ok(EncUnit::new(buffers, Encoding::BP, vec![]))
    }
}

impl Encoder for BPEncoder {
    fn encode(&self, arr: ArrayRef) -> Result<EncUnit> {
        match arr.data_type() {
            DataType::UInt32 => self.encode(arr),
            _ => unimplemented!(),
        }
    }
    fn encoding_type(&self) -> Encoding {
        Encoding::BP
    }
}

pub struct BPDecoder {
    state: NonNullDecoderState,
}

impl BPDecoder {
    pub fn new(encblock: Bytes) -> Self {
        Self {
            state: NonNullDecoderState::new(encblock),
        }
    }
}

impl Decoder for BPDecoder {
    fn decode_all(&mut self) -> Result<Vec<Buffer>> {
        let len = self.state.metadata().num_values as usize * std::mem::size_of::<u32>();
        let mut output_buffer = MutableBuffer::with_capacity(len);
        unsafe {
            output_buffer.set_len(len);
        }
        let output_slice: &mut [u32] = bytemuck::cast_slice_mut(output_buffer.as_mut());
        let metadata = &self.state.metadata();
        let bw_per_mini_block = metadata.metadata.as_ref().unwrap();
        let offsets = &metadata.mini_blocks_offsets;
        let data = &self.state.data;
        let data: &[u32] = bytemuck::cast_slice(data);
        for i in 1..offsets.len() {
            let start = offsets[i - 1] as usize;
            let end = offsets[i] as usize;
            let bit_width = bw_per_mini_block[i - 1];
            unsafe {
                BitPacking::unchecked_unpack(
                    bit_width as usize,
                    &data[start..end],
                    &mut output_slice[MINIBLOCK_SIZE * (i - 1)..MINIBLOCK_SIZE * i],
                );
            }
        }
        Ok(vec![output_buffer.into()])
    }

    fn decode_a_vector(&mut self) -> Result<Option<Vec<Buffer>>> {
        unimplemented!();
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
    fn test_bp() {
        use super::*;
        // create 64k vector, with max value 127, randomly
        let vec: Vec<u32> = (1..=64 * 1024).map(|x| x % 128).collect();
        let arr = UInt32Array::from(vec);
        let arr = Arc::new(arr) as ArrayRef;
        let enc = Rc::new(BPEncoder {}) as Rc<dyn Encoder>;
        let bytes = encode_to_bytes(enc, arr.clone());
        let bytes = FlatEncUnit::read_first_buffer(bytes).unwrap();
        let mut dec = BPDecoder::new(bytes);
        let mut res = vec![Buffer::from_vec::<u8>(vec![])];
        res.extend(dec.decode_all().unwrap());

        let output = primitive_array_from_arrow_buffers(arr.data_type(), res, 64 * 1024).unwrap();
        assert_eq!(*arr, *output);

        // for (_, row_id) in (0..64 * 1024).step_by(MINIBLOCK_SIZE).enumerate() {
        //     let res = dec.decode_a_vector().unwrap().unwrap();
        //     let output =
        //         primitive_array_from_buffers(arr.data_type(), res, MINIBLOCK_SIZE as u64).unwrap();
        //     assert_eq!(*arr.slice(row_id, MINIBLOCK_SIZE), *output);
        // }
    }
}
