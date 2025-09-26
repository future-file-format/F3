use std::sync::Arc;

use arrow_buffer::Buffer;
use bytes::Bytes;
use fff_core::errors::{Error, Result};
use fff_ude_wasm::Runtime;

use fff_encoding::schemes::Decoder;

pub struct WasmDecoder<'a> {
    data: Bytes,
    rt: Arc<Runtime>,
    func_name: &'a str,
}

impl<'a> WasmDecoder<'a> {
    pub fn new(encblock: Bytes, rt: Arc<Runtime>, func_name: &'a str) -> Self {
        Self {
            data: encblock,
            rt,
            func_name,
        }
    }
}

impl<'a> Decoder for WasmDecoder<'a> {
    fn decode_all(&mut self) -> Result<Vec<Buffer>> {
        let res = self
            .rt
            .call_single_buf(self.func_name, &self.data)
            .map_err(|e| Error::General(format!("{:?}", e)))?;

        Ok(vec![res.into()])
        // Ok(vec![Buffer::from_vec(res.into_vec())])
    }

    fn decode_a_vector(&mut self) -> Result<Option<Vec<Buffer>>> {
        unimplemented!();
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use arrow_array::{ArrayRef, UInt32Array};
    use fff_core::util::buffer_to_array::primitive_array_from_arrow_buffers;
    use fff_encoding::{
        enc_unit::FlatEncUnit,
        schemes::{bp::BPEncoder, encode_to_bytes, Encoder},
    };
    use fff_test_util::{BP_WASM_FUNC, VORTEX_WASM_FUNC};
    use rand::Rng;

    #[test]
    fn test_wasm_bp() {
        use super::*;
        // create 64k vector, with max value 127, randomly
        let vec: Vec<u32> = (1..=64 * 1024).map(|x| x % 128).collect();
        let arr = UInt32Array::from(vec);
        let arr = Arc::new(arr) as ArrayRef;
        let enc = Rc::new(BPEncoder {}) as Rc<dyn Encoder>;
        let bytes = encode_to_bytes(enc, arr.clone());
        let bytes = FlatEncUnit::read_first_buffer(bytes).unwrap();
        let rt = Arc::new(
            Runtime::try_new(&std::fs::read(fff_test_util::BP_WASM_PATH).unwrap()).unwrap(),
        );
        let mut dec = WasmDecoder::new(bytes, rt, BP_WASM_FUNC);
        let mut res = vec![Buffer::from_vec::<u8>(vec![])];
        res.extend(dec.decode_all().unwrap());

        let output = primitive_array_from_arrow_buffers(arr.data_type(), res, 64 * 1024).unwrap();
        assert_eq!(*arr, *output);
    }

    #[test]
    fn test_wasm_vortex() {
        use super::*;
        use fff_encoding::schemes::vortex::VortexEncoder;
        // create 64k vector, with max value 127, each value is randomly generated
        let mut rng = rand::thread_rng();
        let vec: Vec<u32> = (0..64 * 1024).map(|_| rng.gen_range(0..=127)).collect();
        let arr = UInt32Array::from(vec);
        let arr = Arc::new(arr) as ArrayRef;
        let enc = Rc::new(VortexEncoder::default()) as Rc<dyn Encoder>;
        let bytes = encode_to_bytes(enc, arr.clone());
        let bytes = FlatEncUnit::read_first_buffer(bytes).unwrap();
        let rt = Arc::new(
            Runtime::try_new(&std::fs::read(fff_test_util::VORTEX_WASM_PATH).unwrap()).unwrap(),
        );
        let mut dec = WasmDecoder::new(bytes, rt, VORTEX_WASM_FUNC);
        let mut res = vec![Buffer::from_vec::<u8>(vec![])];
        res.extend(dec.decode_all().unwrap());

        let output = primitive_array_from_arrow_buffers(arr.data_type(), res, 64 * 1024).unwrap();
        assert_eq!(*arr, *output);
    }
}
