use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

/// Try different parameters for Lance and see the difference in compression ratio, decompression speed, and random access speed
use anyhow::Result;
use bench_vortex::setup_logger;
use clap::{Parser, Subcommand};
use fff_bench::config;
use fff_bench::{
    bench_data::{parquet_into_batches, PqToBatchesOptions},
    read_lance, write_lance,
};
use lance_encoding::encoder::{CoreFieldEncodingStrategy, StructuralEncodingStrategy};
use lance_file::{v2::writer::FileWriterOptions, version::LanceFileVersion};
use log::{error, LevelFilter};
use rand::Rng;
use std::io::Write;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Gen,
    Test,
}

#[tokio::main]
async fn main() -> Result<()> {
    setup_logger(LevelFilter::Error);
    let args = Args::parse();
    match args.command {
        Some(Commands::Gen) => {
            gen_lance_files().await?;
        }
        Some(Commands::Test) => {
            test_lance_files().await?;
        }
        None => {
            println!("No command provided");
        }
    }
    Ok(())
}

fn get_data_path(relative_path: &str) -> String {
    config::get_base_data_path()
        .join(relative_path)
        .to_string_lossy()
        .to_string()
}

async fn gen_lance_files() -> Result<()> {
    let datasets = vec![
        get_data_path("tpch/parquet/lineitem_duckdb_double.parquet"),
        // get_data_path("clickbench/parquet/hits_8M.parquet"),
        // get_data_path("data/parquet/core.parquet"),
        // get_data_path("data/parquet/bi.parquet"),
        // get_data_path("data/parquet/classic.parquet"),
        // get_data_path("data/parquet/geo.parquet"),
        // get_data_path("data/parquet/log.parquet"),
        // get_data_path("data/parquet/ml.parquet"),
    ];
    for dataset in datasets {
        let dataset = dataset.as_str();
        let create_lance_file = |dataset: &str, new_name: &str| {
            let lance_path: String = dataset.replace("parquet", "lance");

            // Split the filename to insert the new name before the extension
            let base_path = lance_path.rsplitn(2, '.').collect::<Vec<&str>>();
            let extension = base_path[0]; // lance
            let base_filename = base_path[1]; // path/to/hits_8M

            // Create the new path with the format path/to/hits_8M_version.lance
            let modified_path = format!("{}{}.{}", base_filename, new_name, extension);

            let lance_file = PathBuf::from(&modified_path);
            //create the directory if it doesn't exist
            std::fs::create_dir_all(lance_file.parent().unwrap()).unwrap();
            lance_file
        };
        let parquet_file = Path::new(dataset);
        let batches = parquet_into_batches(
            parquet_file.to_path_buf(),
            PqToBatchesOptions::with_batch_size(64 * 1024),
        )?;

        {
            // Create an array of all LanceFileVersion variants
            let versions = [
                // LanceFileVersion::Legacy,
                LanceFileVersion::V2_0,
                LanceFileVersion::Stable,
                // LanceFileVersion::V2_1,
                // LanceFileVersion::Next,
            ];

            // Iterate through all versions and create a lance file for each
            for version in versions {
                let mut options = FileWriterOptions::default();
                options.format_version = Some(version);

                let lance_file = create_lance_file(dataset, &format!("_{}", version));

                println!("Creating Lance file with version: {}", version);
                write_lance(&batches, lance_file.to_str().unwrap(), true, options).await?;
            }
        }

        {
            for data_cache_bytes in [
                4 * 1024,
                64 * 1024,
                1 * 1024 * 1024,
                1024 * 1024 * 8,
                8 * 8 * 1024 * 1024,
                17 * 8 * 1024 * 1024,
                64 * 8 * 1024 * 1024,
                105 * 8 * 1024 * 1024,
            ] {
                let mut options = FileWriterOptions::default();
                options.data_cache_bytes = Some(data_cache_bytes);
                let lance_file =
                    create_lance_file(dataset, &format!("_cache_bytes_{}", data_cache_bytes));
                write_lance(&batches, lance_file.to_str().unwrap(), true, options).await?;
            }
        }

        for max_page_bytes in [
            4 * 1024,
            64 * 1024,
            1 * 1024 * 1024,
            32 * 1024 * 1024,
            128 * 1024 * 1024,
        ] {
            let mut options = FileWriterOptions::default();
            options.max_page_bytes = Some(max_page_bytes);
            let lance_file =
                create_lance_file(dataset, &format!("_max_page_bytes_{}", max_page_bytes));
            write_lance(&batches, lance_file.to_str().unwrap(), true, options).await?;
        }

        let mut options = FileWriterOptions::default();
        options.encoding_strategy = Some(Arc::new(CoreFieldEncodingStrategy::default()));
        let lance_file = create_lance_file(dataset, &format!("_encoding_strategy_{}", "core"));
        write_lance(&batches, lance_file.to_str().unwrap(), true, options).await?;

        let mut options = FileWriterOptions::default();
        options.encoding_strategy = Some(Arc::new(StructuralEncodingStrategy::default()));
        let lance_file =
            create_lance_file(dataset, &format!("_encoding_strategy_{}", "structural"));
        write_lance(&batches, lance_file.to_str().unwrap(), true, options).await?;
    }
    Ok(())
}

