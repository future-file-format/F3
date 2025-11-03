use arrow::{
    array::AsArray,
    compute::take_record_batch,
    datatypes::{BinaryType, BinaryViewType, LargeUtf8Type, StringViewType, Utf8Type},
};
use core::panic;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use rstest::rstest;
use rstest_reuse::apply;
use std::{collections::HashMap, io::Seek, path::Path, sync::Arc};

use arrow::{
    array::{Int32Builder, ListBuilder},
    compute::concat_batches,
};
use arrow_array::{Array, ArrayRef, GenericByteViewArray, Int32Array, RecordBatch, UInt64Array};
use arrow_schema::{ArrowError, DataType, Field, Schema};
use fff_poc::{
    context::{WASMId, WasmLib},
    io::reader::{ObjectStoreReadAt, Reader},
    options::{CustomEncodingOptions, FileWriterOptions, FileWriterOptionsBuilder},
    reader::{FileReaderV2Builder, Projection, Selection},
    writer::FileWriter,
};
use object_store::{aws::AmazonS3Builder, ObjectStore};

fn read_parquet_file(file_path: impl AsRef<Path>, batch_size: usize) -> Vec<RecordBatch> {
    let parquet = std::fs::File::open(file_path).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(parquet).unwrap();
    let reader = builder.with_batch_size(batch_size).build().unwrap();

    reader.map(|batch_result| batch_result.unwrap()).collect()
}

/// This testing function assumes that the input Array is always String, not StringView
/// And the output Array may be StringView, and thus will convert input to StringView for compare.
fn array_equal(i: &Arc<dyn Array>, o: &Arc<dyn Array>) {
    assert_eq!(i.len(), o.len());

    if o.data_type().is_primitive() || matches!(o.data_type(), DataType::Binary | DataType::Utf8) {
        assert_eq!(i, o)
    } else if o.as_byte_view_opt::<StringViewType>().is_some() {
        match *i.data_type() {
            DataType::Utf8 => {
                let i: GenericByteViewArray<StringViewType> =
                    GenericByteViewArray::from(i.as_bytes::<Utf8Type>());
                assert_eq!(&(Arc::new(i) as Arc<dyn Array>), o);
            }
            // This is from the taxi data set
            DataType::LargeUtf8 => {
                let i: GenericByteViewArray<StringViewType> =
                    GenericByteViewArray::from(i.as_bytes::<LargeUtf8Type>());
                assert_eq!(&(Arc::new(i) as Arc<dyn Array>), o);
            }
            _ => panic!(),
        }
    } else if o.as_byte_view_opt::<BinaryViewType>().is_some() {
        let i: GenericByteViewArray<BinaryViewType> =
            GenericByteViewArray::from(i.as_bytes::<BinaryType>());
        assert_eq!(&(Arc::new(i) as Arc<dyn Array>), o);
    } else if let DataType::Struct(_) = o.data_type() {
        let i = i.as_struct();
        let o = o.as_struct();
        for (i, o) in i.columns().iter().zip(o.columns().iter()) {
            array_equal(i, o);
        }
    } else if let DataType::List(_) = o.data_type() {
        let i = i.as_list::<i32>();
        let o = o.as_list::<i32>();
        array_equal(i.values(), o.values());
    } else {
        unimplemented!()
    }
}

fn test_read<R: Reader + Clone>(
    file1: R,
    input_batches: &[RecordBatch],
    proj: Projection,
    selection: Selection,
) {
    let mut reader = FileReaderV2Builder::new(file1)
        .with_projections(proj.clone())
        .with_selection(selection.clone())
        .build()
        .unwrap();
    let output_batches = reader.read_file().unwrap();
    let input_single_batch = concat_batches(input_batches[0].schema_ref(), input_batches).unwrap();
    let output_single_batch =
        concat_batches(output_batches[0].schema_ref(), &output_batches).unwrap();
    let input_single_batch = match proj {
        Projection::All => input_single_batch,
        Projection::LeafColumnIndexes(indexes) => input_single_batch.project(&indexes).unwrap(),
    };
    let input_single_batch = match selection {
        Selection::All => input_single_batch,
        Selection::RowIndexes(indexes) => {
            take_record_batch(&input_single_batch, &UInt64Array::from(indexes)).unwrap()
        }
    };
    for (i_col, o_col) in input_single_batch
        .columns()
        .iter()
        .zip(output_single_batch.columns().iter())
    {
        // println!("{i}");
        // if i == 8 {
        //     println!("{:?}", input_batches[0].column(8).slice(0, 5));
        //     println!("{:?}", output_batches[0].column(8).slice(0, 5));
        // }
        array_equal(i_col, o_col);
    }
    // assert_eq!(input_single_batch, output_single_batch);
}

