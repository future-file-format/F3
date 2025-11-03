// Vendored from lance and modified to work with fff
// Provide utility functions to convert Arrow Buffers (potentially from Wasm) to Arrow Arrays.

// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Lance Authors

use std::sync::Arc;

use arrow_array::{
    new_null_array,
    types::{
        ArrowPrimitiveType, BinaryViewType, ByteArrayType, ByteViewType, Date32Type, Date64Type,
        Decimal128Type, Decimal256Type, DurationMicrosecondType, DurationMillisecondType,
        DurationNanosecondType, DurationSecondType, Float16Type, Float32Type, Float64Type,
        GenericBinaryType, GenericStringType, Int16Type, Int32Type, Int64Type, Int8Type,
        IntervalDayTimeType, IntervalMonthDayNanoType, IntervalYearMonthType, StringViewType,
        Time32MillisecondType, Time32SecondType, Time64MicrosecondType, Time64NanosecondType,
        TimestampMicrosecondType, TimestampMillisecondType, TimestampNanosecondType,
        TimestampSecondType, UInt16Type, UInt32Type, UInt64Type, UInt8Type,
    },
    ArrayRef, BooleanArray, FixedSizeBinaryArray, FixedSizeListArray, GenericByteArray,
    GenericByteViewArray, GenericListArray, NullArray, OffsetSizeTrait, PrimitiveArray,
};
use arrow_buffer::{BooleanBuffer, Buffer, NullBuffer, OffsetBuffer, ScalarBuffer};
use arrow_schema::{DataType, Field, FieldRef, IntervalUnit, TimeUnit};
use bytes::BytesMut;
use snafu::location;

use crate::errors::{Error, Result};
use lazy_static::lazy_static;

pub fn new_primitive_array<T: ArrowPrimitiveType>(
    buffers: Vec<BytesMut>,
    num_rows: u64,
    data_type: &DataType,
) -> ArrayRef {
    let mut buffer_iter = buffers.into_iter();
    let null_buffer = buffer_iter.next().unwrap();
    let null_buffer = if null_buffer.is_empty() {
        None
    } else {
        let null_buffer = null_buffer.freeze().into();
        Some(NullBuffer::new(BooleanBuffer::new(
            Buffer::from_bytes(null_buffer),
            0,
            num_rows as usize,
        )))
    };

    let data_buffer = buffer_iter.next().unwrap().freeze();
    let data_buffer = Buffer::from_bytes(data_buffer.into());
    let data_buffer = ScalarBuffer::<T::Native>::new(data_buffer, 0, num_rows as usize);

    // The with_data_type is needed here to recover the parameters for types like Decimal/Timestamp
    Arc::new(PrimitiveArray::<T>::new(data_buffer, null_buffer).with_data_type(data_type.clone()))
}

pub fn new_primitive_array_from_arrow_buffer<T: ArrowPrimitiveType>(
    buffers: Vec<Buffer>,
    num_rows: u64,
    data_type: &DataType,
) -> ArrayRef {
    let buffer_iter = buffers.into_iter();
    new_primitive_array_from_arrow_buffer_iter::<T>(buffer_iter, num_rows, data_type)
}

pub fn new_primitive_array_from_arrow_buffer_iter<T: ArrowPrimitiveType>(
    mut buffer_iter: impl Iterator<Item = Buffer>,
    num_rows: u64,
    data_type: &DataType,
) -> ArrayRef {
    let null_buffer = buffer_iter.next().unwrap();
    let null_buffer = arrow_buffer_to_validity(null_buffer, num_rows);

    let data_buffer = buffer_iter.next().unwrap();
    let data_buffer = ScalarBuffer::<T::Native>::new(data_buffer, 0, num_rows as usize);

    // The with_data_type is needed here to recover the parameters for types like Decimal/Timestamp
    Arc::<_>::new(
        PrimitiveArray::<T>::new(data_buffer, null_buffer).with_data_type(data_type.clone()),
    )
}

