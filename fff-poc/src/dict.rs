use std::{any::Any, sync::Arc};

use arrow_array::{
    ArrayRef, BooleanArray, Date32Array, Float32Array, Float64Array, Int32Array, Int64Array,
    LargeStringArray, StringArray, Time32MillisecondArray, Time32SecondArray,
    Time64MicrosecondArray, Time64NanosecondArray, TimestampMicrosecondArray,
    TimestampMillisecondArray, TimestampNanosecondArray, TimestampSecondArray, UInt64Array,
};
use arrow_schema::{DataType, TimeUnit};
use dict_hash::DictHash;
use fff_core::{
    errors::{Error, Result},
    general_error, nyi_err,
};

mod bottom_k_sketch;
mod dict_hash;
pub mod shared_dictionary;
pub mod shared_dictionary_cache;
pub mod shared_dictionary_context;

#[repr(u8)]
#[derive(PartialEq, Debug, Clone, Copy)]
pub enum DictionaryTypeOptions {
    NoDictionary,
    EncoderDictionary,
    LocalDictionary,
    GlobalDictionary,
    FixedScopeDictionary(u64),
    GlobalDictionaryMultiColSharing,
    GLBest(Option<(f64, usize)>),
}

pub struct Dictionary {
    datatype: DataType,
    typed_dict: Box<dyn Any>,
}

macro_rules! typed_try_new {
    ($datatype: ident, $store_type: ty, $hash_type: ty) => {
        Ok(Self {
            $datatype,
            typed_dict: Box::new(TypedDict::<$store_type, $hash_type>::new()),
        })
    };
}

macro_rules! typed_extend {
    ($self: ident, $arr: ident, $array_type: ty, $store_type: ty, $hash_type: ty) => {{
        let typed_dict = $self
            .typed_dict
            .downcast_mut::<TypedDict<$store_type, $hash_type>>()
            .ok_or_else(|| general_error!("Dict type mismatch in extend"))?;
        let arr = $arr
            .as_any()
            .downcast_ref::<$array_type>()
            .ok_or_else(|| general_error!("Array type mismatch in extend"))?;
        typed_dict.extend(arr.into_iter());
        Ok(())
    }};
}

macro_rules! typed_extend_and_get_index {
    ($self: ident, $arr: ident, $array_type: ty, $store_type: ty, $hash_type: ty) => {{
        let typed_dict = $self
            .typed_dict
            .downcast_mut::<TypedDict<$store_type, $hash_type>>()
            .ok_or_else(|| general_error!("Dict type mismatch in extend and get index"))?;
        let arr = $arr
            .as_any()
            .downcast_ref::<$array_type>()
            .ok_or_else(|| general_error!("Array type mismatch in extend and get index"))?;
        let slice = typed_dict.extend(arr.into_iter()).to_owned();
        Ok(Arc::new(UInt64Array::from(slice)) as ArrayRef)
    }};
}

macro_rules! typed_dict_submit_values {
    ($self: ident, $arr: ident, $array_type: ty, $store_type: ty, $hash_type: ty) => {{
        let typed_dict = $self
            .typed_dict
            .downcast_mut::<TypedDict<$store_type, $hash_type>>()
            .ok_or_else(|| general_error!("Dict type mismatch in submit values"))?;
        let arr = $arr
            .as_any()
            .downcast_ref::<$array_type>()
            .ok_or_else(|| general_error!("Array type mismatch in submit values"))?;
        Ok(typed_dict.submit_values(arr.into_iter()))
    }};
}

macro_rules! typed_merge_with {
    ($self: ident, $other: ident, $store_type: ty, $hash_type: ty) => {{
        let self_typed_dict = $self
            .typed_dict
            .downcast_mut::<TypedDict<$store_type, $hash_type>>()
            .ok_or_else(|| general_error!("Self dict type mismatch in merge"))?;
        let other_typed_dict = $other
            .typed_dict
            .downcast_mut::<TypedDict<$store_type, $hash_type>>()
            .ok_or_else(|| general_error!("Other dict type mismatch in merge"))?;
        Ok(self_typed_dict.merge_with(other_typed_dict))
    }};
}

