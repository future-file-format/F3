> **Note**
>
> This README is deprecated and will be removed in the future.

This examples directory is more like an experiment directory. Just find a place to put executable files.

# Nested data

First get RealNest data: https://homepages.cwi.nl/~boncz/RealNest
We only need `gharchive-PushEvent`.
Then use `convert_gh_push.py` to flatten the struct of author.
Run `./nested.sh` for experiment running. `nested.ipynb` for ploting the results.

# Projection / Metadata Cost

1. Run `cargo run --example metadata_test --release -- gen` first to generate data.
2. In the directory where you generate the data, run `fff-bench/examples/experiments/projection.sh`. (require root access)
3. With the output `proj.log`, you can analyze data with `experiments/plot.ipynb`.

# Compression Ratio and Decompression Speed

See `bench.rs`
