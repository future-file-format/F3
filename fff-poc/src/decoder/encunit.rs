use std::{collections::HashMap, sync::Arc};

use arrow_array::ArrayRef;
use arrow_schema::DataType;
use bytes::Bytes;
use fff_core::{
    errors::{Error, Result},
    general_error, non_nest_types, nyi_err,
    util::buffer_to_array::primitive_array_from_arrow_buffers_iter,
};
use fff_encoding::schemes::{
    vortex::{VortexDecoder, VortexListDecoder, VortexListStructDecoder},
    Decoder,
};
use fff_format::File::fff::flatbuf as fb;
use fff_test_util::WASM_FUNC_GENERAL;
use fff_ude_wasm::Runtime;
use log::debug;
use vortex_sampling_compressor::ALL_ENCODINGS_CONTEXT;

use crate::{
    compression::decompress_data, context::WASMReadingContext,
    file::footer::DEFAULT_ENCODING_VERSIONS, io::reader::Reader,
};

/// Common API for decoding a EncUnit.
pub trait EncUnitDecoder {
    /// Here is the one batch at a time API for decode.
    /// `check` and `init` should be handled during the `new` function of the struct.
    fn decode_v2(&self) -> Result<Option<ArrayRef>> {
        nyi_err!("decode_v2")
    }
    /// Below are old APIs but still in use in the paper's experiments.
    /// Decode all the data into an Arrow Array
    fn decode(&self) -> Result<ArrayRef> {
        nyi_err!("decode")
    }
    /// Slice the data into an Arrow Array, start inclusive, stop exclusive
    fn slice(&self, _start: usize, _stop: usize) -> Result<ArrayRef> {
        nyi_err!("slice")
    }
}

/// The optional Key-Word args for advanced features.
type Key = String;
type Word = String;

pub struct WASMEncUnitDecoderV2 {
    _data: Bytes,
    _rt: Arc<Runtime>,
    _output_type: DataType,
    _num_rows: u64,
}

impl WASMEncUnitDecoderV2 {
    pub fn new(
        data: Bytes,
        rt: Arc<Runtime>,
        output_type: DataType,
        num_rows: u64,
        _kwargs: HashMap<Key, Word>,
    ) -> Self {
        // TODO: call check and init using kwargs.
        Self {
            _data: data,
            _rt: rt,
            _output_type: output_type,
            _num_rows: num_rows,
        }
    }
}

impl EncUnitDecoder for WASMEncUnitDecoderV2 {
    fn decode_v2(&self) -> Result<Option<ArrayRef>> {
        todo!();
    }
}

pub struct WASMEncUnitDecoder<'a> {
    data: Bytes,
    rt: Arc<Runtime>,
    func_name: &'a str,
    output_type: DataType,
    num_rows: u64,
}

impl<'a> WASMEncUnitDecoder<'a> {
    pub fn new(
        data: Bytes,
        rt: Arc<Runtime>,
        func_name: &'a str,
        output_type: DataType,
        num_rows: u64,
    ) -> Self {
        Self {
            data,
            rt,
            func_name,
            output_type,
            num_rows,
        }
    }
}

impl EncUnitDecoder for WASMEncUnitDecoder<'_> {
    fn decode(&self) -> Result<ArrayRef> {
        match &self.output_type {
            non_nest_types!() => {
                let res = self
                    .rt
                    .call_multi_buf(self.func_name, &self.data)
                    .map_err(|e| general_error!("WASM call failed", e))?;
                Ok(primitive_array_from_arrow_buffers_iter(
                    &self.output_type,
                    res,
                    self.num_rows,
                )?)
            }
            _ => unimplemented!(),
        }
    }
}

pub struct VortexEncUnitDecoder {
    data: Bytes,
    output_type: DataType,
}

impl VortexEncUnitDecoder {
    pub fn new(data: Bytes, output_type: DataType) -> Self {
        Self { data, output_type }
    }
}

impl EncUnitDecoder for VortexEncUnitDecoder {
    fn decode(&self) -> Result<ArrayRef> {
        let bytes = self.data.clone();
        let array = match self.output_type {
            non_nest_types!() => {
                let mut vortex_decoder =
                    VortexDecoder::try_new(bytes, ALL_ENCODINGS_CONTEXT.clone())?;
                vortex_decoder.decode_all_as_array()?
            }
            DataType::List(ref child) | DataType::LargeList(ref child)
                if matches!(child.data_type(),
                        DataType::Struct(fields)
                            if fields
                                .iter()
                                .all(|f| matches!(f.data_type(), non_nest_types!()))
                            && cfg!(feature = "list-offsets-pushdown")
                ) =>
            {
                let mut vortex_decoder = VortexListStructDecoder::try_new(
                    bytes,
                    self.output_type.clone(),
                    ALL_ENCODINGS_CONTEXT.clone(),
                )?;
                vortex_decoder.decode_all_as_array()?
            }
            DataType::List(_) | DataType::LargeList(_) => {
                let mut vortex_decoder = VortexListDecoder::try_new(
                    bytes,
                    self.output_type.clone(),
                    ALL_ENCODINGS_CONTEXT.clone(),
                )?;
                vortex_decoder.decode_all_as_array()?
            }
            _ => unimplemented!(),
        };
        if array.data_type() != &self.output_type {
            debug!(
                "data type mismatch: expected {}, got {}",
                self.output_type,
                array.data_type()
            );
        }
        // return Err(Error::General(format!(
        //     "data type mismatch: expected {}, got {}",
        //     self.output_type,
        //     array.data_type()
        // )));
        Ok(array)
    }

