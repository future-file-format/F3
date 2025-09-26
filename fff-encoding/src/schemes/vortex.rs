use std::ops::Range;
use std::sync::Arc;

use crate::enc_unit::ZEROS;

use super::{Decoder, EncUnit, Encoder, Encoding};
use arrow::array::AsArray;
use arrow::datatypes::{Int32Type, Int64Type};
use arrow_array::downcast_integer;
use arrow_array::downcast_primitive_array_helper;
use arrow_array::{Array, ArrayRef, BooleanArray, DictionaryArray, PrimitiveArray};
use arrow_buffer::{BooleanBuffer, Buffer};
use arrow_schema::DataType;
use byteorder::{LittleEndian, ReadBytesExt};
use bytes::Bytes;
use fff_core::errors::{Error, Result};
use fff_core::non_nest_types;
use fff_core::util::bit_util::padding_size;
use fff_core::util::buffer_to_array::new_list_offsets_validity_from_buffers;
use futures::executor::block_on;
use serde::{Deserialize, Serialize};
use vortex_array::array::ConstantArray;
use vortex_array::arrow::FromArrowArray;
use vortex_array::compress::CompressionStrategy;
use vortex_array::compute::{compare, slice, Operator};
use vortex_array::encoding::Encoding as _;
use vortex_array::{ArrayDType, Context, IntoArrayData};
use vortex_array::{ArrayData, IntoCanonical};
use vortex_ipc::messages::reader::{ArrayMessageReader, DTypeBufferReader};
use vortex_ipc::messages::writer::MessageWriter;
use vortex_sampling_compressor::compressors::dict::DictCompressor;
use vortex_sampling_compressor::{CompressConfig, SamplingCompressor, DEFAULT_COMPRESSORS};
use vortex_scalar::Scalar;
const VORTEX_ALIGNMENT: usize = 64;