fn write_batches(
    file: &mut std::fs::File,
    input_batches: &[RecordBatch],
    options: FileWriterOptions,
) {
    let mut writer = FileWriter::try_new(input_batches[0].schema(), file, options).unwrap();
    for batch in input_batches {
        writer.write_batch(batch).unwrap();
    }
    writer.finish().unwrap();
}

fn test_read_file_roundtrip(
    input_batches: &[RecordBatch],
    proj: Projection,
    options: FileWriterOptions,
    selection: Selection,
) {
    let mut file = tempfile::tempfile().unwrap();
    write_batches(&mut file, input_batches, options);
    file.rewind().unwrap();
    test_read(Arc::new(file), input_batches, proj, selection);
}

#[rstest_reuse::template]
#[rstest]
// #[case(true)]
#[case(false)]
fn enable_built_in_wasm(#[case] a: bool) {}

#[apply(enable_built_in_wasm)]
fn test_basic_roundtrip(#[case] enable_built_in_wasm: bool) {
    let schema = Schema::new(vec![
        Field::new("a", DataType::Int32, false),
        Field::new("b", DataType::Int32, true),
    ]);
    // create some data
    let a = Int32Array::from(vec![1, 2, 3, 4, 5]);
    let b = Int32Array::from(vec![5, 4, 3, 2, 1]);

    // build a record batch
    let input_batch =
        RecordBatch::try_new(Arc::new(schema), vec![Arc::new(a), Arc::new(b)]).unwrap();
    test_read_file_roundtrip(
        &[input_batch],
        Projection::default(),
        FileWriterOptionsBuilder::with_defaults()
            .write_built_in_wasm(enable_built_in_wasm)
            .build(),
        Selection::default(),
    );
}

#[apply(enable_built_in_wasm)]
fn test_basic_null_roundtrip(#[case] enable_built_in_wasm: bool) {
    let schema = Schema::new(vec![
        Field::new("a", DataType::Int32, false),
        Field::new("b", DataType::Int32, true),
    ]);
    let a = Int32Array::from(vec![1, 2, 3, 4, 5]);
    let b = Int32Array::from(vec![Some(5), Some(4), None, Some(2), Some(1)]);

    let input_batch =
        RecordBatch::try_new(Arc::new(schema), vec![Arc::new(a), Arc::new(b)]).unwrap();
    test_read_file_roundtrip(
        &[input_batch],
        Projection::default(),
        FileWriterOptionsBuilder::with_defaults()
            .write_built_in_wasm(enable_built_in_wasm)
            .build(),
        Selection::default(),
    );
}

#[apply(enable_built_in_wasm)]
fn test_all_null(#[case] enable_built_in_wasm: bool) {
    let schema = Schema::new(vec![Field::new("a", DataType::Int32, true)]);
    let a = Int32Array::from(vec![None, None, None, None]);

    let input_batch = RecordBatch::try_new(Arc::new(schema), vec![Arc::new(a)]).unwrap();
    test_read_file_roundtrip(
        &[input_batch],
        Projection::default(),
        FileWriterOptionsBuilder::with_defaults()
            .write_built_in_wasm(enable_built_in_wasm)
            .build(),
        Selection::default(),
    );
}

#[apply(enable_built_in_wasm)]
fn test_no_null(#[case] enable_built_in_wasm: bool) {
    let schema = Schema::new(vec![Field::new("a", DataType::Int32, true)]);
    let a = Int32Array::from(vec![1, 2, 3, 4]);

    let input_batch = RecordBatch::try_new(Arc::new(schema), vec![Arc::new(a)]).unwrap();
    test_read_file_roundtrip(
        &[input_batch],
        Projection::default(),
        FileWriterOptionsBuilder::with_defaults()
            .write_built_in_wasm(enable_built_in_wasm)
            .build(),
        Selection::default(),
    );
}

#[apply(enable_built_in_wasm)]
fn test_64k_data(#[case] enable_built_in_wasm: bool) {
    let schema = Schema::new(vec![Field::new("a", DataType::Int32, true)]);
    let a = Int32Array::from((0..65536).collect::<Vec<_>>());

    let batch1 = RecordBatch::try_new(Arc::new(schema.clone()), vec![Arc::new(a)]).unwrap();
    test_read_file_roundtrip(
        &[batch1],
        Projection::default(),
        FileWriterOptionsBuilder::with_defaults()
            .write_built_in_wasm(enable_built_in_wasm)
            .build(),
        Selection::default(),
    );
}

#[test]
fn test_basic_list() {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "a",
        DataType::List(Arc::new(Field::new("item", DataType::Int32, true))),
        true,
    )]));
    let mut builder = ListBuilder::new(Int32Builder::new());
    builder.append_value([Some(1), Some(2), Some(3)]);
    builder.append_value([Some(4), Some(5)]);
    builder.append_value([None]);
    builder.append_value([Some(6), Some(7), Some(8), Some(9)]);
    let a = builder.finish();
    let input_batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(a)]).unwrap();
    test_read_file_roundtrip(
        &[input_batch],
        Projection::default(),
        FileWriterOptions::default(),
        Selection::default(),
    );
}