macro_rules! typed_peek_dict {
    ($self: ident, $array_type: ty, $store_type: ty, $hash_type: ty) => {{
        let typed_dict = $self
            .typed_dict
            .downcast_ref::<TypedDict<$store_type, $hash_type>>()
            .ok_or(Error::General(
                "Self dict type mismatch in peek dict".to_string(),
            ))?;
        let dict_slice = typed_dict.peek_dict().to_owned();
        Ok(Arc::new(<$array_type>::from(dict_slice)) as ArrayRef)
    }};
}

macro_rules! typed_finish {
    ($self: ident, $array_type: ty, $store_type: ty, $hash_type: ty) => {{
        let typed_dict = $self
            .typed_dict
            .downcast::<TypedDict<$store_type, $hash_type>>()
            .map_err(|_| Error::General("Dict type mismatch in finish".to_string()))?;
        let (dict_vec, index_vec) = typed_dict.finish();
        let dict_arr = <$array_type>::from(dict_vec);
        let index_arr = UInt64Array::from(index_vec);
        Ok((
            Arc::new(dict_arr) as ArrayRef,
            Arc::new(index_arr) as ArrayRef,
        ))
    }};
}

macro_rules! typed_len {
    ($self: ident, $store_type: ty, $hash_type: ty) => {{
        let typed_dict = $self
            .typed_dict
            .downcast_ref::<TypedDict<$store_type, $hash_type>>()
            .ok_or(Error::General("Dict type mismatch in len".to_string()))?;
        Ok(typed_dict.len())
    }};
}

macro_rules! typed_dict_hash_iter {
    ($self: ident, $store_type: ty, $hash_type: ty) => {{
        let typed_dict = $self
            .typed_dict
            .downcast_ref::<TypedDict<$store_type, $hash_type>>()
            .ok_or(Error::General(
                "Dict type mismatch in hash iter".to_string(),
            ))?;
        Ok(Box::new(typed_dict.hash_iter()))
    }};
}