pub fn new_generic_byte_array<T: ByteArrayType>(buffers: Vec<BytesMut>, num_rows: u64) -> ArrayRef {
    // iterate over buffers to get offsets and then bytes
    let mut buffer_iter = buffers.into_iter();

    let null_buffer = buffer_iter.next().unwrap();
    let null_buffer = if null_buffer.is_empty() {
        None
    } else {
        let null_buffer = null_buffer.freeze().into();
        Some(NullBuffer::new(BooleanBuffer::new(
            Buffer::from_bytes(null_buffer),
            0,
            num_rows as usize,
        )))
    };

    let indices_bytes = buffer_iter.next().unwrap().freeze();
    let indices_buffer = Buffer::from_bytes(indices_bytes.into());
    let indices_buffer = ScalarBuffer::<T::Offset>::new(indices_buffer, 0, num_rows as usize + 1);

    let offsets = OffsetBuffer::new(indices_buffer.clone());

    // Decoding the bytes creates 2 buffers, the first one is empty since
    // validity is stored in an earlier buffer
    buffer_iter.next().unwrap();

    let bytes_buffer = buffer_iter.next().unwrap().freeze();
    let bytes_buffer = Buffer::from_bytes(bytes_buffer.into());
    let bytes_buffer_len = bytes_buffer.len();
    let bytes_buffer = ScalarBuffer::<u8>::new(bytes_buffer, 0, bytes_buffer_len);

    let bytes_array = Arc::new(
        PrimitiveArray::<UInt8Type>::new(bytes_buffer, None).with_data_type(DataType::UInt8),
    );

    Arc::new(GenericByteArray::<T>::new(
        offsets,
        bytes_array.values().inner().clone(),
        null_buffer,
    ))
}

pub fn new_generic_byte_array_from_arrow_buffers<T: ByteArrayType>(
    buffers: Vec<Buffer>,
    num_rows: u64,
) -> ArrayRef {
    // iterate over buffers to get offsets and then bytes
    let buffer_iter = buffers.into_iter();

    new_generic_byte_array_from_arrow_buffer_iter::<T>(buffer_iter, num_rows)
}

pub fn new_generic_byte_array_from_arrow_buffer_iter<T: ByteArrayType>(
    mut buffer_iter: impl Iterator<Item = Buffer>,
    num_rows: u64,
) -> ArrayRef {
    let null_buffer = buffer_iter.next().unwrap();
    let null_buffer = if null_buffer.is_empty() {
        None
    } else {
        Some(NullBuffer::new(BooleanBuffer::new(
            null_buffer,
            0,
            num_rows as usize,
        )))
    };

    let indices_buffer = buffer_iter.next().unwrap();
    let indices_buffer = ScalarBuffer::<T::Offset>::new(indices_buffer, 0, num_rows as usize + 1);
    // for x in indices_buffer.windows(2) {
    //     if x[0] > x[1] {
    //         println!("{:?}", x);
    //     }
    // }
    let offsets = OffsetBuffer::new(indices_buffer.clone());

    let bytes_buffer = buffer_iter.next().unwrap();
    let bytes_buffer_len = bytes_buffer.len();
    let bytes_buffer = ScalarBuffer::<u8>::new(bytes_buffer, 0, bytes_buffer_len);

    let bytes_array = Arc::new(
        PrimitiveArray::<UInt8Type>::new(bytes_buffer, None).with_data_type(DataType::UInt8),
    );

    Arc::new(GenericByteArray::<T>::new(
        offsets,
        bytes_array.values().inner().clone(),
        null_buffer,
    ))
}

