use std::sync::Arc;

/// Layout experiments to see the peak memory usage and IOUnit sizes in Parquet.
use anyhow::Result;
use bench_vortex::setup_logger;
use fff_bench::bench_data::BenchmarkDataset;
use fff_bench::bench_data::BenchmarkDatasets::{CFB, LAION};
use log::LevelFilter;

#[tokio::main]
async fn main() -> Result<()> {
    setup_logger(LevelFilter::Error);
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 2 {
        // calculate the average page size for the column specified by the index
        let col_idx = args[1].parse::<usize>()?;
        let ds = LAION(fff_bench::bench_data::LaionDataset::Merge8M);
        let ds = ds.list_files(fff_bench::bench_data::FileType::Lance);
        let path = ds.get(0).unwrap();
        let object_store = Arc::new({
            let mut res = lance_io::object_store::ObjectStore::local();
            // We do not want io parallelism for now
            res.set_io_parallelism(1);
            res
        });
        let path = object_store::path::Path::from(path.to_str().unwrap());
        let scheduler = lance_io::scheduler::ScanScheduler::new(
            object_store,
            lance_io::scheduler::SchedulerConfig::default_for_testing(),
        );
        let file_scheduler = scheduler.open_file(&path).await.unwrap();
        let metadata = lance_file::v2::reader::FileReader::read_all_metadata(&file_scheduler)
            .await
            .unwrap();
        println!(
            "Average page size: {:?}",
            metadata.column_infos[col_idx]
                .page_infos
                .iter()
                .map(|p| {
                    let s = p
                        .buffer_offsets_and_sizes
                        .iter()
                        .map(|(_, size)| size)
                        .sum::<u64>();
                    // println!("page size: {:?}", s);
                    s
                })
                .sum::<u64>()
                / metadata.column_infos[col_idx].page_infos.len() as u64
        );
    } else {
        let ds = CFB(fff_bench::bench_data::CFBDataset::Core);
        ds.write_as_lance().await;
        // let ds = LAION(fff_bench::bench_data::LaionDataset::Merge8M);
        // ds.write_as_lance().await;
    }
    Ok(())
}
