use crate::enc_unit::FlatEncUnit;

/// TODO: to mimic the behavior on GPU, we need to explicitly traverse the EncodingTree from leaf nodes up to the root.
/// This is because the GPU does not support recursion.
/// The intermediate buffers need to be managed and not dynamically reused.
pub struct CascadeDecoder {
    flat_enc_unit: FlatEncUnit,
}
