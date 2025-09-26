/// This file demonstrates the benefit of using custom encoding with PCO and sliding window encoding.
/// You need to compile the .so files of `wasm-libs/fff-ude-example-custom-encoder` and
/// `wasm-libs/fff-ude-example-pco-real-encoder` first.
///
/// Run the following command to compile the .so files.
/// ```bash
/// cd wasm-libs/fff-ude-example-custom-encoder
/// cargo build --release
/// cd wasm-libs/fff-ude-example-pco-real-encoder
/// cargo build --release
/// ```
///
/// Then build the wasm files of pco and custom encoding.
/// ```bash
/// cd wasm-libs/fff-ude-example-pco-real-encoder
/// cargo build --profile opt-size-lvl3
/// cd wasm-libs/fff-ude-example-custom
/// cargo build --profile opt-size-lvl3
/// ```
///
/// The original two Parquet files can be found at `https://r2.xinyuzeng.xyz/linear.parquet` and
/// `https://r2.xinyuzeng.xyz/synthetic.parquet`.
///
use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Instant;

use anyhow::Result;
use arrow::datatypes::DataType;
use bench_vortex::setup_logger;
use fff_bench::bench_data::parquet_into_batches;
use fff_bench::{read_fff, write_fff};
use fff_poc::context::{WASMId, WasmLib};
use fff_poc::options::{CustomEncodingOptions, FileWriterOptions};
use log::{error, LevelFilter};

static FFF_BENCH_PATH: LazyLock<PathBuf> =
    LazyLock::new(|| PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()));
static PROJ_ROOT: LazyLock<PathBuf> =
    LazyLock::new(|| FFF_BENCH_PATH.parent().unwrap().to_path_buf());

fn main() -> Result<()> {
    setup_logger(LevelFilter::Error);
    // let input_parquet = "linear.parquet";
    let input_parquet = "synthetic.parquet";
    let name = input_parquet.strip_suffix(".parquet").unwrap();
    // let fff_name = format!("{name}.fff.pco");
    // let fff_name = format!("{name}.fff.custom");
    let fff_name = format!("{name}.fff");
    {
        let fff = File::create(fff_name.clone())?;
        let start = Instant::now();
        let batches = parquet_into_batches(input_parquet.into(), Default::default())?;
        error!("Parquet Elapsed: {:?}", start.elapsed());
        // println!("{:?}", batches[0].schema());
        let wasms = HashMap::from([(
            WASMId(0),
            WasmLib::new(
                // PROJ_ROOT.join("target/release/libfff_ude_example_pco_real_encoder.so"),
                // std::fs::read(PROJ_ROOT.join("target/wasm32-wasip1/opt-size-lvl3/fff_ude_example_pco_real.wasm")).unwrap().into(),
                PROJ_ROOT.join("target/release/libfff_ude_example_custom_encoder.so"),
                std::fs::read(
                    PROJ_ROOT
                        .join("target/wasm32-wasip1/opt-size-lvl3/fff_ude_example_custom.wasm"),
                )
                .unwrap()
                .into(),
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
        data_type_to_wasm_id.insert(
            DataType::Timestamp(arrow::datatypes::TimeUnit::Nanosecond, Some("UTC".into())),
            WASMId(0),
        );
        data_type_to_wasm_id.insert(
            DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
            WASMId(0),
        );
        let custom_encoding_options = CustomEncodingOptions::new(wasms, data_type_to_wasm_id);
        write_fff(
            &batches,
            &fff,
            FileWriterOptions::builder()
                .set_custom_encoding_options(custom_encoding_options)
                .build(),
        )?;
    }
    {
        let start = Instant::now();
        let _batches = read_fff(fff_name.into(), Default::default())?;
        // println!("{:?}", batches[0].schema());
        error!("Elapsed: {:?}", start.elapsed());
    }
    Ok(())
}