impl Dictionary {
    pub fn try_new(datatype: DataType) -> Result<Self> {
        match datatype {
            DataType::Int32 => typed_try_new!(datatype, i32, i32),
            DataType::Int64 => typed_try_new!(datatype, i64, i64),
            DataType::Float32 => typed_try_new!(datatype, f32, u32),
            DataType::Float64 => typed_try_new!(datatype, f64, u64),
            DataType::Utf8 | DataType::LargeUtf8 => typed_try_new!(datatype, String, String),
            DataType::Timestamp(_, _) => typed_try_new!(datatype, i64, i64),
            DataType::Time32(_) => typed_try_new!(datatype, i32, i32),
            DataType::Time64(_) => typed_try_new!(datatype, i64, i64),
            DataType::Date32 => typed_try_new!(datatype, i32, i32),
            DataType::Boolean => typed_try_new!(datatype, bool, bool),
            // TODO: implement required datatypes
            // _ => Err(Error::NYI("Other datatypes are not supported".to_string())),
            _ => nyi_err!(datatype.to_string()),
        }
    }
    pub fn extend(&mut self, arr: ArrayRef) -> Result<(), Error> {
        if *arr.data_type() != self.datatype {
            return Err(Error::General("Data type mismatch".to_string()));
        }
        match self.datatype {
            DataType::Int32 => typed_extend!(self, arr, Int32Array, i32, i32),
            DataType::Int64 => typed_extend!(self, arr, Int64Array, i64, i64),
            DataType::Float32 => typed_extend!(self, arr, Float32Array, f32, u32),
            DataType::Float64 => typed_extend!(self, arr, Float64Array, f64, u64),
            DataType::Utf8 => {
                let typed_dict = self
                    .typed_dict
                    .downcast_mut::<TypedDict<String, String>>()
                    .ok_or_else(|| general_error!("Dict type mismatch in extend"))?;
                let arr = arr
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or_else(|| general_error!("Array type mismatch in extend"))?;
                // TODO: unnecessary copy of strings
                typed_dict.extend(arr.into_iter().map(|x| x.map(|y| y.to_owned())));
                Ok(())
            }
            DataType::LargeUtf8 => {
                let typed_dict = self
                    .typed_dict
                    .downcast_mut::<TypedDict<String, String>>()
                    .ok_or_else(|| general_error!("Dict type mismatch in extend"))?;
                let arr = arr
                    .as_any()
                    .downcast_ref::<LargeStringArray>()
                    .ok_or_else(|| general_error!("Array type mismatch in extend"))?;
                // TODO: unnecessary copy of strings
                typed_dict.extend(arr.into_iter().map(|x| x.map(|y| y.to_owned())));
                Ok(())
            }
            DataType::Timestamp(TimeUnit::Second, _) => {
                typed_extend!(self, arr, TimestampSecondArray, i64, i64)
            }
            DataType::Timestamp(TimeUnit::Millisecond, _) => {
                typed_extend!(self, arr, TimestampMillisecondArray, i64, i64)
            }
            DataType::Timestamp(TimeUnit::Microsecond, _) => {
                typed_extend!(self, arr, TimestampMicrosecondArray, i64, i64)
            }
            DataType::Timestamp(TimeUnit::Nanosecond, _) => {
                typed_extend!(self, arr, TimestampNanosecondArray, i64, i64)
            }
            DataType::Time32(TimeUnit::Second) => {
                typed_extend!(self, arr, Time32SecondArray, i32, i32)
            }
            DataType::Time32(TimeUnit::Millisecond) => {
                typed_extend!(self, arr, Time32MillisecondArray, i32, i32)
            }
            DataType::Time64(TimeUnit::Microsecond) => {
                typed_extend!(self, arr, Time64MicrosecondArray, i64, i64)
            }
            DataType::Time64(TimeUnit::Nanosecond) => {
                typed_extend!(self, arr, Time64NanosecondArray, i64, i64)
            }
            DataType::Date32 => typed_extend!(self, arr, Date32Array, i32, i32),
            DataType::Boolean => typed_extend!(self, arr, BooleanArray, bool, bool),
            _ => nyi_err!("Other datatypes are not supported"),
        }
    }
    pub fn extend_and_get_index(&mut self, arr: ArrayRef) -> Result<ArrayRef, Error> {
        if *arr.data_type() != self.datatype {
            return Err(Error::General("Data type mismatch".to_string()));
        }
        match self.datatype {
            DataType::Int32 => typed_extend_and_get_index!(self, arr, Int32Array, i32, i32),
            DataType::Int64 => typed_extend_and_get_index!(self, arr, Int64Array, i64, i64),
            DataType::Float32 => typed_extend_and_get_index!(self, arr, Float32Array, f32, u32),
            DataType::Float64 => typed_extend_and_get_index!(self, arr, Float64Array, f64, u64),
            DataType::Utf8 => {
                let typed_dict = self
                    .typed_dict
                    .downcast_mut::<TypedDict<String, String>>()
                    .ok_or_else(|| general_error!("Dict type mismatch in extend and get index"))?;
                let arr = arr
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or_else(|| general_error!("Array type mismatch in extend and get index"))?;
                // TODO: unnecessary copy of strings
                let slice = typed_dict
                    .extend(arr.into_iter().map(|x| x.map(|y| y.to_owned())))
                    .to_owned();
                Ok(Arc::new(UInt64Array::from(slice)) as ArrayRef)
            }
            DataType::LargeUtf8 => {
                let typed_dict = self
                    .typed_dict
                    .downcast_mut::<TypedDict<String, String>>()
                    .ok_or_else(|| general_error!("Dict type mismatch in extend and get index"))?;
                let arr = arr
                    .as_any()
                    .downcast_ref::<LargeStringArray>()
                    .ok_or_else(|| general_error!("Array type mismatch in extend and get index"))?;
                // TODO: unnecessary copy of strings
                let slice = typed_dict
                    .extend(arr.into_iter().map(|x| x.map(|y| y.to_owned())))
                    .to_owned();
                Ok(Arc::new(UInt64Array::from(slice)) as ArrayRef)
            }
            DataType::Timestamp(TimeUnit::Second, _) => {
                typed_extend_and_get_index!(self, arr, TimestampSecondArray, i64, i64)
            }
            DataType::Timestamp(TimeUnit::Millisecond, _) => {
                typed_extend_and_get_index!(self, arr, TimestampMillisecondArray, i64, i64)
            }
            DataType::Timestamp(TimeUnit::Microsecond, _) => {
                typed_extend_and_get_index!(self, arr, TimestampMicrosecondArray, i64, i64)
            }
            DataType::Timestamp(TimeUnit::Nanosecond, _) => {
                typed_extend_and_get_index!(self, arr, TimestampNanosecondArray, i64, i64)
            }
            DataType::Time32(TimeUnit::Second) => {
                typed_extend_and_get_index!(self, arr, Time32SecondArray, i32, i32)
            }
            DataType::Time32(TimeUnit::Millisecond) => {
                typed_extend_and_get_index!(self, arr, Time32MillisecondArray, i32, i32)
            }
            DataType::Time64(TimeUnit::Microsecond) => {
                typed_extend_and_get_index!(self, arr, Time64MicrosecondArray, i64, i64)
            }
            DataType::Time64(TimeUnit::Nanosecond) => {
                typed_extend_and_get_index!(self, arr, Time64NanosecondArray, i64, i64)
            }
            DataType::Date32 => typed_extend_and_get_index!(self, arr, Date32Array, i32, i32),
            DataType::Boolean => typed_extend_and_get_index!(self, arr, BooleanArray, bool, bool),
            _ => nyi_err!("Other datatypes are not supported"),
        }
    }
    pub fn submit_values(&mut self, arr: ArrayRef) -> Result<(), Error> {
        match self.datatype {
            DataType::Int32 => typed_dict_submit_values!(self, arr, Int32Array, i32, i32),
            DataType::Int64 => typed_dict_submit_values!(self, arr, Int64Array, i64, i64),
            DataType::Float32 => typed_dict_submit_values!(self, arr, Float32Array, f32, u32),
            DataType::Float64 => typed_dict_submit_values!(self, arr, Float64Array, f64, u64),
            DataType::Utf8 => {
                let typed_dict = self
                    .typed_dict
                    .downcast_mut::<TypedDict<String, String>>()
                    .ok_or(Error::General(
                        "Dict type mismatch in submit values".to_string(),
                    ))?;
                let arr = arr
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or(Error::General(
                        "Array type mismatch in submit values".to_string(),
                    ))?;
                // TODO: unnecessary copy of strings
                typed_dict.submit_values(arr.into_iter().map(|x| x.map(|y| y.to_owned())));
                Ok(())
            }
            DataType::LargeUtf8 => {
                let typed_dict = self
                    .typed_dict
                    .downcast_mut::<TypedDict<String, String>>()
                    .ok_or(Error::General(
                        "Dict type mismatch in submit values".to_string(),
                    ))?;
                let arr = arr
                    .as_any()
                    .downcast_ref::<LargeStringArray>()
                    .ok_or(Error::General(
                        "Array type mismatch in submit values".to_string(),
                    ))?;
                // TODO: unnecessary copy of strings
                typed_dict.submit_values(arr.into_iter().map(|x| x.map(|y| y.to_owned())));
                Ok(())
            }
            DataType::Timestamp(TimeUnit::Second, _) => {
                typed_dict_submit_values!(self, arr, TimestampSecondArray, i64, i64)
            }
            DataType::Timestamp(TimeUnit::Millisecond, _) => {
                typed_dict_submit_values!(self, arr, TimestampMillisecondArray, i64, i64)
            }
            DataType::Timestamp(TimeUnit::Microsecond, _) => {
                typed_dict_submit_values!(self, arr, TimestampMicrosecondArray, i64, i64)
            }
            DataType::Timestamp(TimeUnit::Nanosecond, _) => {
                typed_dict_submit_values!(self, arr, TimestampNanosecondArray, i64, i64)
            }
            DataType::Time32(TimeUnit::Second) => {
                typed_dict_submit_values!(self, arr, Time32SecondArray, i32, i32)
            }
            DataType::Time32(TimeUnit::Millisecond) => {
                typed_dict_submit_values!(self, arr, Time32MillisecondArray, i32, i32)
            }
            DataType::Time64(TimeUnit::Microsecond) => {
                typed_dict_submit_values!(self, arr, Time64MicrosecondArray, i64, i64)
            }
            DataType::Time64(TimeUnit::Nanosecond) => {
                typed_dict_submit_values!(self, arr, Time64NanosecondArray, i64, i64)
            }
            DataType::Date32 => typed_dict_submit_values!(self, arr, Date32Array, i32, i32),
            DataType::Boolean => typed_dict_submit_values!(self, arr, BooleanArray, bool, bool),
            _ => nyi_err!("Other datatypes are not supported"),
        }
    }
    pub fn merge_with(&mut self, other: &mut Self) -> Result<usize, Error> {
        match self.datatype {
            DataType::Int32 => typed_merge_with!(self, other, i32, i32),
            DataType::Int64 => typed_merge_with!(self, other, i64, i64),
            DataType::Float32 => typed_merge_with!(self, other, f32, u32),
            DataType::Float64 => typed_merge_with!(self, other, f64, u64),
            DataType::Utf8 | DataType::LargeUtf8 => typed_merge_with!(self, other, String, String),
            DataType::Timestamp(TimeUnit::Second, _) => typed_merge_with!(self, other, i64, i64),
            DataType::Timestamp(TimeUnit::Millisecond, _) => {
                typed_merge_with!(self, other, i64, i64)
            }
            DataType::Timestamp(TimeUnit::Microsecond, _) => {
                typed_merge_with!(self, other, i64, i64)
            }
            DataType::Timestamp(TimeUnit::Nanosecond, _) => {
                typed_merge_with!(self, other, i64, i64)
            }
            DataType::Time32(TimeUnit::Second) => typed_merge_with!(self, other, i32, i32),
            DataType::Time32(TimeUnit::Millisecond) => typed_merge_with!(self, other, i32, i32),
            DataType::Time64(TimeUnit::Microsecond) => typed_merge_with!(self, other, i64, i64),
            DataType::Time64(TimeUnit::Nanosecond) => typed_merge_with!(self, other, i64, i64),
            DataType::Date32 => typed_merge_with!(self, other, i32, i32),
            DataType::Boolean => typed_merge_with!(self, other, bool, bool),
            _ => nyi_err!("Other datatypes are not supported"),
        }
    }
    pub fn peek_dict(&self) -> Result<ArrayRef, Error> {
        match self.datatype {
            DataType::Int32 => typed_peek_dict!(self, Int32Array, i32, i32),
            DataType::Int64 => typed_peek_dict!(self, Int64Array, i64, i64),
            DataType::Float32 => typed_peek_dict!(self, Float32Array, f32, u32),
            DataType::Float64 => typed_peek_dict!(self, Float64Array, f64, u64),
            DataType::Utf8 => {
                typed_peek_dict!(self, StringArray, String, String)
            }
            DataType::LargeUtf8 => {
                typed_peek_dict!(self, LargeStringArray, String, String)
            }
            DataType::Timestamp(TimeUnit::Second, _) => {
                typed_peek_dict!(self, TimestampSecondArray, i64, i64)
            }
            DataType::Timestamp(TimeUnit::Millisecond, _) => {
                typed_peek_dict!(self, TimestampMillisecondArray, i64, i64)
            }
            DataType::Timestamp(TimeUnit::Microsecond, _) => {
                typed_peek_dict!(self, TimestampMicrosecondArray, i64, i64)
            }
            DataType::Timestamp(TimeUnit::Nanosecond, _) => {
                typed_peek_dict!(self, TimestampNanosecondArray, i64, i64)
            }
            DataType::Time32(TimeUnit::Second) => {
                typed_peek_dict!(self, Time32SecondArray, i32, i32)
            }
            DataType::Time32(TimeUnit::Millisecond) => {
                typed_peek_dict!(self, Time32MillisecondArray, i32, i32)
            }
            DataType::Time64(TimeUnit::Microsecond) => {
                typed_peek_dict!(self, Time64MicrosecondArray, i64, i64)
            }
            DataType::Time64(TimeUnit::Nanosecond) => {
                typed_peek_dict!(self, Time64NanosecondArray, i64, i64)
            }
            DataType::Date32 => typed_peek_dict!(self, Date32Array, i32, i32),
            DataType::Boolean => typed_peek_dict!(self, BooleanArray, bool, bool),
            _ => nyi_err!("Other datatypes are not supported"),
        }
    }
    pub fn finish(self) -> Result<(ArrayRef, ArrayRef), Error> {
        match self.datatype {
            DataType::Int32 => typed_finish!(self, Int32Array, i32, i32),
            DataType::Int64 => typed_finish!(self, Int64Array, i64, i64),
            DataType::Float32 => typed_finish!(self, Float32Array, f32, u32),
            DataType::Float64 => typed_finish!(self, Float64Array, f64, u64),
            DataType::Utf8 => {
                typed_finish!(self, StringArray, String, String)
            }
            DataType::LargeUtf8 => {
                typed_finish!(self, LargeStringArray, String, String)
            }
            DataType::Timestamp(TimeUnit::Second, _) => {
                typed_finish!(self, TimestampSecondArray, i64, i64)
            }
            DataType::Timestamp(TimeUnit::Millisecond, _) => {
                typed_finish!(self, TimestampMillisecondArray, i64, i64)
            }
            DataType::Timestamp(TimeUnit::Microsecond, _) => {
                typed_finish!(self, TimestampMicrosecondArray, i64, i64)
            }
            DataType::Timestamp(TimeUnit::Nanosecond, _) => {
                typed_finish!(self, TimestampNanosecondArray, i64, i64)
            }
            DataType::Time32(TimeUnit::Second) => {
                typed_finish!(self, Time32SecondArray, i32, i32)
            }
            DataType::Time32(TimeUnit::Millisecond) => {
                typed_finish!(self, Time32MillisecondArray, i32, i32)
            }
            DataType::Time64(TimeUnit::Microsecond) => {
                typed_finish!(self, Time64MicrosecondArray, i64, i64)
            }
            DataType::Time64(TimeUnit::Nanosecond) => {
                typed_finish!(self, Time64NanosecondArray, i64, i64)
            }
            DataType::Date32 => typed_finish!(self, Date32Array, i32, i32),
            DataType::Boolean => typed_finish!(self, BooleanArray, bool, bool),
            _ => nyi_err!("Other datatypes are not supported"),
        }
    }
    pub fn len(&self) -> Result<usize, Error> {
        match self.datatype {
            DataType::Int32 => typed_len!(self, i32, i32),
            DataType::Int64 => typed_len!(self, i64, i64),
            DataType::Float32 => typed_len!(self, f32, u32),
            DataType::Float64 => typed_len!(self, f64, u64),
            DataType::Utf8 | DataType::LargeUtf8 => typed_len!(self, String, String),
            DataType::Timestamp(TimeUnit::Second, _) => typed_len!(self, i64, i64),
            DataType::Timestamp(TimeUnit::Millisecond, _) => typed_len!(self, i64, i64),
            DataType::Timestamp(TimeUnit::Microsecond, _) => typed_len!(self, i64, i64),
            DataType::Timestamp(TimeUnit::Nanosecond, _) => typed_len!(self, i64, i64),
            DataType::Time32(TimeUnit::Second) => typed_len!(self, i32, i32),
            DataType::Time32(TimeUnit::Millisecond) => typed_len!(self, i32, i32),
            DataType::Time64(TimeUnit::Microsecond) => typed_len!(self, i64, i64),
            DataType::Time64(TimeUnit::Nanosecond) => typed_len!(self, i64, i64),
            DataType::Date32 => typed_len!(self, i32, i32),
            DataType::Boolean => typed_len!(self, bool, bool),
            _ => nyi_err!("Other datatypes are not supported"),
        }
    }
    pub fn dict_hash_iter(&self) -> Result<Box<dyn Iterator<Item = u64> + '_>, Error> {
        match self.datatype {
            DataType::Int32 => typed_dict_hash_iter!(self, i32, i32),
            DataType::Int64 => typed_dict_hash_iter!(self, i64, i64),
            DataType::Float32 => typed_dict_hash_iter!(self, f32, u32),
            DataType::Float64 => typed_dict_hash_iter!(self, f64, u64),
            DataType::Utf8 | DataType::LargeUtf8 => typed_dict_hash_iter!(self, String, String),
            DataType::Timestamp(TimeUnit::Second, _) => typed_dict_hash_iter!(self, i64, i64),
            DataType::Timestamp(TimeUnit::Millisecond, _) => typed_dict_hash_iter!(self, i64, i64),
            DataType::Timestamp(TimeUnit::Microsecond, _) => typed_dict_hash_iter!(self, i64, i64),
            DataType::Timestamp(TimeUnit::Nanosecond, _) => typed_dict_hash_iter!(self, i64, i64),
            DataType::Time32(TimeUnit::Second) => typed_dict_hash_iter!(self, i32, i32),
            DataType::Time32(TimeUnit::Millisecond) => typed_dict_hash_iter!(self, i32, i32),
            DataType::Time64(TimeUnit::Microsecond) => typed_dict_hash_iter!(self, i64, i64),
            DataType::Time64(TimeUnit::Nanosecond) => typed_dict_hash_iter!(self, i64, i64),
            DataType::Date32 => typed_dict_hash_iter!(self, i32, i32),
            DataType::Boolean => typed_dict_hash_iter!(self, bool, bool),
            _ => nyi_err!("Other datatypes are not supported"),
        }
    }
}

