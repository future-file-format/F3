use std::{error::Error, fs::File, hint::black_box};

use arrow_array::RecordBatch;
use bench_vortex::taxi_data::taxi_data_parquet;
use fff_bench::write_vortex;
use futures_util::TryStreamExt;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use tempfile::tempdir;
use vortex_array::{compute::slice, IntoCanonical};
use vortex_file::{LayoutContext, LayoutDeserializer, VortexReadBuilder};
use vortex_io::TokioFile;
use vortex_sampling_compressor::ALL_ENCODINGS_CONTEXT;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let original_file = taxi_data_parquet();
    let parquet = File::open(&original_file).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(parquet).unwrap();
    let reader = builder.with_batch_size(65536).build().unwrap();

    let batches: Vec<RecordBatch> = reader.map(|batch_result| batch_result.unwrap()).collect();
    let dir = tempdir().unwrap();
    let path = dir.path().join("foo");
    let vortex = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path.clone())
        .await
        .unwrap();
    let _vortex = write_vortex(&batches, vortex).await.unwrap();

    let builder: VortexReadBuilder<_> = VortexReadBuilder::new(
        TokioFile::open(path).unwrap(),
        LayoutDeserializer::new(
            ALL_ENCODINGS_CONTEXT.clone(),
            LayoutContext::default().into(),
        ),
    );

    let stream = builder
        .with_projection(vortex_file::Projection::Flat(vec![1.into()]))
        .build()
        .await
        .unwrap();
    let vecs: Vec<vortex_array::ArrayData> = stream.try_collect().await.unwrap();
    println!("{}", vecs[0].nbytes());
    println!("{}", vecs[0].len());
    let start = std::time::Instant::now();
    let _res = slice(&vecs[0], 6 * 1024, 8 * 1024)
        .unwrap()
        .into_canonical()
        .unwrap()
        .into_arrow()
        .unwrap();
    println!("time: {:?}", start.elapsed());
    let start = std::time::Instant::now();
    let _res = slice(&vecs[0], 32 * 1024, 48 * 1024)
        .unwrap()
        .into_canonical()
        .unwrap()
        .into_arrow()
        .unwrap();
    println!("time: {:?}", start.elapsed());
    const TOTAL_SIZE: usize = 128 * 1024;
    let random_vec: Vec<u8> = (0..TOTAL_SIZE).map(|_| rand::random::<u8>()).collect();
    let start = std::time::Instant::now();
    let mut new_vec = unsafe {
        let mut res = Vec::<u8>::with_capacity(TOTAL_SIZE);
        res.set_len(TOTAL_SIZE);
        res
    };
    new_vec.copy_from_slice(&random_vec);
    black_box(new_vec);
    println!("time: {:?}", start.elapsed());
    Ok(())
}
