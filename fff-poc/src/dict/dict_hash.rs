use std::hash::Hash as _;

pub trait DictHash<U: Ord + std::hash::Hash + Clone> {
    fn dict_hash(x: &Self) -> U;
    fn hash_to_u64(x: &Self) -> u64;
}

impl DictHash<i32> for i32 {
    fn dict_hash(x: &Self) -> i32 {
        *x
    }
    fn hash_to_u64(x: &Self) -> u64 {
        *x as u64
    }
}
impl DictHash<i64> for i64 {
    fn dict_hash(x: &Self) -> i64 {
        *x
    }
    fn hash_to_u64(x: &Self) -> u64 {
        *x as u64
    }
}
impl DictHash<u32> for f32 {
    fn dict_hash(x: &Self) -> u32 {
        x.to_bits()
    }
    fn hash_to_u64(x: &Self) -> u64 {
        x.to_bits() as u64
    }
}
impl DictHash<u64> for f64 {
    fn dict_hash(x: &Self) -> u64 {
        x.to_bits()
    }
    fn hash_to_u64(x: &Self) -> u64 {
        x.to_bits()
    }
}
impl DictHash<String> for String {
    fn dict_hash(x: &Self) -> String {
        x.clone()
    }
    fn hash_to_u64(x: &Self) -> u64 {
        let mut hasher = std::hash::DefaultHasher::new();
        x.hash(&mut hasher);
        std::hash::Hasher::finish(&hasher)
    }
}
impl DictHash<bool> for bool {
    fn dict_hash(x: &Self) -> bool {
        *x
    }
    fn hash_to_u64(x: &Self) -> u64 {
        *x as u64
    }
}
