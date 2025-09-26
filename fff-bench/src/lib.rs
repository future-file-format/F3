#![feature(exit_status_error)]
pub mod bench_data;
pub mod config;
pub mod helper;
use anyhow::Result;
use fff_ude_wasm::Runtime;
use futures::StreamExt;
use humansize::{format_size, DECIMAL};
use log::error;
use object_store::{path::Path, ObjectStore};
use parquet::{
    arrow::{
        arrow_reader::{ParquetRecordBatchReaderBuilder, RowSelection},
        async_reader::ParquetObjectReader,
        ArrowWriter, ParquetRecordBatchStreamBuilder, ProjectionMask,
    },
    file::reader::ChunkReader,
};
use std::{
    collections::HashMap,
    env::temp_dir,
    fs::{create_dir_all, File, OpenOptions},
    os::unix::fs::MetadataExt,
    sync::Arc,
};
use std::{path::PathBuf, process::Command};
use vortex_file::{
    LayoutContext, LayoutDeserializer, Projection, VortexFileWriter, VortexReadBuilder,
};
use vortex_io::TokioFile;
use vortex_sampling_compressor::{SamplingCompressor, ALL_ENCODINGS_CONTEXT};

use arrow_array::{ArrayRef, RecordBatch};
use fff_poc::{
    context::WASMId,
    options::FileWriterOptions,
    reader::{FileReaderV2Builder, Selection},
    writer::FileWriter,
};
use futures_util::TryStreamExt;
use std::io::{BufWriter, Read, Seek, Write};
use vortex_array::{compress::CompressionStrategy, IntoCanonical};

use arrow_array::UInt32Array;
use bytes::BytesMut;
use fff_encoding::schemes::Encoder;
use lance_file::v2::writer::FileWriterOptions as LanceFileWriterOptions;

pub trait IdempotentPath {
    fn to_data_path(&self) -> PathBuf;
    fn to_temp_path(&self) -> PathBuf;
}

impl IdempotentPath for str {
    fn to_data_path(&self) -> PathBuf {
        let path = config::get_base_data_path().join(self);
        if !path.parent().unwrap().exists() {
            create_dir_all(path.parent().unwrap()).unwrap();
        }
        path
    }

    fn to_temp_path(&self) -> PathBuf {
        let temp_dir = temp_dir().join(uuid::Uuid::new_v4().to_string());
        if !temp_dir.exists() {
            create_dir_all(temp_dir.clone()).unwrap();
        }
        temp_dir.join(self)
    }
}

pub fn generate_data(size: usize) -> ArrayRef {
    let vec: Vec<u32> = (1..=size).map(|x| x as u32 % 128).collect();
    let arr = UInt32Array::from(vec);
    Arc::new(arr) as ArrayRef
}

pub fn encode(encoder: impl Encoder, arr: ArrayRef) -> BytesMut {
    let encunit = encoder.encode(arr).unwrap();
    let mut file = tempfile::tempfile().unwrap();
    {
        let mut writer = encunit.try_serialize(BufWriter::new(&file)).unwrap();
        writer.flush().unwrap();
    }
    file.rewind().unwrap();
    let mut buf = vec![];
    file.read_to_end(&mut buf).unwrap();
    let mut bytes = BytesMut::with_capacity(buf.len());
    bytes.extend_from_slice(&buf);
    bytes
}

pub fn write_fff(batches: &[RecordBatch], fff: &File, options: FileWriterOptions) -> Result<()> {
    let mut fff_writer = FileWriter::try_new(batches[0].schema(), fff, options).unwrap();
    let mut memory_usage_sum = 0;
    let mut cnt = 0;
    for batch in batches {
        fff_writer.write_batch(&batch).unwrap();
        if fff_writer.memory_size() != 0 {
            memory_usage_sum += fff_writer.memory_size();
            cnt += 1;
        }
        // if cnt % 16 == 0 {
        //     fff_writer.flush_pending_chunks().unwrap();
        // }
    }
    error!("FFF memory usage: {}", memory_usage_sum / cnt);
    fff_writer.finish().unwrap();
    Ok(())
}

#[derive(Default)]
pub struct ReadFFFOpt {
    pub projections: Option<fff_poc::reader::Projection>,
    pub selection: Option<Selection>,
}

