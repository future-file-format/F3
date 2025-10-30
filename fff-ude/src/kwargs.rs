use std::collections::HashMap;

/// kwargs used for decoding an EncUnit. Check format/kwargs.md for details.
///
use rkyv::{rancor::Error, Archive, Deserialize, Serialize};

/// num_keys (i32)
/// key_lens (i32 * num_keys)
/// word_lens (i32 * num_keys)
/// key-word * num_keys (var len)
pub fn kwargs_serialize(kwargs: &[(&[u8], &[u8])]) -> Vec<u8> {
    let num_keys = kwargs.len() as i32;
    let mut key_lens = Vec::with_capacity(kwargs.len());
    let mut word_lens = Vec::with_capacity(kwargs.len());

    for &(key, word) in kwargs {
        key_lens.push(key.len() as i32);
        word_lens.push(word.len() as i32);
    }

    let mut result = Vec::new();

    // Serialize num_keys
    result.extend_from_slice(&num_keys.to_le_bytes());

    // Serialize key_lens array
    for len in &key_lens {
        result.extend_from_slice(&len.to_le_bytes());
    }

    // Serialize word_lens array
    for len in &word_lens {
        result.extend_from_slice(&len.to_le_bytes());
    }

    // Serialize each key and word in order
    for &(key, word) in kwargs {
        result.extend_from_slice(key);
        result.extend_from_slice(word);
    }

    result
}

pub fn kwargs_deserialize(bytes: &[u8]) -> HashMap<&[u8], &[u8]> {
    // Read the number of keys (first 4 bytes as i32)
    let num_keys = i32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;

    // Parse key lengths array
    let key_lens_start = 4;
    let key_lens_end = key_lens_start + num_keys * 4;
    let key_lens: Vec<i32> = bytes[key_lens_start..key_lens_end]
        .chunks_exact(4)
        .map(|chunk| i32::from_le_bytes(chunk.try_into().unwrap()))
        .collect();

    // Parse word lengths array
    let word_lens_start = key_lens_end;
    let word_lens_end = word_lens_start + num_keys * 4;
    let word_lens: Vec<i32> = bytes[word_lens_start..word_lens_end]
        .chunks_exact(4)
        .map(|chunk| i32::from_le_bytes(chunk.try_into().unwrap()))
        .collect();

    let mut data_offset = word_lens_end;
    let mut result = HashMap::with_capacity(num_keys);

    for i in 0..num_keys {
        let key_len = key_lens[i] as usize;
        let word_len = word_lens[i] as usize;

        // Extract key bytes
        let key_start = data_offset;
        let key_end = key_start + key_len;
        let key = &bytes[key_start..key_end];
        data_offset = key_end;

        // Extract word bytes
        let word_start = data_offset;
        let word_end = word_start + word_len;
        let word = &bytes[word_start..word_end];
        data_offset = word_end;

        result.insert(key, word);
    }

    result
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Hash, Archive, Deserialize, Serialize)]
#[rkyv(
    // This will generate a PartialEq impl between our unarchived
    // and archived types
    compare(PartialEq),
    // Derives can be passed through to the generated type:
    derive(Debug),
    derive(PartialEq),
)]
pub enum Operator {
    // comparison
    Eq,
    NotEq,
    Gt,
    Gte,
    Lt,
    Lte,
    // boolean algebra
    And,
    Or,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Hash, Archive, Deserialize, Serialize)]
#[rkyv(
    // This will generate a PartialEq impl between our unarchived
    // and archived types
    compare(PartialEq),
    // Derives can be passed through to the generated type:
    derive(PartialEq),
    derive(Debug),
)]
pub enum ScalarValue {
    Null,
    I32(i32),
}

impl ArchivedScalarValue {
    pub fn as_i32(&self) -> Option<i32> {
        match self {
            ArchivedScalarValue::I32(i) => Some(i.to_native()),
            _ => None,
        }
    }
}

#[derive(Archive, Deserialize, Serialize)]
pub struct PPDExpr {
    op: Operator,
    right: ScalarValue,
}

impl PPDExpr {
    pub fn new(op: Operator, right: ScalarValue) -> Self {
        Self { op, right }
    }
}

impl ArchivedPPDExpr {
    pub fn op(&self) -> &ArchivedOperator {
        &self.op
    }

    pub fn right(&self) -> &ArchivedScalarValue {
        &self.right
    }
}

/// A naive implementation of PPD serialization
///
pub fn ppd_serialize(expr: PPDExpr) -> Vec<u8> {
    let bytes = rkyv::to_bytes::<Error>(&expr).unwrap();
    bytes.into_vec()
}

pub fn ppd_deserialize(bytes: &[u8]) -> &ArchivedPPDExpr {
    rkyv::access::<ArchivedPPDExpr, Error>(bytes).unwrap()
}
