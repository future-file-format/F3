use std::fs::File;

/// Profile the time breakdown for random access Lance
use anyhow::Result;
use bench_vortex::setup_logger;
use fff_bench::read_lance;
use log::LevelFilter;
#[tokio::main]
async fn main() -> Result<()> {
    setup_logger(LevelFilter::Error);
    let path = std::env::args().nth(1).unwrap();

    let mut row_ids: Vec<usize> = vec![1531968];
    row_ids.sort();
    println!("row_ids: {:?}", row_ids);
    let guard = pprof::ProfilerGuardBuilder::default()
        .frequency(1000)
        .blocklist(&["libc", "libgcc", "pthread", "vdso"])
        .build()
        .unwrap();
    const N: usize = 1;
    for _ in 0..N {
        let _ = read_lance(&path, None, Some(row_ids.clone()), true).await?;
    }
    if let Ok(report) = guard.report().build() {
        let file = File::create("random_access_lance.svg").unwrap();
        report.flamegraph(file).unwrap();
    };
    Ok(())
}
