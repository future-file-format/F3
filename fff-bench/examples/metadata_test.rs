/// CAUTION: this is the old version of the metadata test, which is not used anymore.
use std::{collections::HashSet, io::Seek, sync::Arc};

use anyhow::Result;
use arrow::{
    array::{ArrayRef, Float64Array, RecordBatch},
    datatypes::{DataType, Field, Schema},
};
use clap::{Parser, Subcommand};
use fff_bench::{
    parquet_decompress_from, read_lance, read_orc, read_vortex, write_fff, write_lance, write_orc,
    write_parquet, write_vortex,
};
use fff_poc::{options::FileWriterOptions, reader::FileReaderV2Builder};
use itertools::Itertools;
use rand::{rngs::StdRng, Rng, SeedableRng};
use vortex_file::Projection;

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

fn column_name(i: usize) -> String {
    format!("column_{}", i)
}

fn generate_batch(num_columns: usize, num_rows: usize) -> RecordBatch {
    let mut fields = Vec::with_capacity(num_columns);

    for i in 0..num_columns {
        fields.push(Field::new(&column_name(i), DataType::Float64, false));
    }

    let schema = Arc::new(Schema::new(fields));

    let mut columns: Vec<ArrayRef> = Vec::with_capacity(num_columns);
    let array = Arc::new(Float64Array::from(vec![42.0; num_rows]));

    for _ in 0..num_columns {
        columns.push(array.clone());
    }
    RecordBatch::try_new(schema.clone(), columns).unwrap()
}

#[tokio::main]
async fn main() -> Result<()> {
    // do some I/O to warm up.
    let warmup_dir = "data/copy";
    // read all files with names match the pattern "1000.*"
    let files = std::fs::read_dir(warmup_dir)?;
    for file in files {
        let file = file.unwrap();
        let path = file.path();
        let file_name = path.file_name().unwrap().to_str().unwrap();
        if file_name.starts_with("1000.") {
            let mut f = std::fs::OpenOptions::new().read(true).open(path)?;
            let mut buf = vec![0; 1024];
            std::io::Read::read(&mut f, &mut buf)?;
        }
    }
    let data_dir = "data_8rows";
    // create data_dir if not exists
    std::fs::create_dir_all(data_dir)?;
    let args = Args::parse();
    // for num_columns in [20_000] {
    for num_columns in [
        // 100, 200, 300, 400, 500, 600, 700, 800, 900,
        // 1000,
        2333, 10, 20, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000, 5_000, 10000, 20_000,
        50000, 100_000,
    ] {
        for num_rows in [8] {
            // for num_rows in [65536] {
            // for num_rows in [1000, 10000, 100000] {
            let mut fff = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(format!("{data_dir}/{}.fff", num_columns))?;
            let mut parquet = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(format!("{data_dir}/{}.parquet", num_columns))?;
            // let dir = tempdir().unwrap();
            // let path = dir.path().join("foo");
            let vortex_path = format!("{data_dir}/{}.vortex", num_columns);
            let vortex = tokio::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(vortex_path.clone())
                .await
                .unwrap();
            let lance_path = format!("{data_dir}/{}.lance", num_columns);
            let orc_path = format!("{data_dir}/{}.orc", num_columns);
            match args.command {
                Some(Commands::Gen) => {
                    // create a record batch with 10k rows and 10000 columns, random floats
                    let batch = generate_batch(num_columns, num_rows);
                    write_fff(&[batch.clone()], &fff, FileWriterOptions::default())?;
                    write_parquet(&[batch.clone()], &parquet)?;
                    write_vortex(&[batch.clone()], vortex).await?;
                    write_lance(&[batch.clone()], &lance_path, false, Default::default()).await?;
                    write_orc(&[batch.clone()], &orc_path)?;

                    fff.rewind().unwrap();
                    parquet.rewind().unwrap();
                }
                Some(Commands::Test) => {
                    // create 10 random unique numbers between 0 and num_columns
                    let projections: Vec<usize> = {
                        if num_columns == 1000 {
                            vec![156, 183, 374, 445, 596, 598, 731, 779, 796, 950]
                        } else {
                            let mut rng = StdRng::seed_from_u64(42);
                            let mut unique_numbers = HashSet::new();
                            while unique_numbers.len() < 10 {
                                let num = rng.gen_range(0..num_columns);
                                unique_numbers.insert(num);
                            }
                            unique_numbers.into_iter().sorted().collect::<Vec<_>>()
                        }
                    };
                    // dbg!(&projections);
                    // ---- Test FFF ----
                    let fff_size = fff.metadata().unwrap().len();
                    let start = std::time::Instant::now();
                    let mut reader = FileReaderV2Builder::new(Arc::new(fff))
                        .with_projections(fff_poc::reader::Projection::LeafColumnIndexes(
                            projections.clone(),
                        ))
                        .build()
                        .unwrap();
                    let _result = reader.read_file().unwrap();
                    println!(
                        "FFF num_rows: {}, num_cols:{}, file_size: {}, time: {:?}",
                        num_rows,
                        num_columns,
                        fff_size,
                        start.elapsed().as_nanos()
                    );
                    // ---- Test Parquet ----
                    let parquet_size = parquet.metadata().unwrap().len();
                    let start = std::time::Instant::now();
                    let _ = parquet_decompress_from(parquet, Some(&projections), None);
                    println!(
                        "Parquet num_rows: {}, num_cols:{}, file_size: {}, time: {:?}",
                        num_rows,
                        num_columns,
                        parquet_size,
                        start.elapsed().as_nanos()
                    );

                    // ---- Test Lance ----
                    // first get schema and field id mapping from the file
                    let lance_size = std::fs::OpenOptions::new()
                        .read(true)
                        .open(&lance_path)?
                        .metadata()
                        .unwrap()
                        .len();
                    let col_names: Vec<_> = projections.iter().map(|&i| column_name(i)).collect();
                    let start = std::time::Instant::now();
                    let _ = read_lance(
                        &lance_path,
                        Some(col_names.iter().map(|s| s.as_str()).collect()),
                        None,
                        false,
                    )
                    .await;
                    println!(
                        "Lance num_rows: {}, num_cols:{}, file_size: {}, time: {:?}",
                        num_rows,
                        num_columns,
                        lance_size,
                        start.elapsed().as_nanos()
                    );
                    // ---- Test ORC ----
                    let orc_size = std::fs::OpenOptions::new()
                        .read(true)
                        .open(&orc_path)?
                        .metadata()
                        .unwrap()
                        .len();
                    let proj_cl = projections.clone();
                    let start = std::time::Instant::now();
                    let _ = read_orc(&orc_path, Some(proj_cl)).unwrap();
                    println!(
                        "ORC num_rows: {}, num_cols:{}, file_size: {}, time: {:?}",
                        num_rows,
                        num_columns,
                        orc_size,
                        start.elapsed().as_nanos()
                    );
                    // ---- Test vortex ----
                    let vortex_size = vortex.metadata().await.unwrap().len();
                    let start = std::time::Instant::now();
                    read_vortex(vortex_path.into(), Projection::new(projections))
                        .await
                        .map(|_| {
                            println!(
                                "Vortex num_rows: {}, num_cols:{}, file_size: {}, time: {:?}",
                                num_rows,
                                num_columns,
                                vortex_size,
                                start.elapsed().as_nanos()
                            )
                        })
                        .unwrap_or_else(|e| {
                            println!("Error: {}", e);
                        });
                }
                _ => panic!("Invalid command"),
            }
        }
    }
    Ok(())
}
