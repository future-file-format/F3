use std::{
    io::{Read, Seek, Write},
    mem::{discriminant, Discriminant},
    os::unix::fs::MetadataExt,
    sync::Arc,
};

use arrow::datatypes::{DataType, SchemaRef};
use arrow_array::{ArrayRef, RecordBatch};
use clap::Parser;
use fff_bench::bench_data::{BenchmarkDataset, PBIDataset};
use fff_bench::bench_data::{BenchmarkDatasets::PBI, CsvToPqOptions};
use fff_core::errors::Error;
use fff_poc::{
    options::{DictionaryTypeOptions, FileWriterOptions},
    reader::FileReaderV2Builder,
    writer::FileWriter,
};
use itertools::Itertools as _;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

lazy_static::lazy_static! {
    pub static ref SUPPORTED_DTYPES: Vec<Discriminant<DataType>> = vec![
        discriminant(&DataType::Int32),
        discriminant(&DataType::Int64),
        discriminant(&DataType::Float32),
        discriminant(&DataType::Float64),
        discriminant(&DataType::Utf8),
        discriminant(&DataType::LargeUtf8),
        // Timestamps of all time units are supported,
        // as the discriminant only cares about the enum variant (Timestamp)
        discriminant(&DataType::Timestamp(arrow::datatypes::TimeUnit::Millisecond, None)),
        discriminant(&DataType::Time32(arrow::datatypes::TimeUnit::Second)),
        discriminant(&DataType::Time64(arrow::datatypes::TimeUnit::Microsecond)),
        discriminant(&DataType::Date32),
        discriminant(&DataType::Boolean),
    ];
}

const BASELINE_SCOPE: u64 = 1 * 1024 * 1024;
const MIN_SCOPE: usize = 65536;
const LOG_INC: usize = 2;
const ALIGN_UNIT: usize = 2048;
const TIME_MEASURE_REPEAT: usize = 5;

#[derive(serde::Serialize)]
pub struct PBIRecord {
    dataset: String,
    column_id: usize,
    column_dtype: String,
    num_rows: usize,
    enc_dict_total_size: usize,
    enc_dict_net_size: usize,
    enc_dict_encoding_time: u128,
    global_total_size: usize,
    global_dict_size: usize,
    global_index_size: usize,
    global_encoding_time: u128,
    local_total_size: usize,
    local_dict_size: usize,
    local_index_size: usize,
    glbest_total_size: usize,
    glbest_dict_size: usize,
    glbest_index_size: usize,
    glbest_encoding_time: u128,
    glbest_sample_total_size: usize,
    glbest_sample_dict_size: usize,
    glbest_sample_index_size: usize,
    glbest_sample_encoding_time: u128,
    no_dict_total_size: usize,
    no_dict_net_size: usize,
    no_dict_encoding_time: u128,
    fixed_scopes: String,
    fixed_scope_total_sizes: String,
    fixed_scope_dict_sizes: String,
    fixed_scope_index_sizes: String,
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    output_file: String,
}

const ENCODING_UNIT: usize = 65536;
const SAMPLE_RATIO: f64 = 0.01;

pub struct ParquetReader<R: Read + Seek + parquet::file::reader::ChunkReader> {
    file_reader: R,
}

impl<R: Read + Seek + parquet::file::reader::ChunkReader + 'static> ParquetReader<R> {
    pub fn new(file_reader: R) -> Self {
        ParquetReader { file_reader }
    }

    /// Consumes the reader and reads the Parquet file all at once
    pub fn read(self) -> Result<arrow_array::RecordBatch, Error> {
        let builder = ParquetRecordBatchReaderBuilder::try_new(self.file_reader)
            .map_err(|e| Error::External(Box::new(e)))?;
        let schema = builder.schema().to_owned();
        let reader = builder.build().map_err(|e| Error::External(Box::new(e)))?;
        let batches = reader.collect::<Result<Vec<_>, _>>()?;
        Ok(arrow::compute::concat_batches(&schema, &batches)?)
    }
}

