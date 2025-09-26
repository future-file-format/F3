# Reproduction steps for the results in the paper

## Figure 10 Metadata Overhead

```bash
cargo run --example metadata_test_v2 --release gen
./exp_scripts/projection.sh
```

This should have the data of Parquet, FFF, Vortex, and Lance in `proj_8rows.log`.

For the experiments of ORC and Nimble, we used the C++ version in [https://github.com/XinyuZeng/nimble/tree/xinyu/fff-bench](https://github.com/XinyuZeng/nimble/tree/xinyu/fff-bench).

## Figure 11 Compression Ratio and Decompression Speed

Set the `FFF_BENCH_DATA_PATH` environment variable to your data directory (a NVMe SSD).
Also set the `TMPDIR` environment variable to be the folder on the same device as the data directory.
The above two needs to be set before all the following experiments.

```bash
export FFF_BENCH_DATA_PATH="/your/custom/path"
export TMPDIR="/your/custom/path/tmp"
```

```bash
cargo run -p fff-bench --example bench --release -- compressed-size 2>&1 | tee compress_bench.log
./exp_scripts/decompression.sh
```
This should have the data of Parquet, FFF, Vortex, and Lance in `compress_bench.log` and `decomp.log`.

For the experiments of ORC and Nimble, we used the C++ version in [https://github.com/XinyuZeng/nimble/tree/xinyu/fff-bench](https://github.com/XinyuZeng/nimble/tree/xinyu/fff-bench).

## Figure 12 Random Access

This experiment utilizes the C++ version in [https://github.com/XinyuZeng/nimble/tree/xinyu/fff-bench](https://github.com/XinyuZeng/nimble/tree/xinyu/fff-bench), specifically `selection_nimble` and `selection_orc`. Make sure you have built the C++ version before running the following command.

```bash
./exp_scripts/random_access.sh
```

## Figure 13 and Table 2

This experiment relies on some code modications and tracing while writing/reading the formats. A one-off script is not provided and to be added in the future.

## Figure 14 and Table 3

```bash
cargo run -p fff-bench --example bench_dictscope --release -- --output-file ./results/dictscope_final_run.csv
```

## Figure 15

You need to run the Figure 16 script first before running this one.

```bash
./exp_scripts/wasm_micro_exp.sh
```

## Figure 16

```bash
./exp_scripts/build_wasm.sh | tee results/wasm_size.csv
```

## Figure 17

The time of using Wasm to decode the FFF file is included in `decomp.log` when running Figure 11 reproduction.

## Figure 18

Code in `fff-bench/examples/wasm_benefit_pco.rs` is used to generate the data for this figure.

## Figure 19

```bash
./exp_scripts/checksum.sh
```