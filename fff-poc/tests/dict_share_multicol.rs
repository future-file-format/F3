use std::sync::Arc;

use arrow_array::{ArrayRef, Int32Array, RecordBatch};
use arrow_schema::{Field, Schema};
use fff_poc::{options::FileWriterOptions, reader::FileReaderV2Builder, writer::FileWriter};

#[test]
fn test_multi_col_share_dict() {
    let fields = vec![
        Field::new("a", arrow_schema::DataType::Int32, false),
        Field::new("b", arrow_schema::DataType::Int32, false),
        Field::new("c", arrow_schema::DataType::Int32, false),
        Field::new("d", arrow_schema::DataType::Int32, false),
        Field::new("e", arrow_schema::DataType::Int32, false),
    ];
    let columns = vec![
        Arc::new(Int32Array::from_iter_values(0..10000)) as ArrayRef,
        Arc::new(Int32Array::from_iter_values(13500..23500)) as ArrayRef,
        Arc::new(Int32Array::from_iter_values(39000..49000)) as ArrayRef,
        Arc::new(Int32Array::from_iter_values(3000..13000)) as ArrayRef,
        Arc::new(Int32Array::from_iter_values(13500..23500)) as ArrayRef,
    ];
    let col_count = columns.len();
    let schema = Arc::new(Schema::new(fields));
    let temp_file = Arc::new(tempfile::tempfile().unwrap());
    let file_cloned = temp_file.clone();
    let options = FileWriterOptions::builder()
        .set_dictionary_type(
            fff_poc::options::DictionaryTypeOptions::GlobalDictionaryMultiColSharing,
        )
        .build();
    let mut fff_writer = FileWriter::try_new(schema.clone(), temp_file, options).unwrap();
    let batch = RecordBatch::try_new(schema, columns).unwrap();
    let num_rows = batch.num_rows();
    fff_writer.write_batch(&batch.slice(0, num_rows)).unwrap();
    let counters = fff_writer.finish().unwrap();
    assert_eq!(counters.len(), col_count);
    let mut reader = FileReaderV2Builder::new(file_cloned).build().unwrap();
    let (shared_counters, sharing_peers) = reader.get_shared_dict_sizes().unwrap();
    assert_eq!(shared_counters.len(), col_count);
    assert_eq!(sharing_peers.len(), col_count);
    // Column 0 shares with column 3; while remains its own chunk
    assert_eq!(sharing_peers[0].len(), 1);
    assert_eq!(sharing_peers[0][0].0, 3);
    assert!(shared_counters[0].dict_size > sharing_peers[0][0].1);
    // Column 1 shares with column 4, which is the only chunk
    assert_eq!(sharing_peers[1].len(), 1);
    assert_eq!(sharing_peers[1][0].0, 4);
    assert_eq!(shared_counters[1].dict_size, sharing_peers[1][0].1);
    // Column 2 shares with no one
    assert_eq!(sharing_peers[2].len(), 0);
    // Column 3 shares with column 0; while remains its own chunk
    assert_eq!(sharing_peers[3].len(), 1);
    assert_eq!(sharing_peers[3][0].0, 0);
    assert!(shared_counters[3].dict_size > sharing_peers[3][0].1);
    // Column 4 shares with column 1, which is the only chunk
    assert_eq!(sharing_peers[4].len(), 1);
    assert_eq!(sharing_peers[4][0].0, 1);
    assert_eq!(shared_counters[4].dict_size, sharing_peers[4][0].1);

    eprintln!("Counters: {:?}", counters);
    eprintln!("Shared counters: {:?}", counters);
    eprintln!("Sharing peers: {:?}", sharing_peers);
}