#[test]
fn test_complex_list_roundtrip() {
    use lance_datagen::{array, gen, BatchCount, RowCount};
    let non_nullable_list = DataType::List(Arc::new(Field::new("a", DataType::Int32, false)));
    let nullable_list = DataType::List(Arc::new(Field::new("b", DataType::Int32, true)));
    let reader = gen()
        .col("location", array::rand_type(&non_nullable_list))
        .col("categories", array::rand_type(&nullable_list))
        .into_reader_rows(RowCount::from(65536), BatchCount::from(100));

    let input_batchs = reader
        .into_iter()
        .collect::<Result<Vec<_>, ArrowError>>()
        .unwrap();
    test_read_file_roundtrip(
        &input_batchs,
        Projection::default(),
        FileWriterOptions::default(),
        Selection::default(),
    );
}

#[test]
fn test_complex_list_roundtrip2() {
    use lance_datagen::{array, gen, BatchCount, RowCount};
    let list_of_list = DataType::List(Arc::new(Field::new(
        "a",
        DataType::List(Arc::new(Field::new("b", DataType::Int32, false))),
        true,
    )));
    let reader = gen()
        .col("l_o_l", array::rand_type(&list_of_list))
        .into_reader_rows(RowCount::from(65536), BatchCount::from(100));

    let input_batchs = reader
        .into_iter()
        .collect::<Result<Vec<_>, ArrowError>>()
        .unwrap();
    test_read_file_roundtrip(
        &input_batchs,
        Projection::default(),
        FileWriterOptions::default(),
        Selection::default(),
    );
}

#[test]
fn test_projection() {
    let schema = Schema::new(vec![
        Field::new("a", DataType::Int32, false),
        Field::new("b", DataType::Int32, true),
    ]);
    // create some data
    let a = Int32Array::from(vec![1, 2, 3, 4, 5]);
    let b = Int32Array::from(vec![5, 4, 3, 2, 1]);

    // build a record batch
    let input_batch =
        RecordBatch::try_new(Arc::new(schema), vec![Arc::new(a), Arc::new(b)]).unwrap();
    test_read_file_roundtrip(
        &[input_batch],
        Projection::LeafColumnIndexes(vec![0]),
        FileWriterOptions::default(),
        Selection::default(),
    );
}

