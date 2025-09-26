/// Utility to convert jsonl files in RealNest datasets into FFF and get metadata from FFF.
/// We ignore files with unsupported data types, like map, binary, and json.
/// In total we have 26 feasible files from 43 files.
/// RealNest data got from https://homepages.cwi.nl/~boncz/RealNest/
use fff_poc::{
    io::reader::ObjectStoreReadAt,
    options::FileWriterOptions,
    reader::{collect_stats, FileReader, FileReaderV2Builder},
};
use lazy_static::lazy_static;
use object_store::aws::AmazonS3Builder;
use parquet::arrow::arrow_reader::{RowSelection, RowSelector};
use std::{fs::create_dir_all, io::BufReader, path::PathBuf, sync::Arc};

use arrow::datatypes::{DataType, Field, Schema};
use fff_bench::{parquet_decompress_from_async, write_fff, write_parquet};
use fff_core::errors::{Error, Result};
use serde_json::Value;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Gen,
    GenSingle,
    GenSingleFFF,
    Test,
    // Random Access
    Ra,
}

fn parse_data_type(data_type: &str, children: Option<&Vec<Value>>) -> Result<DataType> {
    let res = match data_type {
        "varchar" => DataType::Utf8,
        "float" => DataType::Float32,
        "double" => DataType::Float64,
        "integer" => DataType::Int32,
        "bigint" => DataType::Int64,
        "boolean" => DataType::Boolean,
        "timestamp" => DataType::Timestamp(arrow::datatypes::TimeUnit::Second, None),
        "struct" => DataType::Struct(
            children
                .unwrap()
                .iter()
                .map(|child| parse_field(child))
                .collect::<Result<Vec<_>>>()?
                .into(),
        ),
        "list" => {
            let child_field = parse_field(&children.unwrap()[0])?;
            DataType::List(child_field.into())
        }
        "map" => {
            return Err(Error::General(
                "Map type is not supported in FFF".to_string(),
            ));
            // let key_field = parse_field(&children.unwrap()[0])?;
            // let value_field = parse_field(&children.unwrap()[1])?;
            // DataType::Map(
            //     Field::new(
            //         "entries",
            //         DataType::Struct(vec![key_field, value_field].into()),
            //         true,
            //     )
            //     .into(),
            //     false,
            // )
        }
        "json" | "binary" => DataType::Binary,
        _ => panic!("Unsupported data type by arrow-json: {}", data_type),
    };
    Ok(res)
}

fn parse_field(column: &Value) -> Result<Field> {
    let name = column["name"].as_str().unwrap().to_string();
    let data_type = column["type"].as_str().unwrap();
    let children = column.get("children").map(|c| c.as_array().unwrap());
    let data_type = parse_data_type(data_type, children)?;
    Ok(Field::new(
        &name,
        data_type.clone(),
        match data_type {
            // DataType::Struct(_) | DataType::List(_) | DataType::LargeList(_) => false,
            _ => true,
        },
    ))
}

fn parse_json_schema(schema_json: &Value) -> Result<Schema> {
    let columns = schema_json["columns"].as_array().unwrap();
    let fields: Result<Vec<Field>> = columns.iter().map(|col| parse_field(col)).collect();
    fields.map(|fields| Schema::new(fields))
}

lazy_static! {
    static ref OUT_ROOT: PathBuf = PathBuf::from("/public/xinyu/RealNest/fff");
    static ref OUT_ROOT_PARQUET: PathBuf = PathBuf::from("/public/xinyu/RealNest/parquet");
    // static ref OUT_ROOT: PathBuf = PathBuf::from("/home/xinyu/fff-devel/fff-poc/data/nested");
    static ref INPUT_ROOT: String = "/public/xinyu/RealNest/tables_655360/".to_string();
    // static ref INPUT_ROOT: String = "/home/xinyu/RealNest/sample-data/100mib/".to_string();
    static ref RUNTIME: tokio::runtime::Runtime = tokio::runtime::Runtime::new().unwrap();
}

