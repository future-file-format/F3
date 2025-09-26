use std::path::Path;

/// Generate FFF files for local random access testing
use anyhow::Result;
use bench_vortex::setup_logger;
use fff_bench::bench_data::{parquet_into_batches, PqToBatchesOptions};
use fff_bench::config;
use fff_poc::{options::FileWriterOptions, writer::FileWriter};
use log::LevelFilter;

#[tokio::main]
async fn main() -> Result<()> {
    setup_logger(LevelFilter::Error);
    let datasets = vec![
        config::get_base_data_path()
            .join("tpch/parquet/lineitem_duckdb_double.parquet")
            .to_string_lossy()
            .to_string(),
        config::get_base_data_path()
            .join("clickbench/parquet/hits_8M.parquet")
            .to_string_lossy()
            .to_string(),
        config::get_base_data_path()
            .join("data/parquet/core.parquet")
            .to_string_lossy()
            .to_string(),
        config::get_base_data_path()
            .join("data/parquet/bi.parquet")
            .to_string_lossy()
            .to_string(),
        config::get_base_data_path()
            .join("data/parquet/classic.parquet")
            .to_string_lossy()
            .to_string(),
        config::get_base_data_path()
            .join("data/parquet/geo.parquet")
            .to_string_lossy()
            .to_string(),
        config::get_base_data_path()
            .join("data/parquet/log.parquet")
            .to_string_lossy()
            .to_string(),
        config::get_base_data_path()
            .join("data/parquet/ml.parquet")
            .to_string_lossy()
            .to_string(),
    ];
    for dataset in datasets {
        let parquet_file = Path::new(&dataset);
        let fff_path = dataset.replace("parquet", "fffra");
        let fff_file = Path::new(&fff_path);
        //create the directory if it doesn't exist
        std::fs::create_dir_all(fff_file.parent().unwrap()).unwrap();
        let fff = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(fff_file)
            .unwrap();
        let batches = parquet_into_batches(
            parquet_file.to_path_buf(),
            PqToBatchesOptions::with_batch_size(64 * 1024),
        )?;
        let mut fff_writer = FileWriter::try_new(
            batches[0].schema(),
            fff,
            FileWriterOptions::builder()
                .set_iounit_size(4 * 1024)
                .build(),
        )
        .unwrap();
        for batch in batches {
            fff_writer.write_batch(&batch).unwrap();
        }
        fff_writer.finish().unwrap();
    }
    Ok(())
}
