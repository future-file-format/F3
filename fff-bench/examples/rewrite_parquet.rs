/// Utility to merge multiple Parquets into one.
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use std::fs::File;

const ROWS_TO_READ: usize = 64 * 1024 * 1024; // 64 million rows

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file_path = "/public/xinyu/RealNest/parquet/gharchive-PushEvent-flat.parquet";

    // Open the output file and initialize the ArrowWriter
    let output_file =
        File::create("/public/xinyu/RealNest/parquet/gharchive-PushEvent-flat-new.parquet")?;
    let props = WriterProperties::builder()
        .set_compression(parquet::basic::Compression::SNAPPY)
        .build();

    // Initialize the writer with the schema from the first file
    let first_file = File::open(&file_path)?;
    let first_builder = ParquetRecordBatchReaderBuilder::try_new(first_file)?;
    let schema = first_builder.schema().clone();
    let mut writer = ArrowWriter::try_new(output_file, schema.clone(), Some(props))?;

    let file = File::open(&file_path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;

    let mut reader = builder.with_batch_size(ROWS_TO_READ).build()?;

    while let Some(batch) = reader.next() {
        let batch = batch?;
        writer.write(&batch)?;
    }
    // Close the writer to finalize the Parquet file
    writer.close()?;

    Ok(())
}