pub fn new_generic_byte_view_array_from_arrow_buffer_iter<T: ByteViewType>(
    mut buffer_iter: impl Iterator<Item = Buffer>,
    num_rows: u64,
) -> ArrayRef {
    let null_buffer = buffer_iter.next().unwrap();
    let null_buffer = if null_buffer.is_empty() {
        None
    } else {
        Some(NullBuffer::new(BooleanBuffer::new(
            null_buffer,
            0,
            num_rows as usize,
        )))
    };

    let views_buffer = buffer_iter.next().unwrap();
    let views_buffer = ScalarBuffer::<u128>::new(views_buffer, 0, num_rows as usize);

    // TODO: Safety: Vortex also does not validate utf8 in their to_arrow function from varbin so this may be fine.
    Arc::new(unsafe {
        GenericByteViewArray::<T>::new_unchecked(views_buffer, buffer_iter.collect(), null_buffer)
    })
}

pub fn bytes_to_validity(bytes: BytesMut, num_rows: u64) -> Option<NullBuffer> {
    if bytes.is_empty() {
        None
    } else {
        let null_buffer = bytes.freeze().into();
        Some(NullBuffer::new(BooleanBuffer::new(
            Buffer::from_bytes(null_buffer),
            0,
            num_rows as usize,
        )))
    }
}

pub fn arrow_buffer_to_validity(buffer: Buffer, num_rows: u64) -> Option<NullBuffer> {
    if buffer.is_empty() {
        None
    } else {
        Some(NullBuffer::new(BooleanBuffer::new(
            buffer,
            0,
            num_rows as usize,
        )))
    }
}

/// CAUTION: only offsets and validity are valid for this List Array! Items are just dummy nulls.
pub fn new_list_offsets_validity<T: ArrowPrimitiveType>(
    buffers: Vec<BytesMut>,
    num_rows: u64,
    _child: FieldRef,
) -> ArrayRef
where
    <T as ArrowPrimitiveType>::Native: OffsetSizeTrait,
    usize: TryFrom<<T as ArrowPrimitiveType>::Native>,
{
    let mut buffer_iter = buffers.into_iter();
    let null_buffer = buffer_iter.next().unwrap();
    let null_buffer = if null_buffer.is_empty() {
        None
    } else {
        let null_buffer = null_buffer.freeze().into();
        Some(NullBuffer::new(BooleanBuffer::new(
            Buffer::from_bytes(null_buffer),
            0,
            num_rows as usize,
        )))
    };

    let data_buffer = buffer_iter.next().unwrap().freeze();
    let data_buffer = Buffer::from_bytes(data_buffer.into());
    let data_buffer = ScalarBuffer::<T::Native>::new(data_buffer, 0, num_rows as usize + 1);
    let max_offset: T::Native = data_buffer[num_rows as usize];
    let dummy_array = Arc::new(NullArray::new(usize::try_from(max_offset).unwrap_or_else(
        |_| panic!("usize conversion failed in new_list_offsets_validity"),
    )));
    Arc::new(GenericListArray::<T::Native>::new(
        // CAUTION: _child is no longer used here because here we simply put a Null dummy can let values decoder to handle items.
        DUMMY_NULL_FIELD.clone(),
        OffsetBuffer::new(data_buffer),
        dummy_array,
        null_buffer,
    ))
}

/// CAUTION: only offsets and validity are valid for this List Array! Items are just dummy nulls.
pub fn new_list_offsets_validity_from_buffers<T: ArrowPrimitiveType>(
    buffers: Vec<Buffer>,
    num_rows: u64,
    child: Option<ArrayRef>,
) -> ArrayRef
where
    <T as ArrowPrimitiveType>::Native: OffsetSizeTrait,
    usize: TryFrom<<T as ArrowPrimitiveType>::Native>,
{
    let buffer_iter = buffers.into_iter();
    new_list_offsets_validity_from_buffer_iter::<T>(buffer_iter, num_rows, child)
}

lazy_static! {
    pub static ref DUMMY_NULL_FIELD: Arc<Field> =
        Arc::new(Field::new("dummy", DataType::Null, true));
}

