use std::default::Default;

use crate::options::DictionaryTypeOptions;

#[derive(Clone, Debug)]
pub struct EncodingCounter {
    pub dict_type: DictionaryTypeOptions,
    pub dict_size: usize,
    pub index_size: usize,
}

impl EncodingCounter {
    pub fn add(&mut self, other: &Self) -> &mut Self {
        self.dict_size += other.dict_size;
        self.index_size += other.index_size;
        self
    }

    pub fn total_size(&self) -> usize {
        self.dict_size + self.index_size
    }
}

impl Default for EncodingCounter {
    fn default() -> Self {
        Self {
            dict_type: DictionaryTypeOptions::EncoderDictionary,
            dict_size: 0,
            index_size: 0,
        }
    }
}
