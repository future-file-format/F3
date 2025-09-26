Note: None of the benchmarks in this directory is used in the paper, should be cleaned up.

# Some microbenchmarks for testing Wasm

`memory_cost.rs` contains code that verifies the function call with memory allocation overhead is significant. 
This means that many small calls are not ideal and we should not make the decoding kernel too small like 1024 values.

To test, run `cargo bench --bench memory_cost -- --profile-time=5` and `cargo bench --bench memory_cost`

`multi_thread.rs` is for testing the multi-thread performance of running wasm decoding. 
The conclusion is to reuse instance with a fail-then-retry strategy for now.