use std::{
    io::{Seek, SeekFrom, Write},
    path::Path,
    sync::Arc,
};

use arrow_array::RecordBatch;
use fff_core::errors::Error;
use fff_poc::{
    options::FileWriterOptions,
    reader::{FileReaderV2Builder, Selection},
    writer::FileWriter,
};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use rand::{Rng, SeedableRng};

fn read_parquet_file(file_path: impl AsRef<Path>, batch_size: usize) -> Vec<RecordBatch> {
    let parquet = std::fs::File::open(file_path).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(parquet).unwrap();
    let reader = builder.with_batch_size(batch_size).build().unwrap();

    reader.map(|batch_result| batch_result.unwrap()).collect()
}

fn prepare_test_file(options: FileWriterOptions) -> Arc<std::fs::File> {
    let batches = read_parquet_file(bench_vortex::taxi_data::taxi_data_parquet(), 65536);
    let schema = batches[0].schema();
    let temp_file = Arc::new(tempfile::tempfile().unwrap());
    let mut writer = FileWriter::try_new(schema, temp_file.clone(), options).unwrap();
    for batch in batches {
        writer.write_batch(&batch).unwrap();
    }
    writer.finish().unwrap();
    temp_file
}

#[test]
fn corrupted_iounit() {
    let options = FileWriterOptions::builder()
        .enable_io_unit_checksum(true)
        .build();
    let temp_file = prepare_test_file(options);

    let mut file = temp_file.clone();
    file.seek(SeekFrom::Start(100)).unwrap();
    file.write_all(&[0; 100]).unwrap();

    let mut reader = FileReaderV2Builder::new(temp_file)
        .with_verify_io_unit_checksum(true)
        .with_selection(Selection::RowIndexes(vec![5]))
        .build()
        .unwrap();
    assert!(matches!(
        reader.read_file(),
        Err(Error::General(e)) if e.eq("Checksum verification failed")
    ));
}

#[test]
fn corrupted_flatbuffer() {
    let options = FileWriterOptions::builder().build();
    let temp_file = prepare_test_file(options);

    let mut file = temp_file.clone();
    file.seek(SeekFrom::End(-32 - 64)).unwrap();
    file.write_all(&[42; 50]).unwrap();

    let reader = FileReaderV2Builder::new(temp_file)
        .with_selection(Selection::RowIndexes(vec![5]))
        .build();
    println!("{}", reader.as_ref().err().unwrap());
    assert!(matches!(
        reader,
        Err(Error::ParseError(e)) if e.contains("Unable to get root as footer:")
    ));
}

#[test]
fn verify_file_checksum() {
    let options = FileWriterOptions::builder().build();
    let temp_file = prepare_test_file(options);

    let mut file = temp_file.clone();
    file.seek(SeekFrom::End(-32 - 42 * 42 * 13)).unwrap();
    file.write_all(&[42; 42 * 42]).unwrap();

    let reader = FileReaderV2Builder::new(temp_file)
        .with_verify_file_checksum(true)
        .with_selection(Selection::RowIndexes(vec![5]))
        .build();
    assert!(matches!(
        reader,
        Err(Error::General(e)) if e.contains("File level Checksum verification failed")
    ));
}

#[test]
fn fuzz_test() {
    let options = FileWriterOptions::builder()
        .enable_io_unit_checksum(true)
        .build();
    let temp_file = prepare_test_file(options);

    // Clone the file to get its length
    let mut file_for_length = temp_file.clone();
    let file_length = file_for_length.seek(SeekFrom::End(0)).unwrap();

    // Number of fuzzing iterations
    const FUZZ_ITERATIONS: usize = 20;

    // Create a deterministic RNG for reproducibility
    let mut rng = rand::rngs::StdRng::seed_from_u64(42);

    for _ in 0..FUZZ_ITERATIONS {
        // Copy the original uncorrupted file for each iteration
        let corrupted_file = {
            let mut dst = tempfile::tempfile().unwrap();
            std::io::copy(&mut temp_file.clone(), &mut dst).unwrap();
            dst.flush().unwrap();
            Arc::new(dst)
        };

        // Decide how many bytes to corrupt (between 1 and 50)
        let corrupt_bytes_count = rng.gen_range(1..=50);

        // Corrupt random bytes at random positions
        let mut file = corrupted_file.clone();
        for _ in 0..corrupt_bytes_count {
            // Choose a random position (avoid the very beginning and end for better results)
            let position = rng.gen_range(10..file_length - 10);

            // Seek to the position
            file.seek(SeekFrom::Start(position)).unwrap();

            // Write random bytes (1-8 bytes)
            let num_bytes = rng.gen_range(1..=8);
            let random_bytes: Vec<u8> = (0..num_bytes).map(|_| rng.gen()).collect();
            file.write_all(&random_bytes).unwrap();
        }

        // Try to build a reader and read the file
        let reader_result = FileReaderV2Builder::new(corrupted_file.clone())
            .with_verify_io_unit_checksum(true)
            .with_selection(Selection::RowIndexes(vec![5]))
            .build();

        match reader_result {
            Ok(mut reader) => {
                // If reader was created successfully, reading should fail
                let read_result = reader.read_file();
                assert!(
                    read_result.is_err(),
                    "Expected read_file to fail after corruption, but it succeeded"
                );
            }
            Err(_) => {
                // Reader creation failed as expected
            }
        }
    }
}