#[apply(enable_built_in_wasm)]
fn test_row_selection_basic(#[case] enable_built_in_wasm: bool) {
    let schema = Schema::new(vec![
        Field::new("a", DataType::Int32, false),
        Field::new("b", DataType::Int32, true),
    ]);
    // create some data
    let a = Int32Array::from(vec![1, 2, 3, 4, 5]);
    let b = Int32Array::from(vec![5, 4, 3, 2, 1]);

    // build a record batch
    let input_batch =
        RecordBatch::try_new(Arc::new(schema), vec![Arc::new(a), Arc::new(b)]).unwrap();
    test_read_file_roundtrip(
        &[input_batch],
        Projection::default(),
        FileWriterOptionsBuilder::with_defaults()
            .write_built_in_wasm(enable_built_in_wasm)
            .build(),
        Selection::RowIndexes(vec![3]),
    );
}

#[apply(enable_built_in_wasm)]
fn test_row_selection_taxi(#[case] enable_built_in_wasm: bool) {
    let original_file = bench_vortex::taxi_data::taxi_data_parquet();

    let batches: Vec<RecordBatch> = read_parquet_file(&original_file, 64 * 1024);

    test_read_file_roundtrip(
        &batches,
        Projection::default(),
        FileWriterOptionsBuilder::with_defaults()
            .write_built_in_wasm(enable_built_in_wasm)
            .build(),
        Selection::RowIndexes(vec![64 * 1024 + 42]),
    );

    test_read_file_roundtrip(
        &batches,
        Projection::default(),
        FileWriterOptionsBuilder::with_defaults()
            .write_built_in_wasm(enable_built_in_wasm)
            .build(),
        Selection::RowIndexes(vec![3339715 - 3]), // this parquet has 3339715 rows in total.
    );
}

#[apply(enable_built_in_wasm)]
fn test_string(#[case] enable_built_in_wasm: bool) {
    let schema = Schema::new(vec![Field::new("a", DataType::Utf8, true)]);
    let a = arrow::array::StringArray::from(vec![Some("a"), Some("b"), None, Some("d")]);

    let input_batch = RecordBatch::try_new(Arc::new(schema), vec![Arc::new(a)]).unwrap();
    test_read_file_roundtrip(
        &[input_batch],
        Projection::default(),
        FileWriterOptionsBuilder::with_defaults()
            .write_built_in_wasm(enable_built_in_wasm)
            .build(),
        Selection::default(),
    );
}

#[apply(enable_built_in_wasm)]
fn test_compressible_string(#[case] enable_built_in_wasm: bool) {
    let schema = Schema::new(vec![Field::new("a", DataType::Utf8, true)]);
    let a = arrow::array::StringArray::from(vec![Some("a"); 65536]);

    let input_batch = RecordBatch::try_new(Arc::new(schema), vec![Arc::new(a)]).unwrap();
    test_read_file_roundtrip(
        &[input_batch],
        Projection::default(),
        FileWriterOptionsBuilder::with_defaults()
            .write_built_in_wasm(enable_built_in_wasm)
            .build(),
        Selection::default(),
    );
}

#[apply(enable_built_in_wasm)]
fn test_struct(#[case] enable_built_in_wasm: bool) {
    let b_field = Arc::new(Field::new("b", DataType::Int32, true));
    let c_field = Arc::new(Field::new("c", DataType::Utf8, true));
    let schema = Schema::new(vec![Field::new(
        "a",
        DataType::Struct(vec![b_field.clone(), c_field.clone()].into()),
        true,
    )]);
    let b = Arc::new(Int32Array::from(vec![Some(1), Some(2), None, Some(4)])) as ArrayRef;
    let c = Arc::new(arrow::array::StringArray::from(vec![
        Some("a"),
        Some("b"),
        None,
        Some("d"),
    ])) as ArrayRef;
    let a = arrow::array::StructArray::from(vec![(b_field, b), (c_field, c)]);
    let input_batch = RecordBatch::try_new(Arc::new(schema), vec![Arc::new(a)]).unwrap();
    test_read_file_roundtrip(
        &[input_batch],
        Projection::default(),
        FileWriterOptionsBuilder::with_defaults()
            .write_built_in_wasm(enable_built_in_wasm)
            .build(),
        Selection::default(),
    );
}

