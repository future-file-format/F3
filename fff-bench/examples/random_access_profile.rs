use std::fs::File;

/// Profile the time breakdown for random access FFF
use anyhow::Result;
use bench_vortex::setup_logger;
use fff_bench::bench_data::{BenchmarkDataset, BenchmarkDatasets, TPCHDataset};
use log::LevelFilter;
#[tokio::main]
async fn main() -> Result<()> {
    setup_logger(LevelFilter::Error);
    let format = std::env::args().nth(1).unwrap();
    // let dataset = BenchmarkDatasets::CLICKBENCH(ClickBenchDataset::Hits);
    let dataset = BenchmarkDatasets::TPCH(TPCHDataset::Lineitem);
    // pick 5 random row ids from 8M
    // let mut row_ids = (0..8 * 1024 * 1024).choose_multiple(&mut rand::thread_rng(), 1);
    // let mut row_ids: Vec<usize> = vec![1531968];
    let mut row_ids: Vec<usize> = vec![1531968];
    row_ids.sort();
    println!("row_ids: {:?}", row_ids);
    let guard = pprof::ProfilerGuardBuilder::default()
        .frequency(1000)
        .blocklist(&["libc", "libgcc", "pthread", "vdso"])
        .build()
        .unwrap();
    const N: usize = 1;
    match format.as_str() {
        "fff" => {
            for _ in 0..N {
                dataset.ra_fff(&row_ids.iter().map(|id| *id as u64).collect::<Vec<_>>())?;
            }
        }
        "vortex" => {
            for _ in 0..N {
                dataset.ra_vortex(row_ids.clone()).await?;
            }
        }
        "lance" => {
            for _ in 0..N {
                dataset.ra_lance(row_ids.clone()).await?;
            }
        }
        _ => {
            panic!("Invalid format: {}", format);
        }
    }
    if let Ok(report) = guard.report().build() {
        let file = match format.as_str() {
            "fff" => File::create("random_access_fff.svg").unwrap(),
            "vortex" => File::create("random_access_vortex.svg").unwrap(),
            "lance" => File::create("random_access_lance.svg").unwrap(),
            _ => panic!("Invalid format: {}", format),
        };
        report.flamegraph(file).unwrap();
    };
    Ok(())
}
