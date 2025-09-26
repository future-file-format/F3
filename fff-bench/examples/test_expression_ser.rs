/// Test expression/predicate serialization overhead for PPD in Wasm decoders
///
use arrow::datatypes::{DataType, Field, Schema};
use datafusion::execution::SessionStateBuilder;
use datafusion_common::{DFSchema, DFSchemaRef, DataFusionError};
use datafusion_expr::{col, lit};
use datafusion_substrait::{
    logical_plan::producer::to_substrait_extended_expr, substrait::proto::ExtendedExpression,
};
use prost::Message;
use roaring::RoaringBitmap;

fn main() -> Result<(), DataFusionError> {
    let state = SessionStateBuilder::default().build();
    let expr = col("c0").lt(lit(42_i32)).and(col("c0").gt(lit(0_i32)));
    // output field
    let field = Field::new("out", DataType::Boolean, false);
    // let empty_schema = DFSchemaRef::new(DFSchema::empty());
    let schema = DFSchemaRef::new(DFSchema::try_from(Schema::new(vec![Field::new(
        "c0",
        DataType::Int32,
        true,
    )]))?);

    let substrait = to_substrait_extended_expr(&[(&expr, &field)], &schema, &state)?;
    let mut protobuf_out = Vec::<u8>::new();
    substrait
        .encode(&mut protobuf_out)
        .map_err(|e| DataFusionError::Substrait(format!("Failed to encode substrait plan: {e}")))?;
    println!("{}", protobuf_out.len());
    let start = std::time::Instant::now();
    let _decoded = ExtendedExpression::decode(&*protobuf_out).unwrap();
    println!("proto decode takes {:?}", start.elapsed());

    let numbers: Vec<u32> = (0..65535).step_by(2).collect();
    // Convert the vector into a RoaringBitmap
    let rb1 = RoaringBitmap::from_iter(numbers);
    let mut bytes = Vec::with_capacity(rb1.serialized_size());
    rb1.serialize_into(&mut bytes).unwrap();
    println!("{}", bytes.len());
    let start = std::time::Instant::now();
    let _rb2 = RoaringBitmap::deserialize_from(&bytes[..]).unwrap();
    println!("roaring decode takes {:?}", start.elapsed());
    Ok(())
}