#[test]
fn test_list_of_struct() {
    use lance_datagen::{array, gen, BatchCount, RowCount};
    let list_of_struct = DataType::List(Arc::new(Field::new(
        "a",
        DataType::Struct(
            vec![
                Field::new("b", DataType::Int32, true),
                Field::new("c", DataType::Utf8, true),
            ]
            .into(),
        ),
        true,
    )));
    let reader = gen()
        .col("l_o_s", array::rand_type(&list_of_struct))
        .into_reader_rows(RowCount::from(5), BatchCount::from(1));
    let input_batches = reader
        .into_iter()
        .collect::<Result<Vec<_>, ArrowError>>()
        .unwrap();
    let mut file = tempfile::tempfile().unwrap();
    {
        write_batches(&mut file, &input_batches, FileWriterOptions::default());
    }
    file.rewind().unwrap();
    let mut reader = FileReaderV2Builder::new(Arc::new(file)).build().unwrap();
    let output_batches = reader
        .point_access_list_struct(1, input_batches[0].schema_ref().fields[0].clone(), 2)
        .unwrap();
    println!("{:?}", output_batches);
}

#[ignore]
#[tokio::test]
async fn test_object_store() {
    let schema = Schema::new(vec![
        Field::new("a", DataType::Int32, false),
        Field::new("b", DataType::Int32, true),
    ]);
    // create some data
    let a = Int32Array::from(vec![1, 2, 3, 4, 5]);
    let b = Int32Array::from(vec![5, 4, 3, 2, 1]);

    // build a record batch
    let input_batch =
        RecordBatch::try_new(Arc::new(schema), vec![Arc::new(a), Arc::new(b)]).unwrap();
    let file_name = "test_object_store.fff";
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("data")
                .join(file_name),
        )
        .unwrap();
    write_batches(
        &mut file,
        &[input_batch.clone()],
        FileWriterOptions::default(),
    );
    let bucket = Arc::new(
        AmazonS3Builder::from_env()
            .with_url("s3://future-file-format/")
            .with_retry({
                object_store::RetryConfig {
                    retry_timeout: std::time::Duration::from_secs(10),
                    max_retries: 1,
                    ..Default::default()
                }
            })
            .build()
            .unwrap(),
    );
    let file1 = ObjectStoreReadAt::new(
        bucket.clone(),
        object_store::path::Path::from(file_name).into(),
    );
    test_read(file1, &[input_batch], Projection::All, Selection::default());
}

#[ignore]
#[tokio::test]
async fn test_s3() {
    let bucket = Arc::new(
        AmazonS3Builder::from_env()
            .with_url("s3://future-file-format/")
            .with_retry({
                object_store::RetryConfig {
                    retry_timeout: std::time::Duration::from_secs(10),
                    max_retries: 1,
                    ..Default::default()
                }
            })
            .build()
            .unwrap(),
    );
    bucket
        .get(&object_store::path::Path::from("test_object_store.fff"))
        .await
        .unwrap();

    println!(
        "{:?}",
        bucket
            .head(&object_store::path::Path::from("test_object_store.fff"))
            .await
            .unwrap() // futures::executor::block_on(async move {
                      //     .spawn(async move {
                      //         bucket
                      //             .head(&object_store::path::Path::from("test_object_store.fff"))
                      //             .await
                      //     })
                      // })
                      // .unwrap()
    );
}

#[apply(enable_built_in_wasm)]
#[ignore]
fn test_taxi(#[case] enable_built_in_wasm: bool) {
    let original_file = bench_vortex::taxi_data::taxi_data_parquet();

    let batches: Vec<RecordBatch> = read_parquet_file(&original_file, 64 * 1024);

    test_read_file_roundtrip(
        &batches,
        Projection::default(),
        FileWriterOptionsBuilder::with_defaults()
            .write_built_in_wasm(enable_built_in_wasm)
            .build(),
        Selection::default(),
    );
}