    fn slice(&self, start: usize, stop: usize) -> Result<ArrayRef> {
        let bytes = self.data.clone();
        let array = match self.output_type {
            non_nest_types!() => {
                let mut vortex_decoder =
                    VortexDecoder::try_new(bytes, ALL_ENCODINGS_CONTEXT.clone())?;
                vortex_decoder.slice(start, stop)?
            }
            DataType::List(ref child) | DataType::LargeList(ref child)
                if matches!(child.data_type(),
                    DataType::Struct(fields)
                        if fields
                            .iter()
                            .all(|f| matches!(f.data_type(), non_nest_types!()))
                            && cfg!(feature = "list-offsets-pushdown")) =>
            {
                let mut vortex_decoder = VortexListStructDecoder::try_new(
                    bytes,
                    self.output_type.clone(),
                    ALL_ENCODINGS_CONTEXT.clone(),
                )?;
                vortex_decoder.slice(start, stop)?
            }
            DataType::List(_) | DataType::LargeList(_) => {
                return nyi_err!("NYI");
                // let mut vortex_decoder = VortexListDecoder::try_new(
                //     bytes,
                //     self.output_type.clone(),
                //     ALL_ENCODINGS_CONTEXT.clone(),
                // )?;
                // vortex_decoder.decode_all_as_array()?
            }
            _ => unimplemented!(),
        };
        if array.data_type() != &self.output_type {
            debug!(
                "data type mismatch: expected {}, got {}",
                self.output_type,
                array.data_type()
            );
        }
        // return Err(Error::General(format!(
        //     "data type mismatch: expected {}, got {}",
        //     self.output_type,
        //     array.data_type()
        // )));
        Ok(array)
    }
}

pub fn create_encunit_decoder<R: Reader>(
    encoding: fb::Encoding,
    compression_type: fb::CompressionType,
    mut data: Bytes,
    num_rows: u64,
    output_type: DataType,
    wasm_context: Option<Arc<WASMReadingContext<R>>>,
) -> Result<Box<dyn EncUnitDecoder>> {
    if compression_type != fb::CompressionType::Uncompressed {
        data = decompress_data(data, compression_type)?;
    }
    let return_wasm_decoder = |data: Bytes,
                               output_type: DataType,
                               wasm_context: Arc<WASMReadingContext<R>>,
                               num_rows: u64|
     -> Result<Box<dyn EncUnitDecoder>> {
        Ok(Box::new(WASMEncUnitDecoder::new(
            data,
            wasm_context.get_runtime(crate::context::WASMId(
                encoding
                    .wasm_encoding()
                    .ok_or_else(|| {
                        Error::General("not provided custom WASM in the file".to_string())
                    })?
                    .wasm_id(),
            )),
            WASM_FUNC_GENERAL, // FIXME: should get from wasm binary
            output_type,
            num_rows,
        )))
    };
    Ok(match encoding.type_() {
        fb::EncodingType::CASCADE => {
            let wasm_context =
                wasm_context.ok_or_else(|| general_error!("WASM context not found"))?;
            let encoding_versions = wasm_context
                .get_encoding_versions()
                .ok_or_else(|| general_error!("Encoding versions not found"))?;
            let encoding_version = encoding_versions
                .get(&encoding.type_())
                .ok_or_else(|| general_error!("Encoding version not found"))?;
            let reader_version = DEFAULT_ENCODING_VERSIONS.get(&encoding.type_()).unwrap();
            // if reader has less version than the file, and major version is different or major version is 0, then it is incompatible
            if reader_version.cmp_precedence(encoding_version).is_lt()
                && (reader_version.major != encoding_version.major || reader_version.major == 0)
            {
                return_wasm_decoder(data, output_type, wasm_context, num_rows)?
            } else {
                Box::new(VortexEncUnitDecoder::new(data, output_type))
            }
        }
        fb::EncodingType::CUSTOM_WASM => {
            if let Some(wasm_context) = wasm_context {
                return_wasm_decoder(data, output_type, wasm_context, num_rows)?
            } else {
                return Err(general_error!("WASM context required for custom encoding"));
            }
        }
        _ => unimplemented!(),
    })
}