pub fn read_fff(pathbuf: PathBuf, opt: ReadFFFOpt) -> Result<Vec<RecordBatch>> {
    if pathbuf.starts_with("s3://") {
        let path = pathbuf.as_path().to_str().unwrap();
        let bucket = Arc::new(
            object_store::aws::AmazonS3Builder::from_env()
                .with_url(path)
                .with_retry({
                    let mut res = object_store::RetryConfig::default();
                    res.retry_timeout = std::time::Duration::from_secs(10);
                    res.max_retries = 1;
                    res
                })
                .build()
                .unwrap(),
        );
        // get the file name from the path, without the bucket name
        // count the number of slashes
        let slash_count = path.chars().filter(|&c| c == '/').count();
        assert!(slash_count == 3, "only support s3://bucket/file_name");
        let file_name = path.split('/').last().unwrap();
        let f1 = fff_poc::io::reader::ObjectStoreReadAt::new(
            bucket.clone(),
            object_store::path::Path::from(file_name).into(),
        );
        let mut reader = FileReaderV2Builder::new(Arc::new(f1))
            .with_projections(opt.projections.clone().unwrap_or_default())
            .with_selection(opt.selection.clone().unwrap_or_default())
            .build()
            .unwrap();
        Ok(reader.read_file().unwrap())
    } else {
        let f1 = OpenOptions::new().read(true).open(pathbuf.clone()).unwrap();

        let mut reader = FileReaderV2Builder::new(Arc::new(f1))
            .with_projections(opt.projections.clone().unwrap_or_default())
            .with_selection(opt.selection.clone().unwrap_or_default())
            .build()
            .unwrap();
        Ok(reader.read_file().unwrap())
    }
}

pub fn read_fff_aot_wasm(
    pathbuf: PathBuf,
    wasm_rts: HashMap<WASMId, Arc<Runtime>>,
) -> Result<Vec<RecordBatch>> {
    let f1 = OpenOptions::new().read(true).open(pathbuf.clone()).unwrap();
    let mut reader = FileReaderV2Builder::new(Arc::new(f1))
        .with_existing_runtimes(wasm_rts)
        .build()
        .unwrap();
    Ok(reader.read_file().unwrap())
}

pub fn write_parquet(batches: &[RecordBatch], parquet: &File) -> Result<()> {
    let mut parquet_writer = ArrowWriter::try_new(parquet, batches[0].schema(), None).unwrap();
    for batch in batches {
        parquet_writer.write(&batch).unwrap();
    }
    parquet_writer.close().unwrap();
    Ok(())
}

fn get_object_store_path_from_local_str(path: &str) -> object_store::path::Path {
    let dir_path = std::env::current_dir()
        .unwrap()
        .as_path()
        .to_str()
        .unwrap()
        .to_owned();
    let dir_path = Path::parse(dir_path).unwrap();
    path.split('/').fold(dir_path, |acc, x| acc.child(x))
}

pub async fn write_lance(
    batches: &[RecordBatch],
    path: &str,
    full_path: bool,
    options: LanceFileWriterOptions,
) -> Result<()> {
    let object_store = Arc::new(lance_io::object_store::ObjectStore::local());
    let res = if full_path {
        Path::from(path)
    } else {
        get_object_store_path_from_local_str(path)
    };
    let writer = object_store.create(&res).await.unwrap();

    let lance_schema =
        lance_core::datatypes::Schema::try_from(batches[0].schema().as_ref()).unwrap();

    let mut file_writer =
        lance_file::v2::writer::FileWriter::try_new(writer, lance_schema.clone(), options).unwrap();
    let mut memory_usage_sum = 0;
    let mut cnt = 0;
    for batch in batches {
        file_writer.write_batch(batch).await.unwrap();
        if file_writer.max_mem_bytes() != 0 {
            memory_usage_sum += file_writer.max_mem_bytes();
            cnt += 1;
        }
    }
    error!(
        "Lance memory usage: {}",
        if cnt != 0 { memory_usage_sum / cnt } else { 0 }
    );
    // let field_id_mapping = file_writer.field_id_to_column_indices().to_vec();
    // file_writer.add_schema_metadata("foo", "bar");
    file_writer.finish().await.unwrap();
    Ok(())
}