fn comp_with_dict_option(
    schema: SchemaRef,
    column: ArrayRef,
    dict_opt: DictionaryTypeOptions,
    measure_time: usize,
) -> Result<(usize, usize, usize, u128), Error> {
    let batch = RecordBatch::try_new(schema.clone(), vec![column])?;
    let num_rows = batch.num_rows();
    let compress = || -> Result<(Arc<std::fs::File>, Vec<fff_poc::counter::EncodingCounter>, u128), Error> {
        let temp_file = Arc::new(tempfile::tempfile()?);
        let file_cloned = temp_file.clone();
        let start = std::time::Instant::now();
        let options = FileWriterOptions::builder()
            .set_dictionary_type(dict_opt)
            .build();
        let mut fff_writer = FileWriter::try_new(schema.clone(), temp_file, options)?;
        for offset in (0..num_rows).step_by(ENCODING_UNIT) {
            let end = std::cmp::min(offset + ENCODING_UNIT, num_rows);
            fff_writer.write_batch(&batch.slice(offset, end - offset))?;
        }
        let counters = fff_writer.finish()?;
        Ok((file_cloned, counters, start.elapsed().as_nanos()))
    };
    let (file_cloned, counters, mut _total_time) = compress()?;
    assert_eq!(counters.len(), 1);
    let counter = counters
        .first()
        .ok_or(Error::General("Cannot find the counter".to_owned()))?;
    let written_size = file_cloned.metadata()?.size() as usize;
    let mut reader = FileReaderV2Builder::new(file_cloned).build()?;
    let (shared_counters, _) = reader.get_shared_dict_sizes().unwrap();
    assert_eq!(shared_counters.len(), 1);
    let mut shared_counter = shared_counters
        .first()
        .ok_or(Error::General(
            "Cannot find the shared dict counter".to_owned(),
        ))?
        .clone();
    shared_counter.add(counter);
    let mut total_time = 0;
    if measure_time > 0 {
        for _ in 0..measure_time {
            let start = std::time::Instant::now();
            let _ = compress()?;
            total_time += start.elapsed().as_nanos();
        }
    }
    Ok((
        written_size,
        shared_counter.dict_size,
        shared_counter.index_size,
        total_time,
    ))
}

fn comp_fixed_scopes(schema: SchemaRef, column: ArrayRef) -> (String, String, String, String) {
    let num_rows = column.len();
    let log_num_rows = ((num_rows as f64).log2() * (LOG_INC as f64)) as usize;
    let log_min_scope = ((MIN_SCOPE as f64).log2() * (LOG_INC as f64)) as usize;
    let sample_points = (log_min_scope..=log_num_rows)
        .map(|i| {
            (2.0f64.powf(i as f64 / LOG_INC as f64) / (ALIGN_UNIT as f64)).round() as usize
                * ALIGN_UNIT
        })
        .filter(|&x| x <= num_rows)
        .chain(std::iter::once(num_rows))
        .dedup()
        .collect::<Vec<_>>();
    let (total_sizes, dict_sizes, index_sizes, _): (Vec<_>, Vec<_>, Vec<_>, Vec<_>) = sample_points
        .iter()
        .map(|scope| {
            comp_with_dict_option(
                schema.clone(),
                column.clone(),
                DictionaryTypeOptions::FixedScopeDictionary(*scope as u64),
                0,
            )
            .unwrap()
        })
        .multiunzip();

    (
        format!("{:?}", sample_points),
        format!("{:?}", total_sizes),
        format!("{:?}", dict_sizes),
        format!("{:?}", index_sizes),
    )
}