struct TypedDict<T: Clone + DictHash<U>, U: Ord + std::hash::Hash + Clone> {
    dictionary: Vec<T>,
    indices: Vec<Option<u64>>,
    key_to_ind: std::collections::HashMap<U, u64>,
}

impl<T: Clone + DictHash<U>, U: Ord + std::hash::Hash + Clone> TypedDict<T, U> {
    pub fn new() -> Self {
        TypedDict {
            indices: Vec::new(),
            dictionary: Vec::new(),
            key_to_ind: std::collections::HashMap::new(),
        }
    }
    pub fn len(&self) -> usize {
        self.dictionary.len()
    }
    pub fn extend(&mut self, arr: impl Iterator<Item = Option<T>>) -> &[Option<u64>] {
        let slice_begin_idx = self.indices.len();
        for val in arr {
            if let Some(val) = val {
                let hash_val = T::dict_hash(&val);
                if let Some(ind) = self.key_to_ind.get(&hash_val) {
                    self.indices.push(Some(*ind));
                } else {
                    let ind = self.dictionary.len();
                    self.key_to_ind.insert(hash_val, ind as u64);
                    self.indices.push(Some(ind as u64));
                    self.dictionary.push(val.clone());
                }
            } else {
                self.indices.push(None)
            }
        }
        &self.indices[slice_begin_idx..]
    }
    pub fn submit_values(&mut self, arr: impl Iterator<Item = Option<T>>) {
        for val in arr.flatten() {
            let hash_val = T::dict_hash(&val);
            if let std::collections::hash_map::Entry::Vacant(e) = self.key_to_ind.entry(hash_val) {
                let ind = self.dictionary.len();
                e.insert(ind as u64);
                self.dictionary.push(val.clone());
            }
        }
    }
    fn submit_values_no_null<'a>(&mut self, arr: impl Iterator<Item = &'a T>)
    where
        T: 'a,
    {
        for val in arr {
            let hash_val = T::dict_hash(val);
            if let std::collections::hash_map::Entry::Vacant(e) = self.key_to_ind.entry(hash_val) {
                let ind = self.dictionary.len();
                e.insert(ind as u64);
                self.dictionary.push(val.clone());
            }
        }
    }
    pub fn merge_with(&mut self, other: &mut Self) -> usize {
        let mut common_values = vec![];
        for val in &other.dictionary {
            let hash_val = T::dict_hash(val);
            if self.key_to_ind.contains_key(&hash_val) {
                common_values.push(val.clone());
            }
        }
        if common_values.is_empty() {
            return 0; // Discard merging if the two dictionaries do not overlap
        }
        let self_dict = std::mem::take(&mut self.dictionary);
        let other_dict = std::mem::take(&mut other.dictionary);
        self.key_to_ind.clear();
        other.key_to_ind.clear();
        self.submit_values_no_null(common_values.iter());
        self.submit_values_no_null(self_dict.iter());
        other.submit_values_no_null(common_values.iter());
        other.submit_values_no_null(other_dict.iter());
        common_values.len()
    }
    pub fn peek_dict(&self) -> &[T] {
        &self.dictionary
    }
    pub fn finish(self) -> (Vec<T>, Vec<Option<u64>>) {
        (self.dictionary, self.indices)
    }
    pub fn hash_iter(&self) -> Box<dyn Iterator<Item = u64> + '_> {
        Box::new(self.dictionary.iter().map(|x| T::hash_to_u64(x)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_dict() {
        let values = Int32Array::from_iter([1, 2, 3, 1, 2, 3, 1, 1, 3]);
        let mut dict = Dictionary::try_new(DataType::Int32).unwrap();
        dict.extend(Arc::new(values) as ArrayRef).unwrap();
        let (dict, indices) = dict.finish().unwrap();
        assert_eq!(
            format!(
                "{:?}",
                dict.as_any()
                    .downcast_ref::<Int32Array>()
                    .unwrap()
                    .into_iter()
                    .collect::<Vec<_>>()
            ),
            format!("{:?}", [1, 2, 3].map(Some))
        );
        assert_eq!(
            format!(
                "{:?}",
                indices
                    .as_any()
                    .downcast_ref::<UInt64Array>()
                    .unwrap()
                    .into_iter()
                    .collect::<Vec<_>>()
            ),
            format!("{:?}", [0, 1, 2, 0, 1, 2, 0, 0, 2].map(Some))
        );
    }

    #[test]
    fn test_dict_merge() {
        let values_1 = Arc::new(Int32Array::from_iter([11, 1, 3, 4, 2, 6, 4, 3])) as ArrayRef;
        let values_2 = Arc::new(Int32Array::from_iter([3, 5, 1, 2, 19, 2, 6])) as ArrayRef;
        // Common values: 3, 1, 2, 6 (following the order of the second dict)
        let mut dict_1 = Dictionary::try_new(DataType::Int32).unwrap();
        let mut dict_2 = Dictionary::try_new(DataType::Int32).unwrap();
        dict_1.submit_values(values_1.clone()).unwrap();
        dict_2.submit_values(values_2.clone()).unwrap();
        assert_eq!(dict_1.merge_with(&mut dict_2).unwrap(), 4);
        dict_1.extend(values_1).unwrap();
        dict_2.extend(values_2).unwrap();
        let (dict_1, indices_1) = dict_1.finish().unwrap();
        let (dict_2, indices_2) = dict_2.finish().unwrap();
        assert_eq!(
            format!(
                "{:?}",
                dict_1
                    .as_any()
                    .downcast_ref::<Int32Array>()
                    .unwrap()
                    .into_iter()
                    .collect::<Vec<_>>()
            ),
            format!("{:?}", [3, 1, 2, 6, 11, 4].map(Some))
        );
        assert_eq!(
            format!(
                "{:?}",
                dict_2
                    .as_any()
                    .downcast_ref::<Int32Array>()
                    .unwrap()
                    .into_iter()
                    .collect::<Vec<_>>()
            ),
            format!("{:?}", [3, 1, 2, 6, 5, 19].map(Some))
        );
        assert_eq!(
            format!(
                "{:?}",
                indices_1
                    .as_any()
                    .downcast_ref::<UInt64Array>()
                    .unwrap()
                    .into_iter()
                    .collect::<Vec<_>>()
            ),
            format!("{:?}", [4, 1, 0, 5, 2, 3, 5, 0].map(Some))
        );
        assert_eq!(
            format!(
                "{:?}",
                indices_2
                    .as_any()
                    .downcast_ref::<UInt64Array>()
                    .unwrap()
                    .into_iter()
                    .collect::<Vec<_>>()
            ),
            format!("{:?}", [0, 4, 1, 2, 5, 2, 3].map(Some))
        );
    }
}
