/// From parquet-rs. Apache License.
/// Returns the ceil of value/divisor.
///
/// This function should be removed after
/// [`int_roundings`](https://github.com/rust-lang/rust/issues/88581) is stable.
#[inline]
pub fn ceil<T: num::Integer>(value: T, divisor: T) -> T {
    num::Integer::div_ceil(&value, &divisor)
}

#[inline]
pub fn padding_size(size: usize, alignment: usize) -> usize {
    size.next_multiple_of(alignment) - size
}
