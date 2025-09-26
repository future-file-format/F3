use anyhow::Result;
use bench_vortex::setup_logger;
use fff_bench::bench_data::{parquet_into_batches, PqToBatchesOptions};
use fff_bench::config;
use log::LevelFilter;
use vortex_array::compress::CompressionStrategy;
use vortex_file::VortexFileWriter;
use vortex_sampling_compressor::SamplingCompressor;

/// 1k batch to write vortex and record the memory usage and each column chunk's size.
/// Failed because of unsupported data type List in the vortex version.
#[tokio::main]
async fn main() -> Result<()> {
    setup_logger(LevelFilter::Error);
    let p = config::get_base_data_path().join("laion/parquet/merged_8M.parquet");
    let parquet_batches =
        parquet_into_batches(p.to_path_buf(), PqToBatchesOptions::with_batch_size(1024))?;
    let vortex =
        tokio::fs::File::create(config::get_base_data_path().join("laion/vortex/merged_8M.vortex"))
            .await?;
    let mut writer = VortexFileWriter::new(vortex);
    let compressor: &dyn CompressionStrategy = &SamplingCompressor::default();
    let mut mem_usage = vec![];
    let mut col_sizes = vec![0; parquet_batches[0].num_columns()];
    for batch in parquet_batches.iter() {
        let vortex_array = vortex_array::ArrayData::try_from(batch.clone()).unwrap();
        let compressed = compressor.compress(&vortex_array).unwrap();
        mem_usage.push(compressed.nbytes());
        let st = vortex_array::array::StructArray::try_from(compressed.clone())?;
        for (i, field) in st.children().enumerate() {
            col_sizes[i] += field.nbytes();
        }
        writer = writer.write_array_columns(compressed).await.unwrap();
    }
    // print the average memory usage and each column chunk's size
    println!(
        "Average memory usage: {}",
        mem_usage.iter().sum::<usize>() / mem_usage.len()
    );
    for (i, size) in col_sizes.iter().enumerate() {
        println!("Column {} size: {}", i, size / parquet_batches.len());
    }
    let _written = writer.finalize().await.unwrap();
    Ok(())
}
