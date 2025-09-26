use std::fs::File;

/// Layout experiments to see the peak memory usage and IOUnit sizes in Parquet.
use anyhow::Result;
use bench_vortex::setup_logger;
use fff_bench::bench_data::BenchmarkDatasets::LAION;
use fff_bench::bench_data::{parquet_into_batches, BenchmarkDataset};
use fff_bench::{rewrite_parquet_via_mine, write_fff};
use fff_poc::options::FileWriterOptions;
use log::LevelFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // let args: Vec<String> = std::env::args().collect();
    // let dataset: u8 = args.get(1).map(|s| s.parse().unwrap()).unwrap();
    // let dataset = dataset.try_into().unwrap();
    setup_logger(LevelFilter::Error);
    const M: usize = 1048576;
    for rg_size in [
        // 65536,
        131072,
        262144,
        524288,
        M,
        2 * M,
        4 * M,
        8 * M,
        // 16 * M,
        // 32 * M,
        // 64 * M,
    ]
    .iter()
    {
        log::error!("Start row group size: {}", *rg_size);
        // Laion
        let ps = LAION(fff_bench::bench_data::LaionDataset::Merge8M)
            .list_files(fff_bench::bench_data::FileType::Parquet);
        assert!(ps.len() == 1);
        let p = ps[0].clone();
        let out = std::path::PathBuf::from(format!(
            "{}_rg{}.parquet",
            p.to_str().unwrap().strip_suffix(".parquet").unwrap(),
            *rg_size
        ));
        let parquet_batches = parquet_into_batches(p.clone(), Default::default())?;
        let _parquet_mem_size = rewrite_parquet_via_mine(p, out.as_path(), *rg_size)?;

        let ps = LAION(fff_bench::bench_data::LaionDataset::Merge8M)
            .list_files(fff_bench::bench_data::FileType::FFF);
        assert!(ps.len() == 1);
        let p = ps[0].clone();
        let out = std::path::PathBuf::from(format!(
            "{}_rg{}.fff",
            p.to_str().unwrap().strip_suffix(".fff").unwrap(),
            *rg_size
        ));
        write_fff(
            &parquet_batches,
            &File::create(out)?,
            FileWriterOptions::builder()
                // Ideally we should not need to set this, but nested encoding do this for us.
                // Refactor needed for writer.
                .set_custom_encunit_len([(11, 128), (12, 128)].into())
                .set_row_group_size(*rg_size as u64)
                .build(),
        )?;

        // core
        // let ps = BenchmarkDatasets::CFB(fff_bench::bench_data::CFBDataset::Core64M)
        //     .list_files(fff_bench::bench_data::FileType::Parquet);
        // assert!(ps.len() == 1);
        // let p = ps[0].clone();
        // let out = std::path::PathBuf::from(format!(
        //     "{}_rg{}.parquet",
        //     p.to_str().unwrap().strip_suffix(".parquet").unwrap(),
        //     *rg_size
        // ));
        // let parquet_batches = parquet_into_batches(p.clone())?;
        // let _parquet_mem_size = rewrite_parquet_via_mine(p, out.as_path(), *rg_size)?;

        // let ps = BenchmarkDatasets::CFB(fff_bench::bench_data::CFBDataset::Core64M)
        //     .list_files(fff_bench::bench_data::FileType::FFF);
        // assert!(ps.len() == 1);
        // let p = ps[0].clone();
        // let out = std::path::PathBuf::from(format!(
        //     "{}_rg{}.fff",
        //     p.to_str().unwrap().strip_suffix(".fff").unwrap(),
        //     *rg_size
        // ));
        // write_fff(
        //     &parquet_batches,
        //     &File::create(out)?,
        //     FileWriterOptions::builder()
        //         .set_row_group_size(*rg_size as u64)
        //         .build(),
        // )?;
    }
    // PBI(dataset).remove_parquet();
    // PBI(dataset).write_as_parquet(CsvToPqOptions { rg_size: *rg_size });
    // PBI(PBIDataset::Bimbo).write_as_parquet(CsvToPqOptions { rg_size: 65536 });
    // PBI(PBIDataset::Bimbo).write_as_fff();
    Ok(())
}
