/// A test file to micro bench the allocation speed of `std::cell::Cell`.
/// 12ns per allocation.
use criterion::black_box;

fn main() {
    const ITERATIONS: u32 = 10000000;
    let start = std::time::Instant::now();
    for _ in 0..ITERATIONS {
        let cell = std::cell::Cell::new(0);
        black_box(cell);
    }
    let elapsed = start.elapsed();
    println!("Elapsed: {:?}", elapsed / ITERATIONS);
}