fn run_one_dataset(dataset: impl BenchmarkDataset) -> Vec<PBIRecord> {
    let paths = dataset.list_files(fff_bench::bench_data::FileType::Parquet);
    let path = paths.get(0).unwrap();

    let file_reader = std::fs::File::open(path.clone()).unwrap();
    let reader = ParquetReader::new(file_reader);
    let array = reader.read().unwrap();
    let num_rows = array.num_rows();
    let dataset_name = dataset_shortname(path.to_str().unwrap());
    let schema = array.schema();
    array
        .columns()
        .iter()
        .filter(|column| {
            SUPPORTED_DTYPES.contains(&discriminant(column.data_type())) || {
                println!("Unsupported data type: {:?}", column.data_type());
                false
            }
        })
        .enumerate()
        .map(|(column_id, column)| {
            let column_dtype = column.data_type().to_string();
            let column_schema = Arc::new(schema.project(&[column_id]).unwrap());

            let (enc_dict_total_size, _, enc_dict_net_size, enc_dict_encoding_time) =
                comp_with_dict_option(
                    column_schema.clone(),
                    column.clone(),
                    DictionaryTypeOptions::EncoderDictionary,
                    TIME_MEASURE_REPEAT,
                )
                .unwrap();
            let (no_dict_total_size, _, no_dict_net_size, no_dict_encoding_time) =
                comp_with_dict_option(
                    column_schema.clone(),
                    column.clone(),
                    DictionaryTypeOptions::NoDictionary,
                    TIME_MEASURE_REPEAT,
                )
                .unwrap();
            let (global_total_size, global_dict_size, global_index_size, global_encoding_time) =
                comp_with_dict_option(
                    column_schema.clone(),
                    column.clone(),
                    DictionaryTypeOptions::GlobalDictionary,
                    TIME_MEASURE_REPEAT,
                )
                .unwrap();
            let (local_total_size, local_dict_size, local_index_size, _local_encoding_time) =
                comp_with_dict_option(
                    column_schema.clone(),
                    column.clone(),
                    DictionaryTypeOptions::LocalDictionary,
                    TIME_MEASURE_REPEAT,
                )
                .unwrap();
            let (
                baseline_total_size,
                baseline_dict_size,
                baseline_index_size,
                _baseline_encoding_time,
            ) = (0, 0, 0, 0);
            let (glbest_total_size, glbest_dict_size, glbest_index_size, glbest_encoding_time) =
                comp_with_dict_option(
                    column_schema.clone(),
                    column.clone(),
                    DictionaryTypeOptions::GLBest(None),
                    TIME_MEASURE_REPEAT,
                )
                .unwrap();
            let (
                glbest_sample_total_size,
                glbest_sample_dict_size,
                glbest_sample_index_size,
                glbest_sample_encoding_time,
            ) = comp_with_dict_option(
                column_schema.clone(),
                column.clone(),
                DictionaryTypeOptions::GLBest(Some((SAMPLE_RATIO, ENCODING_UNIT))),
                TIME_MEASURE_REPEAT,
            )
            .unwrap();
            let (
                fixed_scopes,
                fixed_scope_total_sizes,
                fixed_scope_dict_sizes,
                fixed_scope_index_sizes,
            ) = (
                "[]".to_string(),
                "[]".to_string(),
                "[]".to_string(),
                "[]".to_string(),
            );
            PBIRecord {
                dataset: dataset_name.clone(),
                num_rows,
                column_id,
                column_dtype,
                enc_dict_total_size,
                enc_dict_net_size,
                enc_dict_encoding_time,
                global_total_size,
                global_dict_size,
                global_index_size,
                global_encoding_time,
                local_total_size,
                local_dict_size,
                local_index_size,
                glbest_total_size,
                glbest_dict_size,
                glbest_index_size,
                glbest_encoding_time,
                glbest_sample_total_size,
                glbest_sample_dict_size,
                glbest_sample_index_size,
                glbest_sample_encoding_time,
                no_dict_total_size,
                no_dict_net_size,
                no_dict_encoding_time,
                fixed_scopes,
                fixed_scope_total_sizes,
                fixed_scope_dict_sizes,
                fixed_scope_index_sizes,
            }
        })
        .collect()
}

fn main() {
    let args = Args::parse();

    let header = vec![
        "dataset",
        "column_id",
        "column_dtype",
        "num_rows",
        "enc_dict_total_size",
        "enc_dict_net_size",
        "enc_dict_encoding_time",
        "global_total_size",
        "global_dict_size",
        "global_index_size",
        "global_encoding_time",
        "local_total_size",
        "local_dict_size",
        "local_index_size",
        "glbest_total_size",
        "glbest_dict_size",
        "glbest_index_size",
        "glbest_encoding_time",
        "glbest_sample_total_size",
        "glbest_sample_dict_size",
        "glbest_sample_index_size",
        "glbest_sample_encoding_time",
        "no_dict_total_size",
        "no_dict_net_size",
        "no_dict_encoding_time",
        "fixed_scopes",
        "fixed_scope_total_sizes",
        "fixed_scope_dict_sizes",
        "fixed_scope_index_sizes",
    ];

    std::fs::File::create_new(args.output_file.clone())
        .map(|mut f| {
            let mut header_line = header.join(",");
            header_line.push('\n');
            f.write_all(header_line.as_bytes()).unwrap()
        })
        .unwrap_or_default();
    let file_wtr = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(args.output_file)
        .unwrap();
    let mut wtr = csv::WriterBuilder::new()
        .has_headers(false)
        .from_writer(file_wtr);
    (0..=46)
        // Filter out AirlineSentiment, CityMaxCapita, IUBLibrary, MedPayment2, which would cause errors
        .filter(|i| *i != 0 && *i != 4 && *i != 15 && *i != 18)
        // .filter(|i| *i > 33)
        .for_each(|i| {
            let dataset = PBIDataset::try_from(i).unwrap();
            let datasets = PBI(dataset);
            let mut write_to_parquet_opt = CsvToPqOptions::default();
            write_to_parquet_opt.is_dict_scope = true;
            datasets.write_as_parquet(write_to_parquet_opt);
            let mut dataset_name = "".to_owned();
            run_one_dataset(datasets).into_iter().for_each(|record| {
                dataset_name = record.dataset.clone();
                wtr.serialize(record).unwrap();
                wtr.flush().unwrap();
            });
            eprintln!("[{}/{}] Dataset {} processed", i + 1, 47, dataset_name);
        });
}

fn dataset_shortname(input_path: &str) -> String {
    input_path
        .split('/')
        .last()
        .unwrap()
        .split('.')
        .next()
        .unwrap()
        .to_string()
}