pub async fn read_lance(
    path: &str,
    proj_columns: Option<Vec<&str>>,
    row_ids: Option<Vec<usize>>,
    full_path: bool,
) -> Result<usize> {
    let object_store = Arc::new({
        let res = if path.starts_with("s3://") {
            let (store, _) = lance_io::object_store::ObjectStore::from_uri(path)
                .await
                .unwrap();
            store
        } else {
            let mut res = lance_io::object_store::ObjectStore::local();
            res.set_io_parallelism(1);
            res
        };
        res
    });
    let path = if full_path {
        if path.starts_with("s3://") {
            let (_, path) = lance_io::object_store::ObjectStore::from_uri(path)
                .await
                .unwrap();
            path
        } else {
            Path::from(path)
        }
    } else {
        get_object_store_path_from_local_str(path)
    };
    let scheduler = lance_io::scheduler::ScanScheduler::new(
        object_store,
        lance_io::scheduler::SchedulerConfig::default_for_testing(),
    );
    let file_scheduler = scheduler.open_file(&path).await.unwrap();
    let file_reader = lance_file::v2::reader::FileReader::try_open(
        file_scheduler.clone(),
        None,
        Arc::<lance_encoding::decoder::DecoderPlugins>::default(),
        &lance_file::v2::testing::test_cache(),
        lance_file::v2::reader::FileReaderOptions::default(),
    )
    .await
    .unwrap();
    let params = row_ids
        .map(|ids| {
            lance_io::ReadBatchParams::Indices(UInt32Array::from(
                ids.iter().map(|&x| x as u32).collect::<Vec<_>>(),
            ))
        })
        .unwrap_or(lance_io::ReadBatchParams::RangeFull);
    let batch_stream = if let Some(columns) = proj_columns {
        let schema = file_reader.schema();
        let projected_schema = schema.project(&columns).unwrap();
        let projection = lance_file::v2::reader::ReaderProjection::from_column_names(
            &projected_schema,
            &columns,
        )
        .unwrap();
        file_reader
            .read_stream_projected(
                params,
                64 * 1024 * 1024,
                1, // No CPU parallelism for now.
                projection.clone(),
                lance_encoding::decoder::FilterExpression::no_filter(),
            )
            .unwrap()
    } else {
        file_reader
            .read_stream(
                params,
                64 * 1024 * 1024,
                1, // No CPU parallelism for now.
                lance_encoding::decoder::FilterExpression::no_filter(),
            )
            .unwrap()
    };
    let results: Vec<_> = batch_stream.try_collect().await.unwrap();
    let mut nbytes = 0;
    for batch in results {
        nbytes += batch.get_array_memory_size();
    }
    Ok(nbytes)
}

pub fn write_orc(batches: &[RecordBatch], path: &str) -> Result<()> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(path)?;
    let mut writer = orc_rust::ArrowWriterBuilder::new(file, batches[0].schema())
        .try_build()
        .unwrap();
    for batch in batches {
        writer.write(batch).unwrap();
    }
    writer.close().unwrap();
    Ok(())
}

pub fn read_orc(path: &str, proj_cols: Option<Vec<usize>>) -> Result<usize> {
    let file = std::fs::OpenOptions::new().read(true).open(path)?;
    let builder = orc_rust::ArrowReaderBuilder::try_new(file).unwrap();
    let reader = if let Some(proj_cols) = proj_cols {
        let projection = orc_rust::projection::ProjectionMask::roots(
            builder.file_metadata().root_data_type(),
            proj_cols,
        );
        builder.with_projection(projection).build()
    } else {
        builder.build()
    };
    let mut nbytes = 0;
    for batch in reader {
        nbytes += batch.unwrap().get_array_memory_size();
    }
    Ok(nbytes)
}

pub fn write_btrblocks(_batches: &[RecordBatch], _path: &str) -> Result<Vec<Vec<(usize, usize)>>> {
    panic!(
        "comment off here due to anonymous request and it is hard for us to solve the dep issue"
    );
    // const BTR_ENC_BLOCK_SIZE: usize = 64 * 1024;
    // use dictscope_bench::{
    //     column::{Column, SUPPORTED_DTYPES},
    //     compress::{BtrCompressor, Compressor},
    // };
    // use std::mem::discriminant;
    // assert!(batches.len() == 1);
    // let mut is_append = false;
    // batches[0].columns().iter().for_each(|c| {
    //     if !SUPPORTED_DTYPES.contains(&discriminant(c.data_type())) {
    //         panic!()
    //     }
    // });
    // let compressed_sizes = batches[0]
    //     .columns()
    //     .iter()
    //     .filter(|column| SUPPORTED_DTYPES.contains(&discriminant(column.data_type())))
    //     .map(|column| {
    //         let dict_column = Column::new(column);
    //         let mut compressor = BtrCompressor::new();
    //         compressor.set_disable_dict(false);
    //         dict_column
    //             .slice_with_step(BTR_ENC_BLOCK_SIZE)
    //             .map(|slice| {
    //                 let compression_stats = compressor
    //                     .compress_to_file(slice, path.to_string(), is_append)
    //                     .unwrap();
    //                 is_append = true;
    //                 (
    //                     compression_stats.compressed_size(),
    //                     compression_stats.compressed_nulls_size(),
    //                 )
    //             })
    //             .collect::<Vec<_>>()
    //     })
    //     .collect::<Vec<_>>();
    // Ok(compressed_sizes)
}

