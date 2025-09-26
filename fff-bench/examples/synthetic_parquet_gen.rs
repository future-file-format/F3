use arrow::array::{ArrayRef, Int32Array};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use rand::Rng;
use rand_distr::{Distribution, Zipf};
use std::sync::Arc;

fn create_record_batch(array: &Int32Array, num_columns: usize) -> RecordBatch {
    // Create a vector of ArrayRefs by repeating the input array
    let columns: Vec<ArrayRef> = (0..num_columns)
        .map(|_| Arc::new(array.clone()) as ArrayRef)
        .collect();

    // Define the schema for the record batch
    let schema = Schema::new(
        (0..num_columns)
            .map(|i| Field::new(format!("col_{}", i), DataType::Int32, false))
            .collect::<Vec<_>>(),
    );

    // Create the record batch
    RecordBatch::try_new(Arc::new(schema), columns).unwrap()
}

use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use std::fs::File;

fn write_record_batch_to_parquet(record_batch: &RecordBatch, file_path: &str, num_writes: usize) {
    // Create a file for writing
    let file = File::create(file_path).unwrap();

    // Define the writer properties (optional)
    let props = WriterProperties::builder()
        .set_compression(parquet::basic::Compression::SNAPPY)
        .build();

    // Create an Arrow writer
    let mut writer = ArrowWriter::try_new(file, record_batch.schema(), Some(props)).unwrap();

    // Write the record batch multiple times
    for _ in 0..num_writes {
        writer.write(record_batch).unwrap();
    }

    // Close the writer to finalize the file
    writer.close().unwrap();
}

fn main() {
    let total_values = 65536; // 64k values
    let window_size = 128;

    // Create a random number generator
    let mut rng = rand::thread_rng();

    // Define the number of unique windows
    let num_unique_windows = total_values / window_size; // 65536 / 128 = 512 windows

    // Generate unique windows with uniformly distributed values
    let mut unique_windows: Vec<Vec<i32>> = Vec::with_capacity(num_unique_windows);
    for _ in 0..num_unique_windows {
        let window: Vec<i32> = (0..window_size)
            .map(|_| rng.gen_range(0..i32::MAX))
            .collect(); // Uniformly distributed values
        unique_windows.push(window);
    }

    // Define the Zipf distribution for window frequencies
    let zipf =
        Zipf::new(num_unique_windows as u64, 1.03).expect("Invalid Zipf distribution parameters");

    // Assign frequencies to each window based on the Zipf distribution
    let mut final_windows: Vec<_> = Vec::with_capacity(num_unique_windows);
    for _ in 0..num_unique_windows {
        let freq = zipf.sample(&mut rng) as usize;
        final_windows.push(&unique_windows[freq - 1]);
    }

    // Build the final vector by repeating windows according to their scaled frequencies
    let mut vec: Vec<i32> = Vec::with_capacity(total_values);
    for window in final_windows.iter() {
        vec.extend_from_slice(window);
    }

    // Example: Create a PrimitiveArray<Int32Type>
    let array = Int32Array::from(vec);

    // Repeat the array 16 times to create a 16-column record batch
    let record_batch = create_record_batch(&array, 16);

    // Write the record batch to a Parquet file 16 times
    write_record_batch_to_parquet(&record_batch, "synthetic.parquet", 16 * 8 * 2);
}