fn gen_fff(path: PathBuf) -> Result<()> {
    if path.is_dir() {
        let schema_path = path.join("schema.json");
        let file_path = path.join("data.jsonl");
        let file = std::fs::File::open(file_path).unwrap();
        let schema = parse_json_schema(
            &serde_json::from_reader(std::fs::File::open(schema_path).unwrap()).unwrap(),
        )?
        .into();
        let mut reader = arrow_json::ReaderBuilder::new(schema)
            .with_batch_size(65536)
            .build(BufReader::new(file))?;
        let mut batches = vec![];
        while let Some(batch) = reader.next() {
            batches.push(batch?);
        }
        create_dir_all(OUT_ROOT.as_path())?;
        let fff = std::fs::OpenOptions::new().write(true).create(true).open(
            OUT_ROOT
                .join(path.file_name().unwrap())
                .with_extension("fff"),
        )?;
        write_fff(&batches, &fff, FileWriterOptions::default()).unwrap();
    }
    Ok(())
}
fn gen_parquet(path: PathBuf) -> Result<()> {
    if path.is_dir() {
        let schema_path = path.join("schema.json");
        let file_path = path.join("data.jsonl");
        let file = std::fs::File::open(file_path).unwrap();
        let schema = parse_json_schema(
            &serde_json::from_reader(std::fs::File::open(schema_path).unwrap()).unwrap(),
        )?
        .into();
        let mut reader = arrow_json::ReaderBuilder::new(schema)
            .with_batch_size(65536)
            .build(BufReader::new(file))?;
        let mut batches = vec![];
        while let Some(batch) = reader.next() {
            batches.push(batch?);
        }
        create_dir_all(OUT_ROOT.as_path())?;
        let parquet = std::fs::OpenOptions::new().write(true).create(true).open(
            OUT_ROOT_PARQUET
                .join(path.file_name().unwrap())
                .with_extension("parquet"),
        )?;
        write_parquet(&batches, &parquet).unwrap()
    }
    Ok(())
}

fn random_access_parquet(path: PathBuf) -> Result<()> {
    // let file = std::fs::OpenOptions::new().read(true).open(path)?;
    let bucket = Arc::new(
        AmazonS3Builder::from_env()
            .with_url("s3://future-file-format/")
            .build()
            .unwrap(),
    );

    // let file = ObjectStoreReadAt::new(
    //     bucket.clone(),
    //     object_store::path::Path::from(path.clone().file_name().unwrap().to_str().unwrap()).into(),
    // );
    // parquet_decompress_from(
    //     file,
    //     Some(&[1]),
    //     Some(RowSelection::from(vec![
    //         RowSelector::skip(1),
    //         RowSelector::select(1),
    //     ])),
    // );

    let _head_result = futures_executor::block_on(async move {
        RUNTIME
            .spawn(async move {
                parquet_decompress_from_async(
                    bucket,
                    path.file_name().unwrap().to_str().unwrap(),
                    Some(&[1]),
                    Some(RowSelection::from(vec![
                        RowSelector::skip(1),
                        RowSelector::select(1),
                    ])),
                )
                .await
            })
            .await
            .unwrap()
    });
    Ok(())
}

