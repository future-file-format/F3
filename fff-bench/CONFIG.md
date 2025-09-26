# FFF-Bench Configuration

## Data Path Configuration

The `fff-bench` crate now supports configurable data paths instead of hardcoded paths. This allows you to run benchmarks with data stored in different locations.

### Setting the Base Data Path

You can configure the base data path in two ways:

#### 1. Environment Variable (Recommended)

Set the `FFF_BENCH_DATA_PATH` environment variable to your desired base path:

```bash
export FFF_BENCH_DATA_PATH="/your/custom/path"
```

#### 2. Default Path

If no environment variable is set, the system will use the default path: `/mnt/nvme0n1/xinyu/`

### Usage Examples

```bash
# Using custom path
export FFF_BENCH_DATA_PATH="/home/user/benchmark-data"
cargo run --example bench_mem_vortex

# Using default path (no environment variable needed)
cargo run --example bench_mem_vortex
```

### Directory Structure

Your base data path should contain the following structure:

```
<base_path>/
├── data/
│   └── parquet/
│       ├── core.parquet
│       ├── bi.parquet
│       ├── classic.parquet
│       ├── geo.parquet
│       ├── log.parquet
│       └── ml.parquet
├── tpch/
│   └── parquet/
│       └── lineitem_duckdb_double.parquet
├── clickbench/
│   └── parquet/
│       └── hits_8M.parquet
└── laion/
    ├── parquet/
    │   ├── merged_8M.parquet
    │   └── merged_8M.json
    ├── vortex/
    │   └── merged_8M.vortex
    └── fff/
        └── merged_8M_rg1048576.fff
```
