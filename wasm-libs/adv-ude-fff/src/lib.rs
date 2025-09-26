#![allow(unused_imports)]
use arrow_buffer::Buffer;
use bytes::Bytes;
use datafusion_substrait::substrait::proto::ExtendedExpression;
use fff_encoding::schemes::vortex::VortexDecoder;
use fff_encoding::schemes::vortex::VortexDecoderBuilder;
use fff_encoding::schemes::vortex::VtxPPD;
use fff_encoding::schemes::Decoder;
use fff_ude::arraydata_to_buffers;
use fff_ude::ffi::decode_wrapper;
use fff_ude::ffi::init_wrapper;
use fff_ude::ffi::WasmDecoder;
use fff_ude::kwargs::kwargs_deserialize;
use fff_ude::kwargs::ArchivedOperator;
use fff_ude::kwargs::ArchivedScalarValue;
use fff_ude::Result;
use fff_ude::StatefulWasmDecoder;
use prost::Message;
use roaring::RoaringBitmap;
use vortex_array::array::ConstantArray;
use vortex_array::IntoArrayData;
use vortex_sampling_compressor::ALL_ENCODINGS_CONTEXT;
use vortex_scalar::Scalar;

#[no_mangle]
pub unsafe extern "C" fn init_ffi(
    input_ptr: *const u8,
    input_len: usize,
    kwargs_ptr: *const u8,
    kwargs_len: usize,
    out: *mut fff_ude::ffi::CSlice,
) -> i32 {
    init_wrapper(init_fff, input_ptr, input_len, kwargs_ptr, kwargs_len, out)
}

/// A decoder that does not support any advanced features, and can only decode once.
struct BasicDecoder {
    decoder: VortexDecoder,
    done: bool,
}

impl StatefulWasmDecoder for BasicDecoder {
    fn decode(&mut self) -> Result<Option<Box<dyn Iterator<Item = Buffer>>>> {
        if self.done {
            Ok(None)
        } else {
            let data = self.decoder.decode_all_as_array().unwrap().to_data();

            let mut res: Vec<Buffer> = vec![];
            arraydata_to_buffers(&mut res, &data);
            self.done = true;
            Ok(Some(
                Box::new(res.into_iter()) as Box<dyn Iterator<Item = Buffer>>
            ))
        }
    }
}

fn init_fff(input: &[u8], kwargs: &[u8]) -> Result<Box<dyn StatefulWasmDecoder>> {
    let bytes = Bytes::copy_from_slice(input);
    // let expr = ExtendedExpression::decode(kwargs).unwrap();
    // let rb2 = RoaringBitmap::deserialize_from(&kwargs[..]).unwrap();
    // let t = rb2.iter().next().unwrap();

    let kwargs = kwargs_deserialize(kwargs);
    let mut builder = VortexDecoderBuilder::new(bytes.clone(), ALL_ENCODINGS_CONTEXT.clone());
    builder = if let Some(serialized_expr) = kwargs.get("ppd".as_bytes()) {
        let expr = fff_ude::kwargs::ppd_deserialize(serialized_expr);
        let op = expr.op();
        let right = expr.right();
        assert!(op == &ArchivedOperator::Eq);
        assert!(matches!(right, &ArchivedScalarValue::I32(_)));
        builder.with_ppd(VtxPPD::new(
            Scalar::from(right.as_i32()),
            vortex_array::compute::Operator::Eq,
        ))
    } else if let Some(enabled) = kwargs
        .get("partial_decode".as_bytes())
        .map(|b| *b.get(0).unwrap_or(&0) != 0)
    {
        if enabled {
            builder.with_partial_decode(true)
        } else {
            builder
        }
    } else {
        builder
    };
    let vortex_decoder = builder.try_build()?;

    Ok(Box::new(BasicDecoder {
        decoder: vortex_decoder,
        done: false,
    }))
}

#[no_mangle]
pub unsafe extern "C" fn decode_ffi(
    decoder: *mut WasmDecoder,
    out: *mut fff_ude::ffi::CSlice,
) -> i32 {
    decode_wrapper(decode_fff, decoder, out)
}

fn decode_fff(input: *mut WasmDecoder) -> Result<Option<Box<dyn Iterator<Item = Buffer>>>> {
    let mut decoder = unsafe { Box::from_raw(input) };
    let res = decoder.decode()?;
    if res.is_some() {
        // Do not free the pointer if there is still some to decode.
        let _ = Box::into_raw(decoder);
    }
    Ok(res)
}