fn random_access_fff(path: PathBuf) -> Result<()> {
    // let file = std::fs::OpenOptions::new().read(true).open(path.clone())?;
    // let file2 = std::fs::OpenOptions::new().read(true).open(path)?;
    let bucket = Arc::new(
        AmazonS3Builder::from_env()
            .with_url("s3://future-file-format/")
            .build()
            .unwrap(),
    );
    let file = ObjectStoreReadAt::new(
        bucket.clone(),
        object_store::path::Path::from(path.clone().file_name().unwrap().to_str().unwrap()).into(),
    );
    let mut reader = FileReaderV2Builder::new(file)
        .with_read_ahead(true)
        .build()
        .unwrap();
    let output_batches = reader
        .point_access_list_struct(1, reader.schema().fields[0].clone(), 1)
        .unwrap();
    assert!(output_batches.len() > 0);
    // println!("{:?}", output_batches[0]);
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    let mut error = 0;
    let mut total = 0;
    match args.command {
        Some(Commands::Gen) => {
            // iterate over all the directories in data_root
            // for each directory, read the schema.json file and the data.jsonl file
            // parse the schema.json file and create a schema
            for entry in std::fs::read_dir(INPUT_ROOT.as_str()).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                total += 1;
                if let Err(e) = gen_fff(path.clone()) {
                    println!("Error: {:?}", e);
                    error += 1;
                }
                let _ = gen_parquet(path.clone());
            }
        }
        Some(Commands::GenSingle) => {
            let path =
                PathBuf::from("/public/xinyu/RealNest/tables_655360/gharchive-PushEvent-flat");
            total += 1;
            if let Err(e) = gen_fff(path.clone()) {
                println!("Error: {:?}", e);
                error += 1;
            }
            let _ = gen_parquet(path.clone());
        }
        Some(Commands::Test) => {
            for file in std::fs::read_dir(OUT_ROOT.as_path()).unwrap() {
                let file = file.unwrap();
                let path = file.path();
                let fff = std::fs::OpenOptions::new().read(true).open(path.clone())?;
                let test = || {
                    let mut reader = FileReader::new(fff);
                    let postscript = reader.read_postscript()?;
                    let footer = reader.read_footer(&postscript)?;
                    collect_stats(footer)
                };
                println!("File: {:?}", path.file_name());
                total += 1;
                if let Err(e) = test() {
                    println!("Error: {:?}", e);
                    error += 1;
                };
            }
        }
        Some(Commands::Ra) => {
            let file = OUT_ROOT_PARQUET.join("gharchive-PushEvent-flat.parquet");
            total += 1;
            let start = std::time::Instant::now();
            if let Err(e) = random_access_parquet(file) {
                println!("Error: {:?}", e);
                error += 1;
            }
            println!("Parquet Time: {:?}", start.elapsed());

            let file = if cfg!(feature = "list-offsets-pushdown") {
                OUT_ROOT.join("gharchive-PushEvent-flat-pd.fff")
            } else {
                OUT_ROOT.join("gharchive-PushEvent-flat-nopd.fff")
            };
            let guard = pprof::ProfilerGuardBuilder::default()
                .frequency(1000)
                .blocklist(&["libc", "libgcc", "pthread", "vdso"])
                .build()
                .unwrap();
            let start = std::time::Instant::now();
            if let Err(e) = random_access_fff(file) {
                println!("Error: {:?}", e);
            }
            println!("FFF Time: {:?}", start.elapsed());
            if let Ok(report) = guard.report().build() {
                let file = std::fs::File::create("fff_nested_ra_flamegraph.svg").unwrap();
                report.flamegraph(file).unwrap();
            };
        }
        Some(Commands::GenSingleFFF) => {
            let path = PathBuf::from("/public/xinyu/RealNest/tables_655360/gharchive-PushEvent");
            let file = std::fs::File::open(path.join("data.jsonl")).unwrap();
            let schema = parse_json_schema(
                &serde_json::from_reader(std::fs::File::open(path.join("schema.json")).unwrap())
                    .unwrap(),
            )?
            .into();
            let mut reader = arrow_json::ReaderBuilder::new(schema)
                .with_batch_size(65536)
                .build(BufReader::new(file))?;
            let mut batches = vec![];
            while let Some(batch) = reader.next() {
                batches.push(batch?);
            }
            let fff = std::fs::OpenOptions::new().write(true).create(true).open(
                PathBuf::from("/home/xinyu/fff-devel")
                    .join(path.file_name().unwrap())
                    .with_extension("fff"),
            )?;
            write_fff(&batches, &fff, FileWriterOptions::default()).unwrap();
        }
        _ => panic!("Unsupported command:{:?}", args.command),
    }
    println!("Total: {}, Error: {}", total, error);
    Ok(())
}
