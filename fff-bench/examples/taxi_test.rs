#[allow(unused_imports)]
use fff_bench::{parquet_decompress_from, read_vortex, write_fff, write_vortex};
use fff_poc::options::FileWriterOptionsBuilder;
use std::io::Seek;
use std::{error::Error, fs::File};
use tempfile::tempdir;
use vortex_file::Projection;

use arrow_array::RecordBatch;
use bench_vortex::taxi_data::taxi_data_parquet;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // let original_file = Path::new(env!("CARGO_MANIFEST_DIR"))
    //     .join("data")
    //     .join("combined.parquet");
    let original_file = taxi_data_parquet();
    let parquet = File::open(&original_file).unwrap();
    let parquet_size = parquet.metadata().unwrap().len();
    let builder = ParquetRecordBatchReaderBuilder::try_new(parquet).unwrap();
    let reader = builder.with_batch_size(64 * 1024).build().unwrap();

    let mut fff = tempfile::tempfile().unwrap();
    let batches: Vec<RecordBatch> = reader.map(|batch_result| batch_result.unwrap()).collect();
    write_fff(
        &batches,
        &fff,
        FileWriterOptionsBuilder::with_defaults()
            .write_built_in_wasm(true)
            .build(),
    )
    .unwrap();
    let dir = tempdir().unwrap();
    let vortex_path = dir.path().join("foo");
    let vortex = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(vortex_path.clone())
        .await
        .unwrap();
    let vortex = write_vortex(&batches, vortex).await.unwrap();
    // output size of parquet and fff
    println!("parquet size: {}", parquet_size);
    println!("fff size: {}", fff.metadata().unwrap().len());
    println!("vortex size: {}", vortex.metadata().await.unwrap().len());

    // Parquet read
    let parquet = File::open(original_file).unwrap();
    let start = std::time::Instant::now();
    let _parquet_size = parquet_decompress_from(parquet, None, None);
    println!("Parquet decompression time: {:?}", start.elapsed());
    // fff read
    let iterations = 10;
    fff.rewind().unwrap();
    let guard = pprof::ProfilerGuardBuilder::default()
        .frequency(1000)
        // .blocklist(&["libc", "libgcc", "pthread", "vdso"])
        .build()
        .unwrap();
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let f1 = fff.try_clone().unwrap();
        let mut reader = fff_poc::reader::FileReaderV2Builder::new(std::sync::Arc::new(f1))
            .build()
            .unwrap();
        let _output_batches = reader.read_file().unwrap();
    }
    println!("FFF decompression time: {:?}", start.elapsed() / iterations);
    if let Ok(report) = guard.report().build() {
        let file = File::create("flamegraph.svg").unwrap();
        report.flamegraph(file).unwrap();
    };
    // vortex read
    let start = std::time::Instant::now();
    read_vortex(vortex_path, Projection::All).await.unwrap();
    println!("vortex decompression time: {:?}", start.elapsed());
    Ok(())
}
