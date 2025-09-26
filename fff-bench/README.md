# Independent Benchmark

This repo aims to be a reimplementation of the [evaluation paper](https://www.vldb.org/pvldb/vol17/p148-zeng.pdf). It serves for evaluation purposes of the different file formats.

To avoid the tedious data generation process, we host the generated data on public S3 buckets, and download the data directly.

Currently consider:
- Parquet
- ORC
- Lance
- Vortex
- <del>BtrBlocks</del> (removed because of different in-memory string representation)

```bash
# Need a correct tmp dir to sit on the same device as final data dir
cargo run --example bench --release -- compressed-size 2>&1 | tee compress_bench.log
bash ./exp_scripts/decompression.sh
```

## Acknowledgement

Part of the initial benchmarking code is derived from [bench-vortex](https://github.com/spiraldb/vortex/tree/develop/bench-vortex), in Apache License.

## WASM microbenchmark (Deprecated, no usage)

```bash
cargo run -p fff-bench --release --bin bench_wasm_decode -- wasmtime
cargo run -p fff-bench --release --bin bench_wasm_decode -- wasmer
```