pub fn parquet_decompress_from<T: ChunkReader + 'static>(
    file: T,
    projections: Option<&[usize]>,
    selection: Option<RowSelection>,
) -> usize {
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let builder = builder.with_batch_size(65536);
    let builder = if let Some(projections) = projections {
        let file_metadata = builder.metadata().file_metadata().clone();
        let mask =
            ProjectionMask::leaves(file_metadata.schema_descr(), projections.iter().map(|&x| x));
        builder.with_projection(mask)
    } else {
        builder
    };
    let builder = if let Some(selection) = selection {
        builder.with_row_selection(selection)
    } else {
        builder
    };
    let reader = builder.build().unwrap();

    let mut nbytes = 0;
    for batch in reader {
        let batch = batch.unwrap();
        // println!("{:?}", batch);
        nbytes += batch.get_array_memory_size();
        // println!("Read batch with {} rows", batch.num_rows());
    }
    nbytes
}

pub async fn parquet_decompress_from_async(
    store: Arc<dyn ObjectStore>,
    file_name: &str,
    projections: Option<&[usize]>,
    selection: Option<RowSelection>,
) -> usize {
    let meta = store.head(&Path::from(file_name)).await.unwrap();

    let store = Arc::new(store) as Arc<dyn ObjectStore>;
    let object_reader = ParquetObjectReader::new(Arc::clone(&store), meta.clone());
    let builder = ParquetRecordBatchStreamBuilder::new(object_reader)
        .await
        .unwrap();
    let builder = builder.with_batch_size(65536);
    let builder = if let Some(projections) = projections {
        let file_metadata = builder.metadata().file_metadata().clone();
        let mask =
            ProjectionMask::leaves(file_metadata.schema_descr(), projections.iter().map(|&x| x));
        builder.with_projection(mask)
    } else {
        builder
    };
    let builder = if let Some(selection) = selection {
        builder.with_row_selection(selection)
    } else {
        builder
    };
    let mut reader = builder.build().unwrap();

    let mut nbytes = 0;
    while let Some(batch) = reader.next().await {
        let batch = batch.unwrap();
        // println!("{:?}", batch);
        nbytes += batch.get_array_memory_size();
        // println!("Read batch with {} rows", batch.num_rows());
    }
    nbytes
}

pub async fn write_vortex(
    batches: &[RecordBatch],
    vortex: tokio::fs::File,
) -> Result<tokio::fs::File> {
    let mut writer = VortexFileWriter::new(vortex);
    let compressor: &dyn CompressionStrategy = &SamplingCompressor::default();
    for batch in batches {
        let vortex_array = vortex_array::ArrayData::try_from(batch.clone()).unwrap();
        let compressed = compressor.compress(&vortex_array).unwrap();
        writer = writer.write_array_columns(compressed).await.unwrap();
    }
    let written = writer.finalize().await.unwrap();
    Ok(written)
}

pub async fn read_vortex(path: PathBuf, projections: Projection) -> Result<()> {
    let builder: VortexReadBuilder<_> = VortexReadBuilder::new(
        TokioFile::open(path).unwrap(),
        LayoutDeserializer::new(
            ALL_ENCODINGS_CONTEXT.clone(),
            LayoutContext::default().into(),
        ),
    );

    let stream = builder.with_projection(projections).build().await?;
    let vecs: Vec<vortex_array::ArrayData> = stream.try_collect().await?;
    let _arrays: Vec<ArrayRef> = vecs.into_iter().map(|v| v.into_arrow().unwrap()).collect();
    Ok(())
}

