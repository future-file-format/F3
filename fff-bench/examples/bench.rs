use core::panic;
use std::time::Instant;

use anyhow::Result;
use bench_vortex::setup_logger;
use clap::{Parser, Subcommand};
use fff_bench::bench_data::{
    BenchmarkDataset, BenchmarkDatasets, ClickBenchDataset, CsvToPqOptions, TPCHDataset,
};
use fff_bench::bench_data::{BenchmarkDatasets::CFB, CFBDataset};
use fff_poc::options::FileWriterOptions;
use log::{error, LevelFilter};
use strum::IntoEnumIterator;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    CompressedSize,
    ScanTime { format: String },
    RandomAccess { dataset: String },
    RandomAccessS3,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    setup_logger(LevelFilter::Error);
    match args.command {
        Some(Commands::CompressedSize) => compress_cfb().await?,
        Some(Commands::ScanTime { format }) => scan_cfb(&format).await?,
        Some(Commands::RandomAccess { dataset }) => random_access(&dataset).await?,
        Some(Commands::RandomAccessS3) => random_access_s3().await?,
        None => anyhow::bail!("No command provided"),
    }
    Ok(())
}

async fn compress_cfb() -> Result<()> {
    for dataset in CFBDataset::iter().filter(|x| *x != CFBDataset::Core64M) {
        error!("Start data set {}", dataset);
        let dataset = CFB(dataset);
        write_all_formats(dataset).await;
    }
    for dataset in TPCHDataset::iter() {
        error!("Start data set {}", dataset);
        let dataset = BenchmarkDatasets::TPCH(dataset);
        write_all_formats(dataset).await;
    }
    for dataset in ClickBenchDataset::iter() {
        error!("Start data set {}", dataset);
        let dataset = BenchmarkDatasets::CLICKBENCH(dataset);
        write_all_formats(dataset).await;
    }
    // for dataset in LaionDataset::iter() {
    //     error!("Start data set {}", dataset);
    //     let dataset = BenchmarkDatasets::LAION(dataset);
    //     write_all_formats(dataset).await;
    // }
    error!("Finish all data setes");
    Ok(())
}

async fn scan_cfb(format: &str) -> Result<()> {
    for dataset in CFBDataset::iter().filter(|x| *x != CFBDataset::Core64M) {
        error!("Start data set {}", dataset);
        let dataset = CFB(dataset);
        read_the_format(dataset, format).await?;
    }
    for dataset in TPCHDataset::iter() {
        error!("Start data set {}", dataset);
        let dataset = BenchmarkDatasets::TPCH(dataset);
        read_the_format(dataset, format).await?;
    }
    for dataset in ClickBenchDataset::iter() {
        error!("Start data set {}", dataset);
        let dataset = BenchmarkDatasets::CLICKBENCH(dataset);
        read_the_format(dataset, format).await?;
    }
    Ok(())
}

async fn random_access(dataset: &str) -> Result<()> {
    let dataset = match dataset {
        "tpch" => BenchmarkDatasets::TPCH(TPCHDataset::Lineitem),
        "clickbench" => BenchmarkDatasets::CLICKBENCH(ClickBenchDataset::Hits),
        "core" => BenchmarkDatasets::CFB(CFBDataset::Core),
        "bi" => BenchmarkDatasets::CFB(CFBDataset::Bi),
        "classic" => BenchmarkDatasets::CFB(CFBDataset::Classic),
        "geo" => BenchmarkDatasets::CFB(CFBDataset::Geo),
        "log" => BenchmarkDatasets::CFB(CFBDataset::Log),
        "ml" => BenchmarkDatasets::CFB(CFBDataset::Ml),
        _ => anyhow::bail!("Invalid dataset"),
    };
    use rand::prelude::IteratorRandom;
    // pick 5 random row ids from 8M
    let mut row_ids = (0..dataset.num_rows()).choose_multiple(&mut rand::thread_rng(), 1);
    row_ids.sort();
    println!("row_ids: {:?}", row_ids);
    dataset.ra_lance(row_ids.clone()).await?;
    dataset.ra_parquet(row_ids.clone()).await?;
    dataset.ra_vortex(row_ids.clone()).await?;
    dataset.ra_fff(&row_ids.iter().map(|id| *id as u64).collect::<Vec<_>>())?;
    dataset.ra_orc(row_ids[0])?;
    dataset.ra_nimble(row_ids[0])?;
    Ok(())
}

async fn random_access_s3() -> Result<()> {
    // random row id
    let row_id = rand::Rng::gen_range(&mut rand::thread_rng(), 0..1 * 1024 * 1024);
    let path = std::path::PathBuf::from("s3://f3-experiment/lineitem_duckdb_double_ra_64kEnc.fff");
    let start = Instant::now();
    fff_bench::read_fff(
        path,
        fff_bench::ReadFFFOpt {
            projections: Some(fff_poc::reader::Projection::All),
            selection: Some(fff_poc::reader::Selection::RowIndexes(vec![row_id])),
        },
    )
    .unwrap();
    error!(
        "Random access fff file took {:?}ms",
        start.elapsed().as_millis()
    );

    let start = Instant::now();
    fff_bench::read_lance(
        "s3://f3-experiment/lineitem_duckdb_double.lance",
        None,
        Some(vec![row_id as usize]),
        true,
    )
    .await
    .unwrap();
    error!(
        "Random access lance file took {:?}ms",
        start.elapsed().as_millis()
    );
    Ok(())
}

async fn write_all_formats(dataset: BenchmarkDatasets) {
    dataset.write_as_fff(FileWriterOptions::default());
    dataset.write_as_fff_wasm();
    dataset.write_as_vortex().await;
    dataset.write_as_parquet(CsvToPqOptions::default());
    dataset.write_as_lance().await;
    // dataset.write_as_lance_v2_1().await;
    if std::panic::catch_unwind(|| {
        dataset.write_as_orc();
    })
    .is_err()
    {
        error!("ORC write failed");
    }
    if std::panic::catch_unwind(|| {
        dataset.write_as_btrblocks();
    })
    .is_err()
    {
        error!("BtrBlocks write failed");
    }
}

async fn read_the_format(dataset: BenchmarkDatasets, format: &str) -> Result<()> {
    match format {
        "fff" => dataset.read_fff().unwrap(),
        "fffwasm" => dataset.read_fff_wasm().unwrap(),
        "parquet" => dataset.read_parquet()?,
        "vortex" => dataset.read_vortex().await?,
        "orc" => dataset.read_orc()?,
        "lance" => dataset.read_lance().await?,
        "btrblocks" => {
            if dataset.read_btrblocks().is_err() {
                error!("BtrBlocks read failed!")
            }
        }
        _ => panic!(),
    }
    Ok(())
}
