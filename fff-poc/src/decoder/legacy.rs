pub struct PlainEncUnitDecoder {
    data: Bytes,
    num_rows: u64,
    output_type: DataType,
}

impl PlainEncUnitDecoder {
    pub fn new(data: Bytes, num_rows: u64, output_type: DataType) -> Self {
        Self {
            data,
            num_rows,
            output_type,
        }
    }
}

impl EncUnitDecoder for PlainEncUnitDecoder {
    fn decode(&self) -> Result<ArrayRef> {
        let flat_enc_unit = FlatEncUnit::try_deserialize(self.data.clone()).unwrap();
        let mut buffers = vec![Buffer::from_vec::<u8>(vec![])];
        buffers.extend(
            PlainDecoder::new(flat_enc_unit.buffers()[0].try_to_dense().unwrap()).decode_all()?,
        );
        Ok(primitive_array_from_arrow_buffers(
            &self.output_type,
            buffers,
            self.num_rows,
        )?)
    }
}

pub struct NullableEncUnitDecoder {
    data: Bytes,
    output_type: DataType,
    num_rows: u64,
}

impl NullableEncUnitDecoder {
    pub fn new(data: Bytes, num_rows: u64, output_type: DataType) -> Self {
        Self {
            data,
            num_rows,
            output_type,
        }
    }
}

impl EncUnitDecoder for NullableEncUnitDecoder {
    fn decode(&self) -> Result<ArrayRef> {
        let flat_enc_unit = FlatEncUnit::try_deserialize(self.data.clone()).unwrap();
        let mut dec = NullableDecoder::new(
            flat_enc_unit.buffers()[0].try_to_dense().unwrap(),
            flat_enc_unit.buffers()[1].try_to_dense().unwrap(),
        );
        let buffers = dec.decode_all()?;

        Ok(primitive_array_from_arrow_buffers(
            &self.output_type,
            buffers,
            self.num_rows,
        )?)
    }
}
