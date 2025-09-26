use std::{rc::Rc, sync::Arc};

use arrow_schema::DataType;
use fff_encoding::schemes::{vortex::VortexEncoder, Encoder};

use crate::context::WASMWritingContext;

use super::custom::CustomEncoder;

/// Strategy to map physical DataType to EncUnit Encoder.
/// List is using our custom ones since Vortex does not support it.
/// List appears here because we encode offsets as a List of dummy values.
pub fn create_encunit_encoder(
    wasm_context: Arc<WASMWritingContext>,
    data_type: DataType,
    enable_dict: bool,
) -> Rc<dyn Encoder> {
    if let Some(lib) = wasm_context.data_type_to_wasm_lib(&data_type) {
        // FIXME: function name is fixed as "encode"
        Rc::new(CustomEncoder::try_new(lib.encode_lib_path(), "encode").unwrap())
    } else {
        Rc::new(VortexEncoder::new(enable_dict))
    }
    // match data_type {
    //     DataType::List(_) | DataType::LargeList(_) => {
    //         return Rc::new(NullableEncoder::new(
    //             Box::new(PlainEncoder {}),
    //             Box::new(PlainEncoder {}),
    //         ));
    //     }
    //     _ => {}
    // }
    // if !data_type.is_primitive() {
    //     todo!("Only primitive types are supported for now.");
    // }
    // if nullable {
    //     Rc::new(NullableEncoder::new(
    //         Box::new(PlainEncoder {}),
    //         Box::new(PlainEncoder {}),
    //     ))
    // } else {
    //     Rc::new(PlainEncoder {})
    // }
}