fn vortex_array_to_arrow(vortex: ArrayData) -> ArrayRef {
    vortex.into_canonical().unwrap().into_arrow().unwrap()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VortexFooter {
    pub byte_offsets: Vec<u64>,
    pub row_offsets: Vec<u64>,
    pub dtype_range: Range<u64>,
}

pub struct VortexEncoder {
    enable_dict: bool,
}

impl VortexEncoder {
    pub fn new(enable_dict: bool) -> Self {
        Self { enable_dict }
    }
}

impl Default for VortexEncoder {
    fn default() -> Self {
        Self { enable_dict: true }
    }
}

impl VortexEncoder {
    fn encode_arr(&self, arr: ArrayRef) -> Result<Vec<Bytes>> {
        let compress_options = CompressConfig::default();
        // .with_sample_size(512)
        // .with_sample_count(32);
        let compressor: &dyn CompressionStrategy = if self.enable_dict {
            &SamplingCompressor::new_with_options(
                vortex_array::aliases::hash_set::HashSet::from_iter(DEFAULT_COMPRESSORS),
                compress_options,
            )
        } else {
            &SamplingCompressor::new_with_options(
                vortex_array::aliases::hash_set::HashSet::from_iter(DEFAULT_COMPRESSORS),
                compress_options,
            )
            .excluding(&DictCompressor)
        };

        let compressed_array =
            compressor.compress(&ArrayData::from_arrow(arr.clone(), arr.is_nullable()))?;

        // println!("{:?}", compressed_array.encoding());
        // let children = compressed_array.children();
        // dbg!(children.len());
        // for child in children {
        //     println!("{:?}", child.encoding());
        //     println!("{:?}", child.dtype());
        // }
        let encoded_data = Vec::<u8>::new();
        let mut writer = MessageWriter::new(encoded_data);
        block_on(writer.write_dtype(compressed_array.dtype().clone()))?;
        let dtype_buf = writer.into_inner();
        let dtype_size = Bytes::from((dtype_buf.len() as u32).to_le_bytes().to_vec());
        let written_len = dtype_buf.len() + dtype_size.len();
        let encoded_data = Vec::<u8>::new();
        let mut writer = MessageWriter::new(encoded_data);
        block_on(writer.write_array(compressed_array))?;
        let w = writer.into_inner();

        Ok(vec![
            dtype_size,
            Bytes::from(dtype_buf),
            Bytes::from(&ZEROS[..padding_size(written_len, VORTEX_ALIGNMENT)]),
            Bytes::from(w),
        ])
    }

    fn regular_encode(&self, arr: ArrayRef) -> Result<EncUnit> {
        debug_assert!(match arr.data_type() {
            non_nest_types!() => true,
            _ => false,
        });
        Ok(EncUnit::new(
            self.encode_arr(arr)?,
            Encoding::Vortex,
            vec![],
        ))
    }

    /// For List type, we only encode the validity and offsets, children are handled by logical encoders.
    fn list_encode(&self, arr: ArrayRef) -> Result<EncUnit> {
        debug_assert!(match arr.data_type() {
            DataType::List(_) | DataType::LargeList(_) => true,
            _ => false,
        });
        let (validity, offsets) = match arr.data_type() {
            DataType::List(_) => {
                let list_array = arr
                    .as_any()
                    .downcast_ref::<arrow_array::ListArray>()
                    .unwrap();
                let validity = Arc::new(BooleanArray::new(
                    list_array.nulls().map_or_else(
                        // TODO: reuse all-true boolean buffer using lazy static
                        || BooleanBuffer::new_set(list_array.len()),
                        |v| v.clone().into_inner(),
                    ),
                    None,
                )) as Arc<dyn Array>;
                let offsets = list_array.offsets().clone().into_inner();
                let offsets =
                    Arc::new(PrimitiveArray::<Int32Type>::new(offsets, None)) as Arc<dyn Array>;
                (validity, offsets)
            }
            DataType::LargeList(_) => {
                let list_array = arr
                    .as_any()
                    .downcast_ref::<arrow_array::LargeListArray>()
                    .unwrap();
                let validity = Arc::new(BooleanArray::new(
                    list_array.nulls().map_or_else(
                        // TODO: reuse all-true boolean buffer using lazy static
                        || BooleanBuffer::new_set(list_array.len()),
                        |v| v.clone().into_inner(),
                    ),
                    None,
                )) as Arc<dyn Array>;
                let offsets = list_array.offsets().clone().into_inner();
                let offsets =
                    Arc::new(PrimitiveArray::<Int64Type>::new(offsets, None)) as Arc<dyn Array>;
                (validity, offsets)
            }
            _ => panic!("wrong type in list_encode"),
        };

        Ok(EncUnit::new(
            {
                let mut res = self.encode_arr(validity)?;
                res.extend(self.encode_arr(offsets)?);
                res
            },
            Encoding::Vortex,
            vec![],
        ))
    }

    pub fn list_struct_encode(&self, list_arr: ArrayRef, field: ArrayRef) -> Result<EncUnit> {
        debug_assert!(match list_arr.data_type() {
            DataType::List(_) | DataType::LargeList(_) => true,
            _ => false,
        });
        let (validity, offsets) = match list_arr.data_type() {
            DataType::List(_) => {
                let list_array = list_arr
                    .as_any()
                    .downcast_ref::<arrow_array::ListArray>()
                    .unwrap();
                let validity = Arc::new(BooleanArray::new(
                    list_array.nulls().map_or_else(
                        // TODO: reuse all-true boolean buffer using lazy static
                        || BooleanBuffer::new_set(list_array.len()),
                        |v| v.clone().into_inner(),
                    ),
                    None,
                )) as Arc<dyn Array>;
                let offsets = list_array.offsets().clone().into_inner();
                let offsets =
                    Arc::new(PrimitiveArray::<Int32Type>::new(offsets, None)) as Arc<dyn Array>;
                (validity, offsets)
            }
            DataType::LargeList(_) => {
                let list_array = list_arr
                    .as_any()
                    .downcast_ref::<arrow_array::LargeListArray>()
                    .unwrap();
                let validity = Arc::new(BooleanArray::new(
                    list_array.nulls().map_or_else(
                        // TODO: reuse all-true boolean buffer using lazy static
                        || BooleanBuffer::new_set(list_array.len()),
                        |v| v.clone().into_inner(),
                    ),
                    None,
                )) as Arc<dyn Array>;
                let offsets = list_array.offsets().clone().into_inner();
                let offsets =
                    Arc::new(PrimitiveArray::<Int64Type>::new(offsets, None)) as Arc<dyn Array>;
                (validity, offsets)
            }
            _ => panic!("wrong type in list_encode"),
        };

        Ok(EncUnit::new(
            {
                let mut res = self.encode_arr(validity)?;
                res.extend(self.encode_arr(offsets)?);
                res.extend(self.encode_arr(field)?);
                res
            },
            Encoding::Vortex,
            vec![],
        ))
    }
}

impl Encoder for VortexEncoder {
    fn encode(&self, arr: ArrayRef) -> Result<EncUnit> {
        match arr.data_type() {
            non_nest_types!() => self.regular_encode(arr),
            DataType::List(_) | DataType::LargeList(_) => self.list_encode(arr),
            _ => unimplemented!("Vortex encoding for this type"),
        }
    }
    fn encoding_type(&self) -> Encoding {
        Encoding::Vortex
    }
}

pub fn vortex_deser(encblock: &mut Bytes, context: Arc<Context>) -> Result<ArrayData> {
    let metadata_size = encblock.split_to(4).as_ref().read_u32::<LittleEndian>()?;
    let mut dtype_bytes = encblock.split_to(metadata_size as usize);

    let mut dtype_reader = DTypeBufferReader::new();
    let mut read_buf = Bytes::new();
    while let Some(u) = dtype_reader.read(read_buf)? {
        read_buf = dtype_bytes.split_to(u);
    }
    let dtype = dtype_reader.into_dtype();
    let _padding = encblock.split_to(padding_size(metadata_size as usize + 4, VORTEX_ALIGNMENT));
    let mut array_reader = ArrayMessageReader::new();
    let mut read_buf = Bytes::new();
    while let Some(u) = array_reader.read(read_buf)? {
        read_buf = encblock.split_to(u);
    }
    Ok(array_reader.into_array(context, dtype)?)
}

/// For testing usage here, only limited expressions for now.
pub struct VtxPPD {
    right: Scalar,
    op: Operator,
}

impl VtxPPD {
    pub fn new(right: Scalar, op: Operator) -> Self {
        Self { right, op }
    }
}

pub struct VortexDecoderBuilder {
    encunit: Bytes,
    context: Arc<Context>,
    partial_decode: bool,
    ppd: Option<VtxPPD>,
}

impl VortexDecoderBuilder {
    pub fn new(encunit: Bytes, context: Arc<Context>) -> Self {
        Self {
            encunit,
            context,
            partial_decode: false,
            ppd: None,
        }
    }

    pub fn with_partial_decode(mut self, partial_decode: bool) -> Self {
        assert!(self.ppd.is_none());
        self.partial_decode = partial_decode;
        self
    }

    pub fn with_ppd(mut self, ppd: VtxPPD) -> Self {
        assert!(self.partial_decode == false);
        self.ppd = Some(ppd);
        self
    }

    pub fn try_build(mut self) -> Result<VortexDecoder> {
        match (self.partial_decode, self.ppd) {
            (false, Some(ppd)) => {
                let array = vortex_deser(&mut self.encunit, self.context)?;
                let res = compare(
                    &array,
                    ConstantArray::new(ppd.right, array.len()).into_array(),
                    ppd.op,
                )
                .map_err(|e| Error::External(e.into()))?;
                Ok(VortexDecoder {
                    vortex_array: Some(res),
                    partial_decode: false,
                })
            }
            (true, None) => Ok(VortexDecoder {
                vortex_array: Some(vortex_deser(&mut self.encunit, self.context)?),
                partial_decode: true,
            }),
            (false, None) => Ok(VortexDecoder {
                vortex_array: Some(vortex_deser(&mut self.encunit, self.context)?),
                partial_decode: false,
            }),
            _ => panic!("Cannot have partial decode and PPD at the same time"),
        }
    }
}

pub struct VortexDecoder {
    vortex_array: Option<ArrayData>,
    /// Preserve the last level encoding (if any) or not.
    /// Last level encoding can be Dict, REE, or StringView.
    partial_decode: bool,
}

impl VortexDecoder {
    pub fn try_new(encunit: Bytes, context: Arc<Context>) -> Result<Self, Error> {
        VortexDecoderBuilder::new(encunit, context).try_build()
    }
}

impl Decoder for VortexDecoder {
    fn decode_all(&mut self) -> Result<Vec<Buffer>> {
        if self.vortex_array.is_none() {
            return Ok(vec![]);
        }
        let mut res = Vec::new();
        let arrow_array = vortex_array_to_arrow(self.vortex_array.take().unwrap());
        if let Some(b) = arrow_array.nulls() {
            res.push(b.buffer().clone());
        }
        for b in arrow_array.to_data().buffers() {
            res.push(b.clone());
        }
        Ok(res)
    }

    fn slice(&mut self, start: usize, stop: usize) -> Result<ArrayRef> {
        // let arrow_array = vortex_array_to_arrow(vortex_array::compute::slice(
        //     self.vortex_array.take().unwrap(),
        //     start,
        //     stop,
        // )?);
        let arr = self.vortex_array.take().unwrap();
        let mask = vortex_array::compute::FilterMask::from_indices(arr.len(), start..stop);
        let arr = vortex_array::compute::filter(&arr, mask)
            .and_then(IntoCanonical::into_canonical)
            .map(ArrayData::from)?;
        let arrow_array = arr.into_arrow().unwrap();
        Ok(arrow_array)
    }

    fn decode_all_as_array(&mut self) -> Result<ArrayRef> {
        if self.partial_decode {
            let arr = self.vortex_array.take().unwrap();
            if arr.is_encoding(vortex_runend::RunEndEncoding::ID) {
                todo!();
            } else if arr.is_encoding(vortex_dict::DictEncoding::ID) {
                // Preserve dictionary encoding
                let mut children = arr.children();
                assert!(children.len() == 2);
                let codes = children
                    .remove(0)
                    .into_canonical()
                    .unwrap()
                    .into_arrow()
                    .unwrap();
                let codes = codes.as_ref();
                let codes_dtype = codes.data_type();
                let values = children
                    .remove(0)
                    .into_canonical()
                    .unwrap()
                    .into_arrow()
                    .unwrap();
                let out = downcast_integer! {
                    codes_dtype => (downcast_primitive_array_helper, codes, {Arc::new(DictionaryArray::try_new(codes.clone(), values)?) as ArrayRef}),
                    _ => panic!("wrong type in dictionary codes")
                };

                Ok(out)
            } else {
                unimplemented!("Partial decoding for this encoding")
            }
        } else {
            Ok(vortex_array_to_arrow(self.vortex_array.take().unwrap()))
        }
    }

    /// FIXME: decode_a_vector should be redesigned to be indexing based. Or just like slice.
    fn decode_a_vector(&mut self) -> Result<Option<Vec<Buffer>>> {
        Ok(self.vortex_array.as_ref().map(|arr| {
            let arr = slice(&arr, 0, 1024).expect("Slice failed");
            let mut res = Vec::new();
            let arrow_array = vortex_array_to_arrow(arr);
            if let Some(b) = arrow_array.nulls() {
                res.push(b.buffer().clone());
            }
            for b in arrow_array.to_data().buffers() {
                res.push(b.clone());
            }
            res
        }))
    }
}

/// Sadly, Vortex does not support List yet so we have to manually add our serde of list based on Vortex.
pub struct VortexListDecoder {
    vortex_validity_array: Option<ArrayData>,
    vortex_offsets_array: Option<ArrayData>,
    out_type: DataType,
}

impl VortexListDecoder {
    pub fn try_new(
        mut encblock: Bytes,
        out_type: DataType,
        context: Arc<Context>,
    ) -> Result<Self, Error> {
        Ok(Self {
            vortex_validity_array: Some(vortex_deser(&mut encblock, context.clone())?),
            vortex_offsets_array: Some(vortex_deser(&mut encblock, context)?),
            out_type,
        })
    }
}

impl Decoder for VortexListDecoder {
    fn decode_all_as_array(&mut self) -> Result<ArrayRef> {
        let validity_array = vortex_array_to_arrow(self.vortex_validity_array.take().unwrap());
        let validity = validity_array
            .as_any()
            .downcast_ref::<BooleanArray>()
            .unwrap()
            .values()
            .inner()
            .clone();
        let offsets_array = vortex_array_to_arrow(self.vortex_offsets_array.take().unwrap());
        let offsets = offsets_array.into_data().buffers()[0].clone();
        // .as_any()
        // .downcast_ref::<PrimitiveArray<Int32Type>>()
        // .unwrap()
        // .values()
        // .inner();
        match self.out_type {
            DataType::List(_) => Ok(new_list_offsets_validity_from_buffers::<Int32Type>(
                vec![validity, offsets],
                validity_array.len() as u64,
                None,
            )),
            DataType::LargeList(_) => Ok(new_list_offsets_validity_from_buffers::<Int64Type>(
                vec![validity, offsets],
                validity_array.len() as u64,
                None,
            )),
            _ => panic!("wrong type in VortexListDecoder"),
        }
    }
}

/// An experimental Decoder for List(Struct(_)) using Vortex.
pub struct VortexListStructDecoder {
    vortex_validity_array: Option<ArrayData>,
    vortex_offsets_array: Option<ArrayData>,
    vortex_struct_array: Option<ArrayData>,
    out_type: DataType,
}

impl VortexListStructDecoder {
    pub fn try_new(
        mut encblock: Bytes,
        out_type: DataType,
        context: Arc<Context>,
    ) -> Result<Self, Error> {
        Ok(Self {
            vortex_validity_array: Some(vortex_deser(&mut encblock, context.clone())?),
            vortex_offsets_array: Some(vortex_deser(&mut encblock, context.clone())?),
            vortex_struct_array: Some(vortex_deser(&mut encblock, context)?),
            out_type,
        })
    }
}

impl Decoder for VortexListStructDecoder {
    fn slice(&mut self, start: usize, stop: usize) -> Result<ArrayRef> {
        let validity_array = vortex_array_to_arrow(vortex_array::compute::slice(
            self.vortex_validity_array.take().unwrap(),
            start,
            stop,
        )?);
        let validity = validity_array
            .as_any()
            .downcast_ref::<BooleanArray>()
            .unwrap()
            .values()
            .inner()
            .clone();
        let offsets_array = vortex_array_to_arrow(vortex_array::compute::slice(
            self.vortex_offsets_array.take().unwrap(),
            start,
            stop + 1,
        )?);
        assert!((stop - start) == 1, "only supports random access now");
        let offsets = offsets_array.as_primitive::<Int32Type>();
        let struct_array = vortex_array_to_arrow(vortex_array::compute::slice(
            self.vortex_struct_array.take().unwrap(),
            offsets.value(0) as usize,
            offsets.value(1) as usize,
        )?);
        let offsets: Vec<i32> = vec![0, offsets.value(1) - offsets.value(0)];
        let offsets = Buffer::from_vec(offsets);
        // let offsets = offsets_array.into_data().buffers()[0].clone();
        match self.out_type {
            DataType::List(_) => Ok(new_list_offsets_validity_from_buffers::<Int32Type>(
                vec![validity, offsets],
                validity_array.len() as u64,
                Some(struct_array),
            )),
            DataType::LargeList(_) => unimplemented!(),
            // Ok(new_list_offsets_validity_from_buffers::<Int64Type>(
            //     vec![validity, offsets],
            //     validity_array.len() as u64,
            //     Some(struct_array),
            // ))
            // ,
            _ => panic!("wrong type in VortexListDecoder"),
        }
    }

    fn decode_all_as_array(&mut self) -> Result<ArrayRef> {
        let validity_array = vortex_array_to_arrow(self.vortex_validity_array.take().unwrap());
        let validity = validity_array
            .as_any()
            .downcast_ref::<BooleanArray>()
            .unwrap()
            .values()
            .inner()
            .clone();
        let offsets_array = vortex_array_to_arrow(self.vortex_offsets_array.take().unwrap());
        let struct_array = vortex_array_to_arrow(self.vortex_struct_array.take().unwrap());
        let offsets = offsets_array.into_data().buffers()[0].clone();
        match self.out_type {
            DataType::List(_) => Ok(new_list_offsets_validity_from_buffers::<Int32Type>(
                vec![validity, offsets],
                validity_array.len() as u64,
                Some(struct_array),
            )),
            DataType::LargeList(_) => Ok(new_list_offsets_validity_from_buffers::<Int64Type>(
                vec![validity, offsets],
                validity_array.len() as u64,
                Some(struct_array),
            )),
            _ => panic!("wrong type in VortexListDecoder"),
        }
    }
}
#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use crate::schemes::encode_to_bytes;
    use arrow_array::{UInt16Array, UInt32Array};
    use vortex_sampling_compressor::ALL_ENCODINGS_CONTEXT;

    #[test]
    fn test_vortex() {
        use super::*;
        // create 64k vector, with max value 127, randomly
        let vec: Vec<u32> = (1..=64 * 1024).map(|x| x % 128).collect();
        let arr = UInt32Array::from(vec);
        let arr = Arc::new(arr) as ArrayRef;
        let enc = Rc::new(VortexEncoder::default()) as Rc<dyn Encoder>;
        let bytes = encode_to_bytes(enc, arr.clone());
        // For the new simplified approach, we can directly use the bytes
        let mut dec = VortexDecoder::try_new(bytes, ALL_ENCODINGS_CONTEXT.clone()).unwrap();
        let decoded = dec.decode_all_as_array().unwrap();
        assert_eq!(*arr, *decoded);
    }

    #[test]
    fn test_vortex_long_rle() {
        use super::*;
        let vec: Vec<u16> = (0..7565)
            .map(|_| 0)
            .chain((0..1365).map(|_| 1))
            .chain((0..56606).map(|_| 0))
            .collect::<Vec<_>>();
        let arr = UInt16Array::from(vec);
        let arr = Arc::new(arr) as ArrayRef;
        let enc = Rc::new(VortexEncoder::default()) as Rc<dyn Encoder>;
        let bytes = encode_to_bytes(enc, arr.clone());
        // For the new simplified approach, we can directly use the bytes
        let mut dec = VortexDecoder::try_new(bytes, ALL_ENCODINGS_CONTEXT.clone()).unwrap();
        let decoded = dec.decode_all_as_array().unwrap();
        assert_eq!(*arr, *decoded);
    }
}
