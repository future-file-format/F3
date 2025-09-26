use std::path::PathBuf;
use std::rc::Rc;
use std::result::Result;
use std::sync::Arc;

use arrow::ffi::{to_ffi, FFI_ArrowArray, FFI_ArrowSchema};
use bytes::Bytes;
use libloading::Library;
use uniffi_core::RustBuffer;

use fff_encoding::enc_unit::EncUnit;

use fff_encoding::schemes::Encoder;

type EncodeFunc =
    unsafe extern "C" fn(input: FFI_ArrowArray, schema: FFI_ArrowSchema) -> RustBuffer;

pub struct CustomEncoder {
    lib: Arc<Library>,
    func_name: String,
}

impl CustomEncoder {
    pub fn try_new(
        lib_path: Rc<PathBuf>,
        func_name: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            lib: unsafe { libloading::Library::new(lib_path.as_ref())? }.into(),
            func_name: func_name.to_string(),
        })
    }
}

impl Encoder for CustomEncoder {
    fn encode(&self, arr: arrow_array::ArrayRef) -> fff_core::errors::Result<EncUnit> {
        let func: libloading::Symbol<EncodeFunc> =
            unsafe { self.lib.get(self.func_name.as_bytes()).unwrap() };
        let encoded = {
            let (ffi_arr, ffi_schema) = to_ffi(&arr.to_data()).unwrap();
            Bytes::from(unsafe { func(ffi_arr, ffi_schema) }.destroy_into_vec())
        };
        Ok(EncUnit::new(
            vec![encoded],
            fff_encoding::enc_unit::Encoding::Custom,
            vec![],
        ))
    }

    fn encoding_type(&self) -> fff_encoding::enc_unit::Encoding {
        fff_encoding::enc_unit::Encoding::Custom
    }
}

#[cfg(test)]
mod tests {
    use arrow_array::ArrayRef;
    use fff_core::util::buffer_to_array::primitive_array_from_arrow_buffers_iter;
    use fff_encoding::schemes::Encoder;
    use std::{io::Cursor, path::PathBuf, sync::Arc};
    use wasm_test_encoders::decode_pco_real_general;

    use crate::encoder::custom::CustomEncoder;

    #[test]
    #[ignore]
    fn test_custom_encoder() {
        let encoder = CustomEncoder::try_new(
            PathBuf::from(
                "/home/xinyu/fff-devel/target/release/libfff_ude_example_pco_real_encoder.so",
            )
            .into(),
            "encode",
        )
        .unwrap();
        let arr = Arc::new(arrow::array::Int32Array::from(
            (0..65536).collect::<Vec<i32>>(),
        )) as ArrayRef;
        let enc_unit = encoder.encode(arr.clone()).unwrap();
        let encoded = Vec::new();
        let encoded = Cursor::new(encoded);
        let encoded = enc_unit.try_serialize(encoded).unwrap().into_inner();
        println!("{}", encoded.len());
        let out = primitive_array_from_arrow_buffers_iter(
            arr.data_type(),
            decode_pco_real_general(&encoded).unwrap(),
            arr.len() as u64,
        )
        .unwrap();
        assert_eq!(&(arr), &out);
    }
}
