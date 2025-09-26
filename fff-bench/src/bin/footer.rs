/// This file is currently deprecated.
use std::{io::Seek, sync::Arc};

use arrow::{
    array::{ArrayRef, Float64Array, RecordBatch},
    datatypes::{DataType, Field, Schema},
};
use fff_poc::{options::FileWriterOptions, reader::FileReader, writer::FileWriter};

use fff_core::errors::Result;

fn generate_batch(num_columns: usize, num_rows: usize) -> RecordBatch {
    let mut fields = Vec::with_capacity(num_columns);

    for i in 0..num_columns {
        fields.push(Field::new(
            &format!("column_{}", i),
            DataType::Float64,
            false,
        ));
    }

    let schema = Arc::new(Schema::new(fields));

    let mut columns: Vec<ArrayRef> = Vec::with_capacity(num_columns);
    let array = {
        let mut v = Vec::with_capacity(num_rows);
        for _ in 0..num_rows {
            v.push(42.0);
        }
        Arc::new(Float64Array::from(v))
    };

    for _ in 0..num_columns {
        columns.push(array.clone());
    }
    RecordBatch::try_new(schema.clone(), columns).unwrap()
}

fn main() -> Result<()> {
    for num_columns in [1, 10, 100, 1000, 10000] {
        for num_rows in [1000, 10000, 100000] {
            let mut file = tempfile::tempfile().unwrap();
            {
                // create a record batch with 10k rows and 10000 columns, random floats
                let batch = generate_batch(num_columns, num_rows);
                let mut writer =
                    FileWriter::try_new(batch.schema(), &file, FileWriterOptions::default())?;
                for _ in 0..1 {
                    writer.write_batch(&batch).unwrap();
                }
                writer.finish().unwrap();
            }
            file.rewind().unwrap();
            let mut reader = FileReader::new(file);
            let postscript = reader.read_postscript().unwrap();
            let _footer = reader.read_footer(&postscript).unwrap_or_else(|e| {
                panic!("Error reading footer: {:?}", e);
            });
            println!(
                "num_rows: {}, num_cols:{}, footer_size: {}",
                num_rows, num_columns, postscript.metadata_size
            );
        }
    }
    Ok(())
}
