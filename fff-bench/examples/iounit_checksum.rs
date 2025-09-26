use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use anyhow::Result;
use clap::{Parser, Subcommand};
use fff_bench::bench_data::{parquet_into_batches, PqToBatchesOptions};
use fff_bench::config;
use fff_poc::{options::FileWriterOptionsBuilder, reader::FileReaderV2Builder, writer::FileWriter};
use parquet::{file::reader::FileReader, file::reader::SerializedFileReader};

/// Compare the perforamnce of writing during the iounit checksum vs. without it.
/// First run `cargo run --example iounit_checksum --release -- write` to compare the write performance.
/// Then run `cargo run --example iounit_checksum --release -- read` to compare the read performance.

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Write,
    Read,
}

fn get_data_path(relative_path: &str) -> String {
    config::get_base_data_path()
        .join(relative_path)
        .to_string_lossy()
        .to_string()
}

fn main() -> Result<()> {
    let args = Args::parse();
    let datasets = vec![
        get_data_path("tpch/parquet/lineitem_duckdb_double.parquet"),
        get_data_path("clickbench/parquet/hits_8M.parquet"),
        // get_data_path("data/parquet/core.parquet"),
        // get_data_path("data/parquet/bi.parquet"),
        // get_data_path("data/parquet/classic.parquet"),
        // get_data_path("data/parquet/geo.parquet"),
        // get_data_path("data/parquet/log.parquet"),
        // get_data_path("data/parquet/ml.parquet"),
    ];

    // Print CSV header
    println!("dataset,enable_iounit_checksum_throughput,disable_iounit_checksum_throughput");

    for dataset in datasets {
        let dataset = dataset.as_str();
        let create_fff_file = |dataset: &str, new_name: &str| {
            let fff_path: String = dataset.replace("parquet", "fff");

            // Split the filename to insert the new name before the extension
            let base_path = fff_path.rsplitn(2, '.').collect::<Vec<&str>>();
            let extension = base_path[0]; // fff
            let base_filename = base_path[1]; // path/to/hits_8M

            // Create the new path with the format path/to/hits_8M_version.lance
            let modified_path = format!("{}{}.{}", base_filename, new_name, extension);

            let fff_file = PathBuf::from(&modified_path);
            //create the directory if it doesn't exist
            std::fs::create_dir_all(fff_file.parent().unwrap()).unwrap();
            fff_file
        };

        // Extract just the filename from the full path for cleaner CSV output
        let dataset_name = Path::new(dataset).file_name().unwrap().to_str().unwrap();
        match args.command {
            Some(Commands::Write) => {
                let parquet_file = Path::new(dataset);
                let batches = parquet_into_batches(
                    parquet_file.to_path_buf(),
                    PqToBatchesOptions::with_batch_size(64 * 1024),
                )?;

                // Calculate total number of rows
                let total_rows: usize = batches.iter().map(|batch| batch.num_rows()).sum();

                let schema = batches.first().unwrap().schema();

                // Run with iounit checksum enabled
                let options = FileWriterOptionsBuilder::with_defaults()
                    .enable_io_unit_checksum(true)
                    .build();
                let file = File::create(create_fff_file(dataset, "enable_iounit_checksum"))?;
                let start = Instant::now();
                let mut writer = FileWriter::try_new(schema.clone(), file, options).unwrap();
                for batch in batches.iter() {
                    writer.write_batch(&batch).unwrap();
                }
                writer.finish().unwrap();
                let enable_elapsed = start.elapsed();
                let enable_throughput = total_rows as f64 / enable_elapsed.as_secs_f64();

                // Run with iounit checksum disabled
                let options = FileWriterOptionsBuilder::with_defaults().build();
                let file = File::create(create_fff_file(dataset, "disable_iounit_checksum"))?;
                let start = Instant::now();
                let mut writer = FileWriter::try_new(schema, file, options).unwrap();
                for batch in batches.iter() {
                    writer.write_batch(&batch).unwrap();
                }
                writer.finish().unwrap();
                let disable_elapsed = start.elapsed();
                let disable_throughput = total_rows as f64 / disable_elapsed.as_secs_f64();

                // Output in CSV format (rows/second)
                println!(
                    "{},{:.2},{:.2}",
                    dataset_name, enable_throughput, disable_throughput
                );
            }
            Some(Commands::Read) => {
                let parquet_reader =
                    SerializedFileReader::new(File::open(dataset).expect("Unable to read file"))
                        .unwrap();
                let row_group_metadata = parquet_reader.metadata().row_groups();
                let total_rows = row_group_metadata
                    .iter()
                    .map(|rg| rg.num_rows())
                    .sum::<i64>();
                let file = Arc::new(File::open(create_fff_file(
                    dataset,
                    "enable_iounit_checksum",
                ))?);
                let mut reader = FileReaderV2Builder::new(file.clone())
                    .with_verify_io_unit_checksum(true)
                    .build()
                    .unwrap();
                let start = Instant::now();
                reader.read_file().unwrap();
                let elapsed = start.elapsed();
                let enable_throughput = total_rows as f64 / elapsed.as_secs_f64();

                let file = Arc::new(File::open(create_fff_file(
                    dataset,
                    "disable_iounit_checksum",
                ))?);
                let mut reader = FileReaderV2Builder::new(file)
                    .with_verify_io_unit_checksum(false)
                    .build()
                    .unwrap();
                let start = Instant::now();
                reader.read_file().unwrap();
                let elapsed = start.elapsed();
                let disable_throughput = total_rows as f64 / elapsed.as_secs_f64();
                println!(
                    "{},{:.2},{:.2}",
                    dataset_name, enable_throughput, disable_throughput
                );
            }
            None => {
                println!("No command provided");
            }
        }
    }
    Ok(())
}
