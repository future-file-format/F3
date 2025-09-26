/// For figuring out why e2e scan is much slower than micro bench
/// 12/29/2024
/// Bug fixed: cost is on the validation of utf8 string when constructing Arrow Arrays.
use std::fs::File;

use anyhow::Result;
use fff_bench::bench_data::BenchmarkDataset;
use fff_bench::bench_data::{BenchmarkDatasets::CFB, CFBDataset};
#[tokio::main]
async fn main() -> Result<()> {
    let guard = pprof::ProfilerGuardBuilder::default()
        .frequency(1000)
        .blocklist(&["libc", "libgcc", "pthread", "vdso"])
        .build()
        .unwrap();
    for _ in 0..20 {
        CFB(CFBDataset::Geo).read_fff_wasm().unwrap();
    }
    if let Ok(report) = guard.report().build() {
        let file = File::create("wasm_e2e_test_wasm.svg").unwrap();
        report.flamegraph(file).unwrap();
    };

    // let guard = pprof::ProfilerGuardBuilder::default()
    //     .frequency(1000)
    //     .blocklist(&["libc", "libgcc", "pthread", "vdso"])
    //     .build()
    //     .unwrap();
    // for _ in 0..20 {
    //     CFB(CFBDataset::Geo).read_fff().unwrap();
    // }
    // if let Ok(report) = guard.report().build() {
    //     let file = File::create("wasm_e2e_test.svg").unwrap();
    //     report.flamegraph(file).unwrap();
    // };

    Ok(())
}
