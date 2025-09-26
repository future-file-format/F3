/// Utility to merge multiple Parquets into one.
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use std::fs::File;

const ROWS_TO_READ: usize = 64 * 1024 * 1024; // 64 million rows

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file_paths = (0..=106)
        .map(|i| format!("/public/xinyu/laion_100m/{:04}.parquet", i))
        .collect::<Vec<_>>();

    // Open the output file and initialize the ArrowWriter
    let output_file = File::create("merged.parquet")?;
    let props = WriterProperties::builder()
        .set_compression(parquet::basic::Compression::SNAPPY)
        .build();

    // Initialize the writer with the schema from the first file
    let first_file = File::open(&file_paths[0])?;
    let first_builder = ParquetRecordBatchReaderBuilder::try_new(first_file)?;
    let schema = first_builder.schema().clone();
    let mut writer = ArrowWriter::try_new(output_file, schema.clone(), Some(props))?;

    let mut break_flag = false;
    for file_path in file_paths {
        let file = File::open(&file_path)?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;

        let mut reader = builder.with_batch_size(ROWS_TO_READ).build()?;
        let mut total_rows_read = 0;

        while let Some(batch) = reader.next() {
            let batch = batch?;
            let rows_in_batch = batch.num_rows();

            if total_rows_read + rows_in_batch > ROWS_TO_READ {
                let rows_to_take = ROWS_TO_READ - total_rows_read;
                let sliced_batch = batch.slice(0, rows_to_take);
                writer.write(&sliced_batch)?;
                break_flag = true;
                break;
            } else {
                writer.write(&batch)?;
                total_rows_read += rows_in_batch;
            }
        }
        if break_flag {
            break;
        }
    }

    // Close the writer to finalize the Parquet file
    writer.close()?;

    println!("Merged data written to merged.parquet");
    Ok(())
}