#[apply(enable_built_in_wasm)]
fn test_multi_row_group(#[case] enable_built_in_wasm: bool) {
    use lance_datagen::{array, gen, BatchCount, RowCount};
    let reader = gen()
        .col("a", array::rand_type(&DataType::Int32))
        .col("b", array::rand_type(&DataType::Int32))
        .into_reader_rows(RowCount::from(65536), BatchCount::from(16 * 64));

    let batches = reader
        .into_iter()
        .collect::<Result<Vec<_>, ArrowError>>()
        .unwrap();

    let test = |row_group_size: u64| {
        test_read_file_roundtrip(
            &batches,
            Projection::default(),
            FileWriterOptionsBuilder::with_defaults()
                .write_built_in_wasm(enable_built_in_wasm)
                .set_row_group_size(row_group_size)
                .build(),
            Selection::default(),
        );
    };

    test(64 * 1024);
    test(1024 * 1024);
}

#[apply(enable_built_in_wasm)]
fn test_multi_row_group_selection(#[case] enable_built_in_wasm: bool) {
    use lance_datagen::{array, gen, BatchCount, RowCount};
    let reader = gen()
        .col("a", array::rand_type(&DataType::Int32))
        .col("b", array::rand_type(&DataType::Int32))
        .into_reader_rows(RowCount::from(65536), BatchCount::from(16 * 64));

    let batches = reader
        .into_iter()
        .collect::<Result<Vec<_>, ArrowError>>()
        .unwrap();

    let test = |row_group_size: u64, selected_row: u64| {
        test_read_file_roundtrip(
            &batches,
            Projection::default(),
            FileWriterOptionsBuilder::with_defaults()
                .write_built_in_wasm(enable_built_in_wasm)
                .set_row_group_size(row_group_size)
                .build(),
            Selection::RowIndexes(vec![selected_row]),
        );
    };

    use rand::Rng;
    let mut rng = rand::thread_rng();
    const NUM_TESTS: usize = 3;
    // Test with 64KB row group size
    for _ in 0..NUM_TESTS {
        let row = rng.gen_range(0..64 * 1024 * 1024);
        test(64 * 1024, row);
    }

    // Test with 1MB row group size
    for _ in 0..5 {
        let row = rng.gen_range(0..64 * 1024 * 1024);
        test(1024 * 1024, row);
    }
}

#[apply(enable_built_in_wasm)]
fn test_multi_row_group_projection(#[case] enable_built_in_wasm: bool) {
    use lance_datagen::{array, gen, BatchCount, RowCount};
    let reader = gen()
        .col("a", array::rand_type(&DataType::Int32))
        .col("b", array::rand_type(&DataType::Int32))
        .into_reader_rows(RowCount::from(65536), BatchCount::from(16 * 64));

    let batches = reader
        .into_iter()
        .collect::<Result<Vec<_>, ArrowError>>()
        .unwrap();

    let test = |row_group_size: u64| {
        test_read_file_roundtrip(
            &batches,
            Projection::LeafColumnIndexes(vec![1]),
            FileWriterOptionsBuilder::with_defaults()
                .write_built_in_wasm(enable_built_in_wasm)
                .set_row_group_size(row_group_size)
                .build(),
            Selection::default(),
        );
    };

    test(64 * 1024);
    test(1024 * 1024);
}

#[apply(enable_built_in_wasm)]
#[ignore]
fn test_core(#[case] enable_built_in_wasm: bool) {
    // let original_file = "/mnt/nvme0n1/xinyu/tmp/12_str.parquet";
    // let original_file = "/mnt/nvme0n1/xinyu/tmp/core_no_double.parquet";
    let original_file = "/mnt/nvme0n1/xinyu/data/parquet/core.parquet";
    let batches: Vec<RecordBatch> = read_parquet_file(original_file, 64 * 1024);
    test_read_file_roundtrip(
        &batches,
        Projection::default(),
        FileWriterOptionsBuilder::with_defaults()
            .write_built_in_wasm(enable_built_in_wasm)
            .build(),
        Selection::default(),
    );
}