pub fn new_list_offsets_validity_from_buffer_iter<T: ArrowPrimitiveType>(
    mut buffer_iter: impl Iterator<Item = Buffer>,
    num_rows: u64,
    child: Option<ArrayRef>,
) -> ArrayRef
where
    <T as ArrowPrimitiveType>::Native: OffsetSizeTrait,
    usize: TryFrom<<T as ArrowPrimitiveType>::Native>,
{
    let null_buffer = buffer_iter.next().unwrap();
    let null_buffer = if null_buffer.is_empty() {
        None
    } else {
        Some(NullBuffer::new(BooleanBuffer::new(
            null_buffer,
            0,
            num_rows as usize,
        )))
    };

    let data_buffer = buffer_iter.next().unwrap();
    let data_buffer = ScalarBuffer::<T::Native>::new(data_buffer, 0, num_rows as usize + 1);
    let max_offset: T::Native = data_buffer[num_rows as usize];
    let dummy_array = Arc::new(NullArray::new(usize::try_from(max_offset).unwrap_or_else(
        |_| panic!("usize conversion failed in new_list_offsets_validity"),
    )));
    Arc::new(GenericListArray::<T::Native>::new(
        if let Some(ref child) = child {
            Arc::new(Field::new(
                "dummy",
                child.data_type().clone(),
                child.is_nullable(),
            ))
        } else {
            DUMMY_NULL_FIELD.clone()
        },
        OffsetBuffer::new(data_buffer),
        if let Some(child) = child {
            child
        } else {
            dummy_array
        },
        null_buffer,
    ))
}

pub fn primitive_array_from_arrow_buffers(
    data_type: &DataType,
    buffers: Vec<Buffer>,
    num_rows: u64,
) -> Result<ArrayRef> {
    let buffer_iter = buffers.into_iter();
    primitive_array_from_arrow_buffers_iter(data_type, buffer_iter, num_rows)
}

