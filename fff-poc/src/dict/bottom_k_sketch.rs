use std::collections::BinaryHeap;

use lazy_static::lazy_static;
use rand::Rng;

/// Store the bottom-K elements per hash function
const K: usize = 2048;
/// Use M hash functions
const M: usize = 3;
// Total complexity is O(M(n log K + c^2 K)), where n is #elements, c is #columns

lazy_static! {
    static ref COEFFS: Vec<(u64, u64)> = {
        let mut rng = rand::thread_rng();
        (0..M).map(|_| (rng.gen(), rng.gen())).collect()
    };
}

pub struct BottomKSketch {
    /// A heap to store the bottom K hash values
    bottom_k: Vec<BinaryHeap<u64>>,
    bottom_k_arr: Vec<Vec<u64>>,
}

impl BottomKSketch {
    pub fn new() -> Self {
        Self {
            bottom_k: vec![BinaryHeap::new(); M],
            bottom_k_arr: vec![],
        }
    }

    pub fn add_hash(&mut self, val: u64) {
        for (i, (a, b)) in COEFFS.iter().enumerate() {
            let hash_val = a.wrapping_mul(val).wrapping_add(*b);
            if self.bottom_k[i].len() < K {
                self.bottom_k[i].push(hash_val);
            } else if hash_val < *self.bottom_k[i].peek().unwrap() {
                self.bottom_k[i].pop();
                self.bottom_k[i].push(hash_val);
            }
        }
    }

    pub fn finish(&mut self) {
        self.bottom_k_arr = self
            .bottom_k
            .iter_mut()
            .map(|heap| {
                let mut vec: Vec<_> = heap.drain().collect();
                vec.sort_unstable();
                vec
            })
            .collect::<Vec<_>>();
    }

    pub fn estimate_jaccard(&self, other: &Self) -> f64 {
        let mut jaccard = 0f64;
        for i in 0..M {
            let mut collected = 0;
            let mut common = 0;
            let mut self_bottom_pos = 0;
            let mut other_bottom_pos = 0;
            let self_bottom_len = self.bottom_k_arr[i].len();
            let other_bottom_len = other.bottom_k_arr[i].len();
            for _ in 0..K {
                if self_bottom_pos == self_bottom_len {
                    collected = K.min(collected + other_bottom_len - other_bottom_pos);
                    break;
                }
                if other_bottom_pos == other_bottom_len {
                    collected = K.min(collected + self_bottom_len - self_bottom_pos);
                    break;
                }
                let self_val = self.bottom_k_arr[i][self_bottom_pos];
                let other_val = other.bottom_k_arr[i][other_bottom_pos];
                collected += 1;
                if self_val == other_val {
                    common += 1;
                    self_bottom_pos += 1;
                    other_bottom_pos += 1;
                } else if self_val < other_val {
                    self_bottom_pos += 1;
                } else {
                    other_bottom_pos += 1;
                }
            }
            jaccard += (common as f64) / (collected as f64);
        }
        jaccard / M as f64
    }
}