async fn test_lance_files() -> Result<()> {
    let datasets = vec![get_data_path("tpch/parquet/lineitem_duckdb_double.parquet")];

    for dataset in datasets {
        let dataset = dataset.as_str();
        let parquet_file = Path::new(dataset);
        let dataset_name = parquet_file.file_name().unwrap().to_str().unwrap();

        // Helper function to create lance file paths
        let create_lance_file = |dataset: &str, new_name: &str| -> PathBuf {
            let lance_path: String = dataset.replace("parquet", "lance");
            let base_path = lance_path.rsplitn(2, '.').collect::<Vec<&str>>();
            let extension = base_path[0]; // lance
            let base_filename = base_path[1]; // path/to/hits_8M
            let modified_path = format!("{}{}.{}", base_filename, new_name, extension);
            PathBuf::from(&modified_path)
        };

        // Create output directory for CSV files
        let output_dir = PathBuf::from("results/lance_benchmark_results");
        std::fs::create_dir_all(&output_dir)?;

        // Function to run benchmarks for a given file
        async fn run_benchmark(
            file_path: &str,
            row_count: usize,
            repeat_count: usize,
        ) -> Result<(usize, Duration, Duration)> {
            let mut rng = rand::thread_rng();

            // Measure full read time
            let start = Instant::now();
            let mut nbytes = 0;
            for _ in 0..3 {
                nbytes = read_lance(file_path, None, None, true).await?;
            }
            let read_time = start.elapsed() / repeat_count as u32;

            // Measure random access time (average of multiple runs)
            let mut total_ra_time = Duration::from_secs(0);
            for _ in 0..repeat_count {
                let row_id = rng.gen_range(0..row_count);
                let start = Instant::now();
                let _ = read_lance(file_path, None, Some(vec![row_id]), true).await?;
                total_ra_time += start.elapsed();
            }
            let avg_ra_time = total_ra_time / repeat_count as u32;

            Ok((nbytes, read_time, avg_ra_time))
        }

        // Get the total number of rows in the dataset
        // Assuming 8M rows based on the filename, but you could read metadata if needed
        let total_rows = 8_000_000;
        let repeat_count = 10; // Number of times to repeat random access test

        // Base comparison (original Parquet file)
        {
            let base_output_path = output_dir.join("base_comparison.csv");
            let mut base_file = std::fs::File::create(base_output_path)?;

            writeln!(
                base_file,
                "file_name,format,file_size_mb,read_time_ms,random_access_time_ms"
            )?;

            // Measure Parquet file size
            let parquet_size = std::fs::metadata(parquet_file)?.len() as f64 / (1024.0 * 1024.0);
            writeln!(
                base_file,
                "{},parquet,{:.2},N/A,N/A",
                dataset_name, parquet_size
            )?;

            // Measure default Lance file if it exists
            let default_lance_file = create_lance_file(dataset, "");
            if default_lance_file.exists() {
                let lance_size =
                    std::fs::metadata(&default_lance_file)?.len() as f64 / (1024.0 * 1024.0);
                let (_, read_time, ra_time) = run_benchmark(
                    default_lance_file.to_str().unwrap(),
                    total_rows,
                    repeat_count,
                )
                .await?;

                writeln!(
                    base_file,
                    "{},lance_default,{:.2},{},{}",
                    dataset_name,
                    lance_size,
                    read_time.as_millis(),
                    ra_time.as_millis()
                )?;
            } else {
                writeln!(base_file, "{},lance_default,N/A,N/A,N/A", dataset_name)?;
            }
        }

        // Helper function to benchmark a parameter value
        async fn benchmark_param<T: std::fmt::Display>(
            dataset: &str,
            param_name: &str,
            param_value: T,
            create_lance_file: impl Fn(&str, &str) -> PathBuf,
            total_rows: usize,
            repeat_count: usize,
            results: &mut Vec<(String, f64, u128, u128)>,
        ) -> Result<()> {
            let param_str = format!("_{}{}", param_name, param_value);
            let lance_file = create_lance_file(dataset, &param_str);

            if !lance_file.exists() {
                error!("File doesn't exist: {}", lance_file.display());
                results.push((param_value.to_string(), f64::NAN, 0, 0));
                return Ok(());
            }

            // Get file size
            let file_size = std::fs::metadata(&lance_file)?.len() as f64 / (1024.0 * 1024.0);

            // Run benchmarks
            let (_, read_time, ra_time) =
                run_benchmark(lance_file.to_str().unwrap(), total_rows, repeat_count).await?;

            results.push((
                param_value.to_string(),
                file_size,
                read_time.as_millis(),
                ra_time.as_millis(),
            ));

            println!("Tested {}: {}", param_name, param_value);
            Ok(())
        }

        // Test version variants
        {
            let version_variants = [LanceFileVersion::V2_0, LanceFileVersion::Stable];
            let mut results = Vec::new();
            for &value in &version_variants {
                benchmark_param(
                    dataset,
                    "",
                    value,
                    |d, s| create_lance_file(d, s),
                    total_rows,
                    repeat_count,
                    &mut results,
                )
                .await?;
            }

            // Write results to CSV
            let csv_path = output_dir.join("version_comparison.csv");
            let mut csv_file = std::fs::File::create(csv_path)?;
            writeln!(
                csv_file,
                "version,file_size_mb,read_time_ms,random_access_time_ms"
            )?;

            for (value, size, read_time, ra_time) in results {
                writeln!(csv_file, "{},{:.2},{},{}", value, size, read_time, ra_time)?;
            }
        }

        // Test data cache bytes variants
        {
            let data_cache_bytes_variants = [
                4 * 1024,
                64 * 1024,
                1 * 1024 * 1024,
                8 * 1024 * 1024,
                17 * 1024 * 1024,
                17 * 8 * 1024 * 1024,
                64 * 8 * 1024 * 1024,
                105 * 8 * 1024 * 1024,
            ];

            let mut results = Vec::new();
            for &value in &data_cache_bytes_variants {
                benchmark_param(
                    dataset,
                    "cache_bytes_",
                    value,
                    |d, s| create_lance_file(d, s),
                    total_rows,
                    repeat_count,
                    &mut results,
                )
                .await?;
            }

            // Write results to CSV
            let csv_path = output_dir.join("data_cache_bytes_comparison.csv");
            let mut csv_file = std::fs::File::create(csv_path)?;
            writeln!(
                csv_file,
                "data_cache_bytes,file_size_mb,read_time_ms,random_access_time_ms"
            )?;

            for (value, size, read_time, ra_time) in results {
                writeln!(csv_file, "{},{:.2},{},{}", value, size, read_time, ra_time)?;
            }
        }

        // Test max page bytes variants
        {
            let max_page_bytes_variants = [
                4 * 1024,
                64 * 1024,
                1 * 1024 * 1024,
                32 * 1024 * 1024,
                128 * 1024 * 1024,
            ];

            let mut results = Vec::new();
            for &value in &max_page_bytes_variants {
                benchmark_param(
                    dataset,
                    "max_page_bytes_",
                    value,
                    |d, s| create_lance_file(d, s),
                    total_rows,
                    repeat_count,
                    &mut results,
                )
                .await?;
            }

            // Write results to CSV
            let csv_path = output_dir.join("max_page_bytes_comparison.csv");
            let mut csv_file = std::fs::File::create(csv_path)?;
            writeln!(
                csv_file,
                "max_page_bytes,file_size_mb,read_time_ms,random_access_time_ms"
            )?;

            for (value, size, read_time, ra_time) in results {
                writeln!(csv_file, "{},{:.2},{},{}", value, size, read_time, ra_time)?;
            }
        }

        // Test encoding strategies
        {
            let encoding_strategies = ["core", "structural"];

            let mut results = Vec::new();
            for strategy in &encoding_strategies {
                benchmark_param(
                    dataset,
                    "encoding_strategy_",
                    strategy,
                    |d, s| create_lance_file(d, s),
                    total_rows,
                    repeat_count,
                    &mut results,
                )
                .await?;
            }

            // Write results to CSV
            let csv_path = output_dir.join("encoding_strategy_comparison.csv");
            let mut csv_file = std::fs::File::create(csv_path)?;
            writeln!(
                csv_file,
                "encoding_strategy,file_size_mb,read_time_ms,random_access_time_ms"
            )?;

            for (value, size, read_time, ra_time) in results {
                writeln!(csv_file, "{},{:.2},{},{}", value, size, read_time, ra_time)?;
            }
        }

        println!("Results saved to the lance_benchmark_results directory");
    }

    Ok(())
}