pub fn primitive_array_from_arrow_buffers_iter(
    data_type: &DataType,
    mut buffer_iter: impl Iterator<Item = Buffer>,
    num_rows: u64,
) -> Result<ArrayRef> {
    match data_type {
        DataType::Boolean => {
            let null_buffer = buffer_iter.next().unwrap();
            let null_buffer = arrow_buffer_to_validity(null_buffer, num_rows);

            let data_buffer = buffer_iter.next().unwrap();
            let data_buffer = data_buffer;
            let data_buffer = BooleanBuffer::new(data_buffer, 0, num_rows as usize);

            Ok(Arc::new(BooleanArray::new(data_buffer, null_buffer)))
        }
        DataType::Date32 => Ok(new_primitive_array_from_arrow_buffer_iter::<Date32Type>(
            buffer_iter,
            num_rows,
            data_type,
        )),
        DataType::Date64 => Ok(new_primitive_array_from_arrow_buffer_iter::<Date64Type>(
            buffer_iter,
            num_rows,
            data_type,
        )),
        DataType::Decimal128(_, _) => Ok(new_primitive_array_from_arrow_buffer_iter::<
            Decimal128Type,
        >(buffer_iter, num_rows, data_type)),
        DataType::Decimal256(_, _) => Ok(new_primitive_array_from_arrow_buffer_iter::<
            Decimal256Type,
        >(buffer_iter, num_rows, data_type)),
        DataType::Duration(units) => Ok(match units {
            TimeUnit::Second => new_primitive_array_from_arrow_buffer_iter::<DurationSecondType>(
                buffer_iter,
                num_rows,
                data_type,
            ),
            TimeUnit::Microsecond => new_primitive_array_from_arrow_buffer_iter::<
                DurationMicrosecondType,
            >(buffer_iter, num_rows, data_type),
            TimeUnit::Millisecond => new_primitive_array_from_arrow_buffer_iter::<
                DurationMillisecondType,
            >(buffer_iter, num_rows, data_type),
            TimeUnit::Nanosecond => new_primitive_array_from_arrow_buffer_iter::<
                DurationNanosecondType,
            >(buffer_iter, num_rows, data_type),
        }),
        DataType::Float16 => Ok(new_primitive_array_from_arrow_buffer_iter::<Float16Type>(
            buffer_iter,
            num_rows,
            data_type,
        )),
        DataType::Float32 => Ok(new_primitive_array_from_arrow_buffer_iter::<Float32Type>(
            buffer_iter,
            num_rows,
            data_type,
        )),
        DataType::Float64 => Ok(new_primitive_array_from_arrow_buffer_iter::<Float64Type>(
            buffer_iter,
            num_rows,
            data_type,
        )),
        DataType::Int16 => Ok(new_primitive_array_from_arrow_buffer_iter::<Int16Type>(
            buffer_iter,
            num_rows,
            data_type,
        )),
        DataType::Int32 => Ok(new_primitive_array_from_arrow_buffer_iter::<Int32Type>(
            buffer_iter,
            num_rows,
            data_type,
        )),
        DataType::Int64 => Ok(new_primitive_array_from_arrow_buffer_iter::<Int64Type>(
            buffer_iter,
            num_rows,
            data_type,
        )),
        DataType::Int8 => Ok(new_primitive_array_from_arrow_buffer_iter::<Int8Type>(
            buffer_iter,
            num_rows,
            data_type,
        )),
        DataType::Interval(unit) => Ok(match unit {
            IntervalUnit::DayTime => new_primitive_array_from_arrow_buffer_iter::<
                IntervalDayTimeType,
            >(buffer_iter, num_rows, data_type),
            IntervalUnit::MonthDayNano => new_primitive_array_from_arrow_buffer_iter::<
                IntervalMonthDayNanoType,
            >(buffer_iter, num_rows, data_type),
            IntervalUnit::YearMonth => new_primitive_array_from_arrow_buffer_iter::<
                IntervalYearMonthType,
            >(buffer_iter, num_rows, data_type),
        }),
        DataType::Null => Ok(new_null_array(data_type, num_rows as usize)),
        DataType::Time32(unit) => {
            match unit {
                TimeUnit::Millisecond => Ok(new_primitive_array_from_arrow_buffer_iter::<
                    Time32MillisecondType,
                >(buffer_iter, num_rows, data_type)),
                TimeUnit::Second => Ok(new_primitive_array_from_arrow_buffer_iter::<
                    Time32SecondType,
                >(buffer_iter, num_rows, data_type)),
                _ => Err(Error::IO(
                    format!("invalid time unit {:?} for 32-bit time type", unit),
                    location!(),
                )),
            }
        }
        DataType::Time64(unit) => match unit {
            TimeUnit::Microsecond => Ok(new_primitive_array_from_arrow_buffer_iter::<
                Time64MicrosecondType,
            >(buffer_iter, num_rows, data_type)),
            TimeUnit::Nanosecond => Ok(new_primitive_array_from_arrow_buffer_iter::<
                Time64NanosecondType,
            >(buffer_iter, num_rows, data_type)),
            _ => Err(Error::IO(
                format!("invalid time unit {:?} for 64-bit time type", unit),
                location!(),
            )),
        },
        DataType::Timestamp(unit, _) => Ok(match unit {
            TimeUnit::Microsecond => new_primitive_array_from_arrow_buffer_iter::<
                TimestampMicrosecondType,
            >(buffer_iter, num_rows, data_type),
            TimeUnit::Millisecond => new_primitive_array_from_arrow_buffer_iter::<
                TimestampMillisecondType,
            >(buffer_iter, num_rows, data_type),
            TimeUnit::Nanosecond => new_primitive_array_from_arrow_buffer_iter::<
                TimestampNanosecondType,
            >(buffer_iter, num_rows, data_type),
            TimeUnit::Second => new_primitive_array_from_arrow_buffer_iter::<TimestampSecondType>(
                buffer_iter,
                num_rows,
                data_type,
            ),
        }),
        DataType::UInt16 => Ok(new_primitive_array_from_arrow_buffer_iter::<UInt16Type>(
            buffer_iter,
            num_rows,
            data_type,
        )),
        DataType::UInt32 => Ok(new_primitive_array_from_arrow_buffer_iter::<UInt32Type>(
            buffer_iter,
            num_rows,
            data_type,
        )),
        DataType::UInt64 => Ok(new_primitive_array_from_arrow_buffer_iter::<UInt64Type>(
            buffer_iter,
            num_rows,
            data_type,
        )),
        DataType::UInt8 => Ok(new_primitive_array_from_arrow_buffer_iter::<UInt8Type>(
            buffer_iter,
            num_rows,
            data_type,
        )),
        // DataType::FixedSizeBinary(dimension) => {
        //     let mut buffers_iter = buffers.into_iter();
        //     let fsb_validity = buffers_iter.next().unwrap();
        //     let fsb_nulls = bytes_to_validity(fsb_validity, num_rows);

        //     let fsb_values = buffers_iter.next().unwrap();
        //     let fsb_values = Buffer::from_bytes(fsb_values.freeze().into());
        //     Ok(Arc::new(FixedSizeBinaryArray::new(
        //         *dimension, fsb_values, fsb_nulls,
        //     )))
        // }
        // DataType::FixedSizeList(items, dimension) => {
        //     let mut buffers_iter = buffers.into_iter();
        //     let fsl_validity = buffers_iter.next().unwrap();
        //     let fsl_nulls = bytes_to_validity(fsl_validity, num_rows);

        //     let remaining_buffers = buffers_iter.collect::<Vec<_>>();
        //     let items_array = primitive_array_from_buffers(
        //         items.data_type(),
        //         remaining_buffers,
        //         num_rows * (*dimension as u64),
        //     )?;
        //     Ok(Arc::new(FixedSizeListArray::new(
        //         items.clone(),
        //         *dimension,
        //         items_array,
        //         fsl_nulls,
        //     )))
        // }

        // FIXME: vortex currently output Utf8View as canonical type
        DataType::Utf8 | DataType::LargeUtf8 => {
            Ok(new_generic_byte_view_array_from_arrow_buffer_iter::<
                StringViewType,
            >(buffer_iter, num_rows))
        }
        DataType::Binary | DataType::LargeBinary => {
            Ok(new_generic_byte_view_array_from_arrow_buffer_iter::<
                BinaryViewType,
            >(buffer_iter, num_rows))
        }
        // DataType::Utf8 => Ok(new_generic_byte_array_from_arrow_buffer_iter::<
        //     GenericStringType<i32>,
        // >(buffer_iter, num_rows)),
        // DataType::LargeUtf8 => Ok(new_generic_byte_array_from_arrow_buffer_iter::<
        //     GenericStringType<i32>,
        // >(buffer_iter, num_rows)),
        // DataType::Binary => Ok(new_generic_byte_array_from_arrow_buffer_iter::<
        //     GenericBinaryType<i32>,
        // >(buffer_iter, num_rows)),
        // DataType::LargeBinary => Ok(new_generic_byte_array_from_arrow_buffer_iter::<
        //     GenericBinaryType<i64>,
        // >(buffer_iter, num_rows)),
        DataType::List(_child) => Ok(new_list_offsets_validity_from_buffer_iter::<Int32Type>(
            buffer_iter,
            num_rows,
            None,
        )),
        DataType::LargeList(_child) => Ok(new_list_offsets_validity_from_buffer_iter::<Int64Type>(
            buffer_iter,
            num_rows,
            None,
        )),
        _ => Err(Error::IO(
            format!(
                "The data type {} cannot be decoded from a primitive encoding",
                data_type
            ),
            location!(),
        )),
    }
}