#[test]
#[ignore]
fn test_pco_custom_wasm() {
    let schema = Schema::new(vec![
        Field::new("a", DataType::Int32, false),
        Field::new("b", DataType::Int32, true),
        Field::new("c", DataType::Utf8, true),
    ]);
    // create some data
    let a = Int32Array::from(vec![1, 2, 3, 4, 5]);
    let b = Int32Array::from(vec![5, 4, 3, 2, 1]);
    let c = arrow::array::StringArray::from(vec![Some("a"), Some("b"), None, Some("d"), Some("z")]);
    let batches: Vec<RecordBatch> = vec![RecordBatch::try_new(
        Arc::new(schema),
        vec![Arc::new(a), Arc::new(b), Arc::new(c)],
    )
    .unwrap()];
    let wasms = HashMap::from([(
        WASMId(0),
        WasmLib::new(
            "/home/xinyu/fff-devel/target/release/libfff_ude_example_pco_real_encoder.so".into(),
            std::fs::read("/home/xinyu/fff-devel/target/wasm32-wasip1/opt-size-lvl3/fff_ude_example_pco_real.wasm").unwrap(),
        ),
    )]);
    let mut data_type_to_wasm_id: HashMap<DataType, WASMId> = HashMap::new();
    data_type_to_wasm_id.insert(DataType::UInt16, WASMId(0));
    data_type_to_wasm_id.insert(DataType::Int16, WASMId(0));
    data_type_to_wasm_id.insert(DataType::UInt32, WASMId(0));
    data_type_to_wasm_id.insert(DataType::Int32, WASMId(0));
    data_type_to_wasm_id.insert(DataType::UInt64, WASMId(0));
    data_type_to_wasm_id.insert(DataType::Int64, WASMId(0));
    data_type_to_wasm_id.insert(DataType::Float32, WASMId(0));
    data_type_to_wasm_id.insert(DataType::Float64, WASMId(0));
    let custom_encoding_options = CustomEncodingOptions::new(wasms, data_type_to_wasm_id);
    test_read_file_roundtrip(
        &batches,
        Projection::default(),
        FileWriterOptions::builder()
            .set_custom_encoding_options(custom_encoding_options)
            .build(),
        Selection::default(),
    );
}

#[apply(enable_built_in_wasm)]
fn test_compression(#[case] enable_built_in_wasm: bool) {
    use arrow_array::{Int8Array, RecordBatch};
    use fff_format::File::fff::flatbuf::CompressionType;

    // Create repetitive pattern data for better compression testing
    let batch_size = 65536;
    let num_batches = 16;

    // Generate patterned data: repeating sequences for better compression
    let mut batches: Vec<RecordBatch> = Vec::with_capacity(num_batches);
    let schema = Arc::new(Schema::new(vec![
        Field::new("a", DataType::Int8, false),
        Field::new("b", DataType::Int8, false),
    ]));

    for batch_idx in 0..num_batches {
        // Create a repeating pattern: 0,1,2,3,4,5,6,7,0,1,2,3,4,5,6,7,...
        let pattern_length = 8;
        let mut a_values = Vec::with_capacity(batch_size);
        let mut b_values = Vec::with_capacity(batch_size);

        for i in 0..batch_size {
            // For column 'a': repeating pattern of 0-7
            a_values.push((i % pattern_length) as i8);
            // For column 'b': repeating pattern with offset based on batch index
            b_values.push(((i + batch_idx) % (pattern_length * 2)) as i8);
        }

        let a_array = Int8Array::from(a_values);
        let b_array = Int8Array::from(b_values);

        let batch =
            RecordBatch::try_new(schema.clone(), vec![Arc::new(a_array), Arc::new(b_array)])
                .unwrap();

        batches.push(batch);
    }

    test_read_file_roundtrip(
        &batches,
        Projection::default(),
        FileWriterOptionsBuilder::with_defaults()
            .write_built_in_wasm(enable_built_in_wasm)
            .set_compression_type(CompressionType::Uncompressed)
            .build(),
        Selection::default(),
    );

    test_read_file_roundtrip(
        &batches,
        Projection::default(),
        FileWriterOptionsBuilder::with_defaults()
            .write_built_in_wasm(enable_built_in_wasm)
            .set_compression_type(CompressionType::Lz4)
            .build(),
        Selection::default(),
    );

    test_read_file_roundtrip(
        &batches,
        Projection::default(),
        FileWriterOptionsBuilder::with_defaults()
            .write_built_in_wasm(enable_built_in_wasm)
            .set_compression_type(CompressionType::Zstd)
            .build(),
        Selection::default(),
    );
}
