/// Layout experiments to see the peak memory usage and IOUnit sizes in fff.
use anyhow::Result;
use bench_vortex::setup_logger;
use fff_bench::bench_data::BenchmarkDatasets::{LAION, PBI};
use fff_bench::bench_data::{BenchmarkDataset, PBIDataset};
use fff_poc::options::FileWriterOptionsBuilder;
use log::LevelFilter;

#[tokio::main]
async fn main() -> Result<()> {
    setup_logger(LevelFilter::Error);
    // PBI(PBIDataset::Arade).remove_fff();
    // PBI(PBIDataset::Arade).write_as_fff();
    // PBI(PBIDataset::Bimbo).remove_fff();
    // PBI(PBIDataset::Bimbo).write_as_fff();
    // if argv[1] exits, then get avg io unit size for the column specified by argv[2]
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 3 {
        match args[1].as_str() {
            "laion" => {
                let col_idx = args[2].parse::<usize>()?;
                let avg_io_unit_size = get_avg_io_unit_size(
                    &LAION(fff_bench::bench_data::LaionDataset::Merge8M),
                    col_idx,
                )?;
                println!("Avg io unit size: {}", avg_io_unit_size);
            }
            _ => {
                println!("Invalid dataset");
            }
        }
    } else {
        LAION(fff_bench::bench_data::LaionDataset::Data0001).remove_fff();
        LAION(fff_bench::bench_data::LaionDataset::Data0001).write_as_fff(
            FileWriterOptionsBuilder::with_defaults()
                .set_custom_encunit_len([(11, 128), (12, 128)].into())
                .build(),
        );
    }
    Ok(())
}

fn get_avg_io_unit_size(dataset: &impl BenchmarkDataset, col_idx: usize) -> Result<usize> {
    let ds = dataset.list_files(fff_bench::bench_data::FileType::FFF);
    let fff = ds.get(0).unwrap();
    fff_poc::reader::get_avg_io_unit_size(std::sync::Arc::new(std::fs::File::open(fff)?), col_idx)
        .map_err(|e| anyhow::anyhow!("Error getting avg io unit size: {}", e))
}