/// Rewrite the input Parquet according to rg_size, using the parquet_mine crate
/// which is my custom fork to get the correct Parquet memory size
pub fn rewrite_parquet_via_mine(
    input_path: PathBuf,
    output_path: &std::path::Path,
    rg_size: usize,
) -> Result<usize> {
    let builder = parquet_mine::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(
        std::fs::File::open(input_path).unwrap(),
    )
    .unwrap();
    let reader = builder.with_batch_size(65536).build().unwrap();
    let batches: Vec<_> = reader.collect::<Result<_, _>>().unwrap();
    let writer_properties = parquet_mine::file::properties::WriterProperties::builder()
        .set_compression(parquet_mine::basic::Compression::SNAPPY)
        .set_max_row_group_size(rg_size)
        .build();
    let mut writer = parquet_mine::arrow::arrow_writer::ArrowWriter::try_new(
        std::fs::File::create(output_path).unwrap(),
        batches[0].schema(),
        Some(writer_properties),
    )
    .unwrap();
    let mut memory_usage_sum = 0;
    let mut cnt = 0;
    for batch in batches {
        writer.write(&batch).unwrap();
        // error!("{}", writer.in_progress_size() + writer.memory_size());
        let total_size = writer.in_progress_size();
        // if total_size > max_memory_usage {
        //     max_memory_usage = total_size;
        // }
        if total_size != 0 {
            memory_usage_sum += total_size;
            cnt += 1;
        }
    }
    error!(
        "Parquet rg {} memory usage: {}",
        rg_size,
        memory_usage_sum / cnt
    );
    writer.finish().unwrap();
    let pq_size = std::fs::OpenOptions::new()
        .read(true)
        .open(output_path)
        .unwrap()
        .metadata()
        .unwrap()
        .size();
    error!(
        "Parquet size: {}, {}B",
        format_size(pq_size, DECIMAL),
        pq_size
    );
    Ok(memory_usage_sum / cnt)
}

/// Not use Vortex's function here since our data has different delim and nullstr.
pub fn write_csv_as_parquet(
    csv_path: PathBuf,
    output_path: &std::path::Path,
    delim: &str,
    nullstr: &str,
    rg_size: usize,
    is_dict_pbi: bool,
) -> Result<()> {
    error!(
        "Compressing {} to parquet",
        csv_path.as_path().to_str().unwrap()
    );
    let duckdb_cmd = if is_dict_pbi {
        format!(
            "COPY (SELECT * FROM read_csv('{}', delim = '{delim}', header = false, ignore_errors = true)) TO '{}' (COMPRESSION SNAPPY);",
            csv_path.as_path().to_str().unwrap(),
            output_path.to_str().unwrap()
        )
    } else {
        format!(
            "COPY (SELECT * FROM read_csv('{}', delim = '{delim}', header = false, nullstr = '{nullstr}')) TO '{}' (COMPRESSION SNAPPY);",
            csv_path.as_path().to_str().unwrap(),
            output_path.to_str().unwrap()
        )
    };

    Command::new("duckdb")
        .arg("-c")
        .arg(duckdb_cmd)
        .status()
        .unwrap()
        .exit_ok()
        .unwrap();
    // Rewrite with arrow-rs
    rewrite_parquet_via_mine(output_path.to_path_buf(), output_path, rg_size)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::{bench_data::parquet_into_batches, config, read_lance, read_vortex, write_vortex};

    #[tokio::test]
    #[ignore]
    async fn vortex_round_trip() {
        let f = config::get_base_data_path()
            .join("data")
            .join("parquet")
            .join("core.parquet");
        let batches = parquet_into_batches(f, Default::default()).unwrap();
        let output_path = std::env::current_dir().unwrap().join("core.vortex");
        let write = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(output_path.clone())
            .await
            .unwrap();
        write_vortex(&batches, write).await.unwrap();
        read_vortex(output_path.into(), vortex_file::Projection::All)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn lance_s3_test() {
        read_lance(
            "s3://f3-experiment/lineitem_duckdb_double.lance",
            None,
            Some(vec![100]),
            true,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn f3_s3_test() {
        let path = std::path::PathBuf::from("s3://f3-experiment/lineitem_duckdb_double.fff");
        crate::read_fff(
            path,
            crate::ReadFFFOpt {
                projections: Some(fff_poc::reader::Projection::All),
                selection: Some(fff_poc::reader::Selection::RowIndexes(vec![100])),
            },
        )
        .unwrap();
    }
}
