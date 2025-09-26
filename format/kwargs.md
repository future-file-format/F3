# Spec for the serialized kwargs passed to Wasm decoder for an EncUnit

## Top level serialization

num_keys (i32)
key_lens (i32 * num_keys)
word_lens (i32 * num_keys)
key-word * num_keys (var len)

## Key-Word s

### spd

- spd stands for Selection-Pushdown

- Word is a serialized form of roaring bitmap: https://github.com/RoaringBitmap/RoaringFormatSpec. set-bit (1) means the row is selected and should be decoded out, while unset-bit (0) means the row is not selected.

### ppd

- ppd stands for Predicate-Pushdown

- Word is our custom serialized format. Currently supporting conjunctive of comparison operators on a single column.

### partial_decode

- output partially decoded data, in the form of Arrow Array: Dict/REE/StringView.

- Word is a single byte with 1 indicating enabled and 0 indicating disabled.

## Notes

ppd and partial_decode are two different approaches and should only use one. We allow them here to testing different approaches' trade-offs.