pub fn primitive_array_from_buffers(
    data_type: &DataType,
    buffers: Vec<BytesMut>,
    num_rows: u64,
) -> Result<ArrayRef> {
    match data_type {
        DataType::Boolean => {
            let mut buffer_iter = buffers.into_iter();
            let null_buffer = buffer_iter.next().unwrap();
            let null_buffer = bytes_to_validity(null_buffer, num_rows);

            let data_buffer = buffer_iter.next().unwrap();
            let data_buffer = Buffer::from_vec(Vec::<u8>::from(data_buffer));
            let data_buffer = BooleanBuffer::new(data_buffer, 0, num_rows as usize);

            Ok(Arc::new(BooleanArray::new(data_buffer, null_buffer)))
        }
        DataType::Date32 => Ok(new_primitive_array::<Date32Type>(
            buffers, num_rows, data_type,
        )),
        DataType::Date64 => Ok(new_primitive_array::<Date64Type>(
            buffers, num_rows, data_type,
        )),
        DataType::Decimal128(_, _) => Ok(new_primitive_array::<Decimal128Type>(
            buffers, num_rows, data_type,
        )),
        DataType::Decimal256(_, _) => Ok(new_primitive_array::<Decimal256Type>(
            buffers, num_rows, data_type,
        )),
        DataType::Duration(units) => Ok(match units {
            TimeUnit::Second => {
                new_primitive_array::<DurationSecondType>(buffers, num_rows, data_type)
            }
            TimeUnit::Microsecond => {
                new_primitive_array::<DurationMicrosecondType>(buffers, num_rows, data_type)
            }
            TimeUnit::Millisecond => {
                new_primitive_array::<DurationMillisecondType>(buffers, num_rows, data_type)
            }
            TimeUnit::Nanosecond => {
                new_primitive_array::<DurationNanosecondType>(buffers, num_rows, data_type)
            }
        }),
        DataType::Float16 => Ok(new_primitive_array::<Float16Type>(
            buffers, num_rows, data_type,
        )),
        DataType::Float32 => Ok(new_primitive_array::<Float32Type>(
            buffers, num_rows, data_type,
        )),
        DataType::Float64 => Ok(new_primitive_array::<Float64Type>(
            buffers, num_rows, data_type,
        )),
        DataType::Int16 => Ok(new_primitive_array::<Int16Type>(
            buffers, num_rows, data_type,
        )),
        DataType::Int32 => Ok(new_primitive_array::<Int32Type>(
            buffers, num_rows, data_type,
        )),
        DataType::Int64 => Ok(new_primitive_array::<Int64Type>(
            buffers, num_rows, data_type,
        )),
        DataType::Int8 => Ok(new_primitive_array::<Int8Type>(
            buffers, num_rows, data_type,
        )),
        DataType::Interval(unit) => Ok(match unit {
            IntervalUnit::DayTime => {
                new_primitive_array::<IntervalDayTimeType>(buffers, num_rows, data_type)
            }
            IntervalUnit::MonthDayNano => {
                new_primitive_array::<IntervalMonthDayNanoType>(buffers, num_rows, data_type)
            }
            IntervalUnit::YearMonth => {
                new_primitive_array::<IntervalYearMonthType>(buffers, num_rows, data_type)
            }
        }),
        DataType::Null => Ok(new_null_array(data_type, num_rows as usize)),
        DataType::Time32(unit) => match unit {
            TimeUnit::Millisecond => Ok(new_primitive_array::<Time32MillisecondType>(
                buffers, num_rows, data_type,
            )),
            TimeUnit::Second => Ok(new_primitive_array::<Time32SecondType>(
                buffers, num_rows, data_type,
            )),
            _ => Err(Error::IO(
                format!("invalid time unit {:?} for 32-bit time type", unit),
                location!(),
            )),
        },
        DataType::Time64(unit) => match unit {
            TimeUnit::Microsecond => Ok(new_primitive_array::<Time64MicrosecondType>(
                buffers, num_rows, data_type,
            )),
            TimeUnit::Nanosecond => Ok(new_primitive_array::<Time64NanosecondType>(
                buffers, num_rows, data_type,
            )),
            _ => Err(Error::IO(
                format!("invalid time unit {:?} for 64-bit time type", unit),
                location!(),
            )),
        },
        DataType::Timestamp(unit, _) => Ok(match unit {
            TimeUnit::Microsecond => {
                new_primitive_array::<TimestampMicrosecondType>(buffers, num_rows, data_type)
            }
            TimeUnit::Millisecond => {
                new_primitive_array::<TimestampMillisecondType>(buffers, num_rows, data_type)
            }
            TimeUnit::Nanosecond => {
                new_primitive_array::<TimestampNanosecondType>(buffers, num_rows, data_type)
            }
            TimeUnit::Second => {
                new_primitive_array::<TimestampSecondType>(buffers, num_rows, data_type)
            }
        }),
        DataType::UInt16 => Ok(new_primitive_array::<UInt16Type>(
            buffers, num_rows, data_type,
        )),
        DataType::UInt32 => Ok(new_primitive_array::<UInt32Type>(
            buffers, num_rows, data_type,
        )),
        DataType::UInt64 => Ok(new_primitive_array::<UInt64Type>(
            buffers, num_rows, data_type,
        )),
        DataType::UInt8 => Ok(new_primitive_array::<UInt8Type>(
            buffers, num_rows, data_type,
        )),
        DataType::FixedSizeBinary(dimension) => {
            let mut buffers_iter = buffers.into_iter();
            let fsb_validity = buffers_iter.next().unwrap();
            let fsb_nulls = bytes_to_validity(fsb_validity, num_rows);

            let fsb_values = buffers_iter.next().unwrap();
            let fsb_values = Buffer::from_bytes(fsb_values.freeze().into());
            Ok(Arc::new(FixedSizeBinaryArray::new(
                *dimension, fsb_values, fsb_nulls,
            )))
        }
        DataType::FixedSizeList(items, dimension) => {
            let mut buffers_iter = buffers.into_iter();
            let fsl_validity = buffers_iter.next().unwrap();
            let fsl_nulls = bytes_to_validity(fsl_validity, num_rows);

            let remaining_buffers = buffers_iter.collect::<Vec<_>>();
            let items_array = primitive_array_from_buffers(
                items.data_type(),
                remaining_buffers,
                num_rows * (*dimension as u64),
            )?;
            Ok(Arc::new(FixedSizeListArray::new(
                items.clone(),
                *dimension,
                items_array,
                fsl_nulls,
            )))
        }
        DataType::Utf8 => Ok(new_generic_byte_array::<GenericStringType<i32>>(
            buffers, num_rows,
        )),
        DataType::LargeUtf8 => Ok(new_generic_byte_array::<GenericStringType<i64>>(
            buffers, num_rows,
        )),
        DataType::Binary => Ok(new_generic_byte_array::<GenericBinaryType<i32>>(
            buffers, num_rows,
        )),
        DataType::LargeBinary => Ok(new_generic_byte_array::<GenericBinaryType<i64>>(
            buffers, num_rows,
        )),
        DataType::List(child) => Ok(new_list_offsets_validity::<Int32Type>(
            buffers,
            num_rows,
            Arc::clone(child),
        )),
        DataType::LargeList(child) => Ok(new_list_offsets_validity::<Int64Type>(
            buffers,
            num_rows,
            Arc::clone(child),
        )),
        _ => Err(Error::IO(
            format!(
                "The data type {} cannot be decoded from a primitive encoding",
                data_type
            ),
            location!(),
        )),
    }
}
