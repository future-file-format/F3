// Code initially vendored from vortex-bench. Modified for our usage.
use core::fmt::Display;
use core::panic;
use std::collections::HashMap;
use std::fs::{self, File};
use std::future::Future;
use std::io::BufWriter;
use std::process::Command;
use std::sync::{Arc, LazyLock};

use crate::bench_data::CFBDataset::*;
use crate::{write_btrblocks, IdempotentPath, ReadFFFOpt};
use anyhow::{bail, Ok, Result};
use arrow_array::RecordBatch;
// use dictscope_bench::compress::Compressor;
use fff_poc::context::WASMId;
use fff_poc::options::{FileWriterOptions, FileWriterOptionsBuilder};
use fff_poc::reader::get_max_chunk_size;
use fff_ude_wasm::Runtime;
use humansize::{format_size, DECIMAL};
use itertools::Itertools;
use lance_file::v2::writer::FileWriterOptions as LanceFileWriterOptions;
use lance_file::version::LanceFileVersion;
use log::{error, info};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Instant;
use strum::IntoEnumIterator;
use strum_macros::EnumIter;
use vortex_array::IntoCanonical;

/// FIXME: hacky code to print CR when testing CR, but not print to avoid Arrow read time when testing decomp time.
const PRINT_CR: bool = false;

use crate::{
    bench_data::PBIDataset::*, read_fff, read_fff_aot_wasm, read_lance, read_orc, read_vortex,
    write_csv_as_parquet, write_fff, write_lance, write_orc, write_vortex,
};
use bench_vortex::data_downloads::{decompress_bz2, download_data};
use bench_vortex::reader::{compress_parquet_to_vortex, take_parquet, take_vortex_tokio};
use bench_vortex::{idempotent, idempotent_async};
use fs_extra::dir::get_size;
use std::os::unix::fs::MetadataExt;

pub struct PqToBatchesOptions {
    batch_size: usize,
}

impl Default for PqToBatchesOptions {
    fn default() -> Self {
        Self { batch_size: 65536 }
    }
}

impl PqToBatchesOptions {
    pub fn with_batch_size(batch_size: usize) -> Self {
        Self { batch_size }
    }
}

pub fn parquet_into_batches(f: PathBuf, opt: PqToBatchesOptions) -> Result<Vec<RecordBatch>> {
    let pq = File::open(f)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(pq)?;
    let reader = builder.with_batch_size(opt.batch_size).build()?;
    Ok(reader
        .into_iter()
        .map(|batch_result| batch_result.unwrap())
        .collect::<Vec<_>>())
}

#[derive(Copy, Clone)]
pub enum FileType {
    Csv,
    Parquet,
    Vortex,
    FFF,
    FFFWasm,
    FFFRa,
    Lance,
    Orc,
    BtrBlocks,
}

impl Display for FileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileType::Csv => write!(f, "csv"),
            FileType::Parquet => write!(f, "parquet"),
            FileType::Vortex => write!(f, "vortex"),
            FileType::Lance => write!(f, "lance"),
            FileType::FFF => write!(f, "fff"),
            FileType::FFFWasm => write!(f, "fffwasm"),
            FileType::FFFRa => write!(f, "fffra"),
            FileType::Orc => write!(f, "orc"),
            FileType::BtrBlocks => write!(f, "btr"),
        }
    }
}

#[derive(EnumIter, Debug, PartialEq)]
pub enum CFBDataset {
    Core,
    Core64M,
    Bi,
    Classic,
    Geo,
    Log,
    Ml,
}

impl Display for CFBDataset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Core => write!(f, "core"),
            Core64M => write!(f, "core_64M"),
            Bi => write!(f, "bi"),
            Classic => write!(f, "classic"),
            Geo => write!(f, "geo"),
            Log => write!(f, "log"),
            Ml => write!(f, "ml"),
        }
    }
}

impl CFBDataset {
    fn get_url(&self) -> Url {
        static PREFIX: &str = "https://cfb-data.xinyuzeng.xyz/";
        fn join_with_prefix(wl: &str) -> Url {
            Url::parse(PREFIX)
                .unwrap()
                .join(format!("{}_r1000000_c20/gen_data/{}_r1000000_c20.csv", wl, wl).as_str())
                .unwrap()
        }
        join_with_prefix(format!("{}", self).as_str())
    }

    pub fn download_all() {
        for dataset in CFBDataset::iter() {
            download_data(
                dataset.get_file_path(FileType::Csv),
                CFBDataset::get_url(&dataset).as_str(),
            );
        }
    }

    fn list_files(&self, file_type: FileType) -> Vec<PathBuf> {
        vec![self.get_file_path(file_type)]
    }

    // pub fn directory_location() -> PathBuf {
    //     Path::new("/mnt/nvme0n1/xinyu/data").to_path_buf()
    //     // Path::new(env!("CARGO_MANIFEST_DIR")).join("data")
    // }

    fn get_file_path(&self, file_type: FileType) -> PathBuf {
        let extension = format!("{file_type}");

        "data"
            .to_data_path()
            .join(extension.as_str())
            .join(format!("{}.csv", self).as_str())
            .with_extension(extension.as_str())
    }
}

#[derive(Copy, Clone, EnumIter, Debug)]
pub enum LaionDataset {
    Data0001,
    // Merge64M,
    Merge8M,
}

impl Display for LaionDataset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaionDataset::Data0001 => write!(f, "0001"),
            // LaionDataset::Merge64M => write!(f, "merged_64M"),
            LaionDataset::Merge8M => write!(f, "merged_8M"),
        }
    }
}

impl LaionDataset {
    fn get_url(&self) -> Url {
        static PREFIX: &str = "https://datasets-documentation.s3.eu-west-3.amazonaws.com/laion/";
        fn join_with_prefix(num: &str) -> Url {
            Url::parse(PREFIX)
                .unwrap()
                .join(format!("{}.parquet", num).as_str())
                .unwrap()
        }
        join_with_prefix(format!("{}", self).as_str())
    }

    pub fn download_all() {
        for dataset in LaionDataset::iter() {
            download_data(
                dataset.get_file_path(FileType::Parquet),
                LaionDataset::get_url(&dataset).as_str(),
            );
        }
    }

    fn list_files(&self, file_type: FileType) -> Vec<PathBuf> {
        vec![self.get_file_path(file_type)]
    }

    // pub fn directory_location() -> PathBuf {
    //     Path::new("/mnt/nvme0n1/xinyu/data").to_path_buf()
    //     // Path::new(env!("CARGO_MANIFEST_DIR")).join("data")
    // }

    fn get_file_path(&self, file_type: FileType) -> PathBuf {
        let extension = format!("{file_type}");

        "laion"
            .to_data_path()
            .join(extension.as_str())
            .join(format!("{}.csv", self).as_str())
            .with_extension(extension.as_str())
    }
}

impl TryFrom<u8> for LaionDataset {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => LaionDataset::Data0001,
            _ => panic!(),
        })
    }
}

#[derive(Copy, Clone, EnumIter, Debug)]
pub enum ClickBenchDataset {
    Hits,
}

impl Display for ClickBenchDataset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClickBenchDataset::Hits => write!(f, "hits_8M"),
        }
    }
}

impl ClickBenchDataset {
    pub fn download_all() {
        // panic!("I manullay generate the file using scripts/tpch_dbgen.py");
    }

    fn list_files(&self, file_type: FileType) -> Vec<PathBuf> {
        vec![self.get_file_path(file_type)]
    }

    fn get_file_path(&self, file_type: FileType) -> PathBuf {
        let extension = format!("{file_type}");

        "clickbench"
            .to_data_path()
            .join(extension.as_str())
            .join(format!("{}.csv", self).as_str())
            .with_extension(extension.as_str())
    }
}

#[derive(Copy, Clone, EnumIter, Debug)]
pub enum TPCHDataset {
    Lineitem,
}

impl Display for TPCHDataset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TPCHDataset::Lineitem => write!(f, "lineitem_duckdb_double"),
        }
    }
}

impl TPCHDataset {
    pub fn download_all() {
        // panic!("I manullay generate the file using scripts/tpch_dbgen.py");
    }

    fn list_files(&self, file_type: FileType) -> Vec<PathBuf> {
        vec![self.get_file_path(file_type)]
    }

    fn get_file_path(&self, file_type: FileType) -> PathBuf {
        let extension = format!("{file_type}");

        "tpch"
            .to_data_path()
            .join(extension.as_str())
            .join(
                match file_type {
                    // Use the random access preferred version of fff.
                    FileType::FFF => format!("{}_ra_64kEnc.csv", self),
                    _ => format!("{}.csv", self),
                }
                .as_str(),
            )
            .with_extension(extension.as_str())
    }
}

// NB: we do not expect this to change, otherwise we'd crawl the site and populate it at runtime
// We will eventually switch over to self-hosting this data, at which time this map will need
// to be updated once.
static URLS: LazyLock<HashMap<PBIDataset, Vec<PBIUrl>>> = LazyLock::new(|| {
    HashMap::from([
        (
            AirlineSentiment,
            vec![PBIUrl::new(
                "AirlineSentiment",
                "AirlineSentiment_1.csv.bz2",
            )],
        ),
        (Arade, vec![PBIUrl::new("Arade", "Arade_1.csv.bz2")]),
        (Bimbo, vec![PBIUrl::new("Bimbo", "Bimbo_1.csv.bz2")]),
        (
            CMSprovider,
            vec![
                PBIUrl::new("CMSprovider", "CMSprovider_1.csv.bz2"),
                PBIUrl::new("CMSprovider", "CMSprovider_2.csv.bz2"),
            ],
        ),
        (
            CityMaxCapita,
            vec![PBIUrl::new("CityMaxCapita", "CityMaxCapita_1.csv.bz2")],
        ),
        (
            CommonGovernment,
            vec![
                PBIUrl::new("CommonGovernment", "CommonGovernment_1.csv.bz2"),
                PBIUrl::new("CommonGovernment", "CommonGovernment_2.csv.bz2"),
                PBIUrl::new("CommonGovernment", "CommonGovernment_3.csv.bz2"),
                PBIUrl::new("CommonGovernment", "CommonGovernment_4.csv.bz2"),
                PBIUrl::new("CommonGovernment", "CommonGovernment_5.csv.bz2"),
                PBIUrl::new("CommonGovernment", "CommonGovernment_6.csv.bz2"),
                PBIUrl::new("CommonGovernment", "CommonGovernment_7.csv.bz2"),
                PBIUrl::new("CommonGovernment", "CommonGovernment_8.csv.bz2"),
                PBIUrl::new("CommonGovernment", "CommonGovernment_9.csv.bz2"),
                PBIUrl::new("CommonGovernment", "CommonGovernment_10.csv.bz2"),
                PBIUrl::new("CommonGovernment", "CommonGovernment_11.csv.bz2"),
                PBIUrl::new("CommonGovernment", "CommonGovernment_12.csv.bz2"),
                PBIUrl::new("CommonGovernment", "CommonGovernment_13.csv.bz2"),
            ],
        ),
        (
            Corporations,
            vec![PBIUrl::new("Corporations", "Corporations_1.csv.bz2")],
        ),
        (Eixo, vec![PBIUrl::new("Eixo", "Eixo_1.csv.bz2")]),
        (
            Euro2016,
            vec![PBIUrl::new("Euro2016", "Euro2016_1.csv.bz2")],
        ),
        (Food, vec![PBIUrl::new("Food", "Food_1.csv.bz2")]),
        (
            Generico,
            vec![
                PBIUrl::new("Generico", "Generico_1.csv.bz2"),
                PBIUrl::new("Generico", "Generico_2.csv.bz2"),
                PBIUrl::new("Generico", "Generico_3.csv.bz2"),
                PBIUrl::new("Generico", "Generico_4.csv.bz2"),
                PBIUrl::new("Generico", "Generico_5.csv.bz2"),
            ],
        ),
        (
            HashTags,
            vec![PBIUrl::new("HashTags", "HashTags_1.csv.bz2")],
        ),
        (Hatred, vec![PBIUrl::new("Hatred", "Hatred_1.csv.bz2")]),
        (
            IGlocations1,
            vec![PBIUrl::new("IGlocations1", "IGlocations1_1.csv.bz2")],
        ),
        (
            IGlocations2,
            vec![
                PBIUrl::new("IGlocations2", "IGlocations2_1.csv.bz2"),
                PBIUrl::new("IGlocations2", "IGlocations2_2.csv.bz2"),
            ],
        ),
        (
            IUBLibrary,
            vec![PBIUrl::new("IUBLibrary", "IUBLibrary_1.csv.bz2")],
        ),
        (
            MLB,
            vec![
                PBIUrl::new("MLB", "MLB_1.csv.bz2"),
                PBIUrl::new("MLB", "MLB_2.csv.bz2"),
                PBIUrl::new("MLB", "MLB_3.csv.bz2"),
                PBIUrl::new("MLB", "MLB_4.csv.bz2"),
                PBIUrl::new("MLB", "MLB_5.csv.bz2"),
                PBIUrl::new("MLB", "MLB_6.csv.bz2"),
                PBIUrl::new("MLB", "MLB_7.csv.bz2"),
                PBIUrl::new("MLB", "MLB_8.csv.bz2"),
                PBIUrl::new("MLB", "MLB_9.csv.bz2"),
                PBIUrl::new("MLB", "MLB_10.csv.bz2"),
                PBIUrl::new("MLB", "MLB_11.csv.bz2"),
                PBIUrl::new("MLB", "MLB_12.csv.bz2"),
                PBIUrl::new("MLB", "MLB_13.csv.bz2"),
                PBIUrl::new("MLB", "MLB_14.csv.bz2"),
                PBIUrl::new("MLB", "MLB_15.csv.bz2"),
                PBIUrl::new("MLB", "MLB_16.csv.bz2"),
                PBIUrl::new("MLB", "MLB_17.csv.bz2"),
                PBIUrl::new("MLB", "MLB_18.csv.bz2"),
                PBIUrl::new("MLB", "MLB_19.csv.bz2"),
                PBIUrl::new("MLB", "MLB_20.csv.bz2"),
                PBIUrl::new("MLB", "MLB_21.csv.bz2"),
                PBIUrl::new("MLB", "MLB_22.csv.bz2"),
                PBIUrl::new("MLB", "MLB_23.csv.bz2"),
                PBIUrl::new("MLB", "MLB_24.csv.bz2"),
                PBIUrl::new("MLB", "MLB_25.csv.bz2"),
                PBIUrl::new("MLB", "MLB_26.csv.bz2"),
                PBIUrl::new("MLB", "MLB_27.csv.bz2"),
                PBIUrl::new("MLB", "MLB_28.csv.bz2"),
                PBIUrl::new("MLB", "MLB_29.csv.bz2"),
                PBIUrl::new("MLB", "MLB_30.csv.bz2"),
                PBIUrl::new("MLB", "MLB_31.csv.bz2"),
                PBIUrl::new("MLB", "MLB_32.csv.bz2"),
                PBIUrl::new("MLB", "MLB_33.csv.bz2"),
                PBIUrl::new("MLB", "MLB_34.csv.bz2"),
                PBIUrl::new("MLB", "MLB_35.csv.bz2"),
                PBIUrl::new("MLB", "MLB_36.csv.bz2"),
                PBIUrl::new("MLB", "MLB_37.csv.bz2"),
                PBIUrl::new("MLB", "MLB_38.csv.bz2"),
                PBIUrl::new("MLB", "MLB_39.csv.bz2"),
                PBIUrl::new("MLB", "MLB_40.csv.bz2"),
                PBIUrl::new("MLB", "MLB_41.csv.bz2"),
                PBIUrl::new("MLB", "MLB_42.csv.bz2"),
                PBIUrl::new("MLB", "MLB_43.csv.bz2"),
                PBIUrl::new("MLB", "MLB_44.csv.bz2"),
                PBIUrl::new("MLB", "MLB_45.csv.bz2"),
                PBIUrl::new("MLB", "MLB_46.csv.bz2"),
                PBIUrl::new("MLB", "MLB_47.csv.bz2"),
                PBIUrl::new("MLB", "MLB_48.csv.bz2"),
                PBIUrl::new("MLB", "MLB_49.csv.bz2"),
                PBIUrl::new("MLB", "MLB_50.csv.bz2"),
                PBIUrl::new("MLB", "MLB_51.csv.bz2"),
                PBIUrl::new("MLB", "MLB_52.csv.bz2"),
                PBIUrl::new("MLB", "MLB_53.csv.bz2"),
                PBIUrl::new("MLB", "MLB_54.csv.bz2"),
                PBIUrl::new("MLB", "MLB_55.csv.bz2"),
                PBIUrl::new("MLB", "MLB_56.csv.bz2"),
                PBIUrl::new("MLB", "MLB_57.csv.bz2"),
                PBIUrl::new("MLB", "MLB_58.csv.bz2"),
                PBIUrl::new("MLB", "MLB_59.csv.bz2"),
                PBIUrl::new("MLB", "MLB_60.csv.bz2"),
                PBIUrl::new("MLB", "MLB_61.csv.bz2"),
                PBIUrl::new("MLB", "MLB_62.csv.bz2"),
                PBIUrl::new("MLB", "MLB_63.csv.bz2"),
                PBIUrl::new("MLB", "MLB_64.csv.bz2"),
                PBIUrl::new("MLB", "MLB_65.csv.bz2"),
                PBIUrl::new("MLB", "MLB_66.csv.bz2"),
                PBIUrl::new("MLB", "MLB_67.csv.bz2"),
                PBIUrl::new("MLB", "MLB_68.csv.bz2"),
            ],
        ),
        (
            MedPayment1,
            vec![PBIUrl::new("MedPayment1", "MedPayment1_1.csv.bz2")],
        ),
        (
            MedPayment2,
            vec![PBIUrl::new("MedPayment2", "MedPayment2_1.csv.bz2")],
        ),
        (
            Medicare1,
            vec![
                PBIUrl::new("Medicare1", "Medicare1_1.csv.bz2"),
                PBIUrl::new("Medicare1", "Medicare1_2.csv.bz2"),
            ],
        ),
        (
            Medicare2,
            vec![
                PBIUrl::new("Medicare2", "Medicare2_1.csv.bz2"),
                PBIUrl::new("Medicare2", "Medicare2_2.csv.bz2"),
            ],
        ),
        (
            Medicare3,
            vec![PBIUrl::new("Medicare3", "Medicare3_1.csv.bz2")],
        ),
        (
            Motos,
            vec![
                PBIUrl::new("Motos", "Motos_1.csv.bz2"),
                PBIUrl::new("Motos", "Motos_2.csv.bz2"),
            ],
        ),
        (
            MulheresMil,
            vec![PBIUrl::new("MulheresMil", "MulheresMil_1.csv.bz2")],
        ),
        (
            NYC,
            vec![
                PBIUrl::new("NYC", "NYC_1.csv.bz2"),
                PBIUrl::new("NYC", "NYC_2.csv.bz2"),
            ],
        ),
        (
            PanCreactomy1,
            vec![PBIUrl::new("PanCreactomy1", "PanCreactomy1_1.csv.bz2")],
        ),
        (
            PanCreactomy2,
            vec![
                PBIUrl::new("PanCreactomy2", "PanCreactomy2_1.csv.bz2"),
                PBIUrl::new("PanCreactomy2", "PanCreactomy2_2.csv.bz2"),
            ],
        ),
        (
            Physicians,
            vec![PBIUrl::new("Physicians", "Physicians_1.csv.bz2")],
        ),
        (
            Provider,
            vec![
                PBIUrl::new("Provider", "Provider_1.csv.bz2"),
                PBIUrl::new("Provider", "Provider_2.csv.bz2"),
                PBIUrl::new("Provider", "Provider_3.csv.bz2"),
                PBIUrl::new("Provider", "Provider_4.csv.bz2"),
                PBIUrl::new("Provider", "Provider_5.csv.bz2"),
                PBIUrl::new("Provider", "Provider_6.csv.bz2"),
                PBIUrl::new("Provider", "Provider_7.csv.bz2"),
                PBIUrl::new("Provider", "Provider_8.csv.bz2"),
            ],
        ),
        (
            RealEstate1,
            vec![
                PBIUrl::new("RealEstate1", "RealEstate1_1.csv.bz2"),
                PBIUrl::new("RealEstate1", "RealEstate1_2.csv.bz2"),
            ],
        ),
        (
            RealEstate2,
            vec![
                PBIUrl::new("RealEstate2", "RealEstate2_1.csv.bz2"),
                PBIUrl::new("RealEstate2", "RealEstate2_2.csv.bz2"),
                PBIUrl::new("RealEstate2", "RealEstate2_3.csv.bz2"),
                PBIUrl::new("RealEstate2", "RealEstate2_4.csv.bz2"),
                PBIUrl::new("RealEstate2", "RealEstate2_5.csv.bz2"),
                PBIUrl::new("RealEstate2", "RealEstate2_6.csv.bz2"),
                PBIUrl::new("RealEstate2", "RealEstate2_7.csv.bz2"),
            ],
        ),
        (
            Redfin1,
            vec![
                PBIUrl::new("Redfin1", "Redfin1_1.csv.bz2"),
                PBIUrl::new("Redfin1", "Redfin1_2.csv.bz2"),
                PBIUrl::new("Redfin1", "Redfin1_3.csv.bz2"),
                PBIUrl::new("Redfin1", "Redfin1_4.csv.bz2"),
            ],
        ),
        (
            Redfin2,
            vec![
                PBIUrl::new("Redfin2", "Redfin2_1.csv.bz2"),
                PBIUrl::new("Redfin2", "Redfin2_2.csv.bz2"),
                PBIUrl::new("Redfin2", "Redfin2_3.csv.bz2"),
            ],
        ),
        (
            Redfin3,
            vec![
                PBIUrl::new("Redfin3", "Redfin3_1.csv.bz2"),
                PBIUrl::new("Redfin3", "Redfin3_2.csv.bz2"),
            ],
        ),
        (Redfin4, vec![PBIUrl::new("Redfin4", "Redfin4_1.csv.bz2")]),
        (
            Rentabilidad,
            vec![
                PBIUrl::new("Rentabilidad", "Rentabilidad_1.csv.bz2"),
                PBIUrl::new("Rentabilidad", "Rentabilidad_2.csv.bz2"),
                PBIUrl::new("Rentabilidad", "Rentabilidad_3.csv.bz2"),
                PBIUrl::new("Rentabilidad", "Rentabilidad_4.csv.bz2"),
                PBIUrl::new("Rentabilidad", "Rentabilidad_5.csv.bz2"),
                PBIUrl::new("Rentabilidad", "Rentabilidad_6.csv.bz2"),
                PBIUrl::new("Rentabilidad", "Rentabilidad_7.csv.bz2"),
                PBIUrl::new("Rentabilidad", "Rentabilidad_8.csv.bz2"),
                PBIUrl::new("Rentabilidad", "Rentabilidad_9.csv.bz2"),
            ],
        ),
        (
            Romance,
            vec![
                PBIUrl::new("Romance", "Romance_1.csv.bz2"),
                PBIUrl::new("Romance", "Romance_2.csv.bz2"),
            ],
        ),
        (
            SalariesFrance,
            vec![
                PBIUrl::new("SalariesFrance", "SalariesFrance_1.csv.bz2"),
                PBIUrl::new("SalariesFrance", "SalariesFrance_2.csv.bz2"),
                PBIUrl::new("SalariesFrance", "SalariesFrance_3.csv.bz2"),
                PBIUrl::new("SalariesFrance", "SalariesFrance_4.csv.bz2"),
                PBIUrl::new("SalariesFrance", "SalariesFrance_5.csv.bz2"),
                PBIUrl::new("SalariesFrance", "SalariesFrance_6.csv.bz2"),
                PBIUrl::new("SalariesFrance", "SalariesFrance_7.csv.bz2"),
                PBIUrl::new("SalariesFrance", "SalariesFrance_8.csv.bz2"),
                PBIUrl::new("SalariesFrance", "SalariesFrance_9.csv.bz2"),
                PBIUrl::new("SalariesFrance", "SalariesFrance_10.csv.bz2"),
                PBIUrl::new("SalariesFrance", "SalariesFrance_11.csv.bz2"),
                PBIUrl::new("SalariesFrance", "SalariesFrance_12.csv.bz2"),
                PBIUrl::new("SalariesFrance", "SalariesFrance_13.csv.bz2"),
            ],
        ),
        (
            TableroSistemaPenal,
            vec![
                PBIUrl::new("TableroSistemaPenal", "TableroSistemaPenal_1.csv.bz2"),
                PBIUrl::new("TableroSistemaPenal", "TableroSistemaPenal_2.csv.bz2"),
                PBIUrl::new("TableroSistemaPenal", "TableroSistemaPenal_3.csv.bz2"),
                PBIUrl::new("TableroSistemaPenal", "TableroSistemaPenal_4.csv.bz2"),
                PBIUrl::new("TableroSistemaPenal", "TableroSistemaPenal_5.csv.bz2"),
                PBIUrl::new("TableroSistemaPenal", "TableroSistemaPenal_6.csv.bz2"),
                PBIUrl::new("TableroSistemaPenal", "TableroSistemaPenal_7.csv.bz2"),
                PBIUrl::new("TableroSistemaPenal", "TableroSistemaPenal_8.csv.bz2"),
            ],
        ),
        (
            Taxpayer,
            vec![
                PBIUrl::new("Taxpayer", "Taxpayer_1.csv.bz2"),
                PBIUrl::new("Taxpayer", "Taxpayer_2.csv.bz2"),
                PBIUrl::new("Taxpayer", "Taxpayer_3.csv.bz2"),
                PBIUrl::new("Taxpayer", "Taxpayer_4.csv.bz2"),
                PBIUrl::new("Taxpayer", "Taxpayer_5.csv.bz2"),
                PBIUrl::new("Taxpayer", "Taxpayer_6.csv.bz2"),
                PBIUrl::new("Taxpayer", "Taxpayer_7.csv.bz2"),
                PBIUrl::new("Taxpayer", "Taxpayer_8.csv.bz2"),
                PBIUrl::new("Taxpayer", "Taxpayer_9.csv.bz2"),
                PBIUrl::new("Taxpayer", "Taxpayer_10.csv.bz2"),
            ],
        ),
        (Telco, vec![PBIUrl::new("Telco", "Telco_1.csv.bz2")]),
        (
            TrainsUK1,
            vec![
                PBIUrl::new("TrainsUK1", "TrainsUK1_1.csv.bz2"),
                PBIUrl::new("TrainsUK1", "TrainsUK1_2.csv.bz2"),
                PBIUrl::new("TrainsUK1", "TrainsUK1_3.csv.bz2"),
                PBIUrl::new("TrainsUK1", "TrainsUK1_4.csv.bz2"),
            ],
        ),
        (
            TrainsUK2,
            vec![
                PBIUrl::new("TrainsUK2", "TrainsUK2_1.csv.bz2"),
                PBIUrl::new("TrainsUK2", "TrainsUK2_2.csv.bz2"),
            ],
        ),
        (
            USCensus,
            vec![
                PBIUrl::new("USCensus", "USCensus_1.csv.bz2"),
                PBIUrl::new("USCensus", "USCensus_2.csv.bz2"),
                PBIUrl::new("USCensus", "USCensus_3.csv.bz2"),
            ],
        ),
        (
            Uberlandia,
            vec![PBIUrl::new("Uberlandia", "Uberlandia_1.csv.bz2")],
        ),
        (
            Wins,
            vec![
                PBIUrl::new("Wins", "Wins_1.csv.bz2"),
                PBIUrl::new("Wins", "Wins_2.csv.bz2"),
                PBIUrl::new("Wins", "Wins_3.csv.bz2"),
                PBIUrl::new("Wins", "Wins_4.csv.bz2"),
            ],
        ),
        (
            YaleLanguages,
            vec![
                PBIUrl::new("YaleLanguages", "YaleLanguages_1.csv.bz2"),
                PBIUrl::new("YaleLanguages", "YaleLanguages_2.csv.bz2"),
                PBIUrl::new("YaleLanguages", "YaleLanguages_3.csv.bz2"),
                PBIUrl::new("YaleLanguages", "YaleLanguages_4.csv.bz2"),
                PBIUrl::new("YaleLanguages", "YaleLanguages_5.csv.bz2"),
            ],
        ),
    ])
});

impl PBIDataset {
    pub fn dataset_name(&self) -> &str {
        let url = URLS.get(self).unwrap();
        url.first().unwrap().dataset_name
    }

    fn list_files(&self, file_type: FileType) -> Vec<PathBuf> {
        let urls = URLS.get(self).unwrap();
        urls.iter()
            .map(|url| self.get_file_path(url, file_type))
            .collect_vec()
    }

    fn get_file_path(&self, url: &PBIUrl, file_type: FileType) -> PathBuf {
        let extension = format!("{file_type}");

        "PBI"
            .to_data_path()
            .join(self.dataset_name())
            .join(extension.as_str())
            .join(url.file_name.strip_suffix(".csv.bz2").unwrap())
            .with_extension(extension.as_str())
    }

    fn download_bzip(&self) {
        let urls = URLS.get(self).unwrap();
        self.dataset_name();
        urls.iter().for_each(|url| {
            let fname = self.get_bzip_path(url);
            download_data(fname, url.to_url_string().as_str());
        });
    }

    fn get_bzip_path(&self, url: &PBIUrl) -> PathBuf {
        "PBI"
            .to_data_path()
            .join(self.dataset_name())
            .join("bzip2")
            .join(url.file_name)
    }

    fn unzip(&self) {
        for url in URLS.get(self).unwrap() {
            let bzipped = self.get_bzip_path(url);
            let unzipped_csv = self.get_file_path(url, FileType::Csv);
            decompress_bz2(bzipped, unzipped_csv);
        }
    }
}

#[derive(Debug)]
struct PBIUrl {
    dataset_name: &'static str,
    file_name: &'static str,
}

impl PBIUrl {
    fn new(dataset_name: &'static str, file_name: &'static str) -> Self {
        Self {
            dataset_name,
            file_name,
        }
    }
    fn to_url_string(&self) -> Url {
        Url::parse("https://event.cwi.nl/da/PublicBIbenchmark/")
            .unwrap()
            .join(format!("{}/", self.dataset_name).as_str())
            .unwrap()
            .join(self.file_name)
            .unwrap()
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, EnumIter)]
pub enum PBIDataset {
    AirlineSentiment,
    Arade,
    Bimbo,
    CMSprovider,
    CityMaxCapita,
    CommonGovernment,
    Corporations,
    Eixo,
    Euro2016,
    Food,
    Generico,
    HashTags,
    Hatred,
    IGlocations1,
    IGlocations2,
    IUBLibrary,
    MLB,
    MedPayment1,
    MedPayment2,
    Medicare1,
    Medicare2,
    Medicare3,
    Motos,
    MulheresMil,
    NYC,
    PanCreactomy1,
    PanCreactomy2,
    Physicians,
    Provider,
    RealEstate1,
    RealEstate2,
    Redfin1,
    Redfin2,
    Redfin3,
    Redfin4,
    Rentabilidad,
    Romance,
    SalariesFrance,
    TableroSistemaPenal,
    Taxpayer,
    Telco,
    TrainsUK1,
    TrainsUK2,
    USCensus,
    Uberlandia,
    Wins,
    YaleLanguages,
}

impl TryFrom<u8> for PBIDataset {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(AirlineSentiment),
            1 => Ok(Arade),
            2 => Ok(Bimbo),
            3 => Ok(CMSprovider),
            4 => Ok(CityMaxCapita),
            5 => Ok(CommonGovernment),
            6 => Ok(Corporations),
            7 => Ok(Eixo),
            8 => Ok(Euro2016),
            9 => Ok(Food),
            10 => Ok(Generico),
            11 => Ok(HashTags),
            12 => Ok(Hatred),
            13 => Ok(IGlocations1),
            14 => Ok(IGlocations2),
            15 => Ok(IUBLibrary),
            16 => Ok(MLB),
            17 => Ok(MedPayment1),
            18 => Ok(MedPayment2),
            19 => Ok(Medicare1),
            20 => Ok(Medicare2),
            21 => Ok(Medicare3),
            22 => Ok(Motos),
            23 => Ok(MulheresMil),
            24 => Ok(NYC),
            25 => Ok(PanCreactomy1),
            26 => Ok(PanCreactomy2),
            27 => Ok(Physicians),
            28 => Ok(Provider),
            29 => Ok(RealEstate1),
            30 => Ok(RealEstate2),
            31 => Ok(Redfin1),
            32 => Ok(Redfin2),
            33 => Ok(Redfin3),
            34 => Ok(Redfin4),
            35 => Ok(Rentabilidad),
            36 => Ok(Romance),
            37 => Ok(SalariesFrance),
            38 => Ok(TableroSistemaPenal),
            39 => Ok(Taxpayer),
            40 => Ok(Telco),
            41 => Ok(TrainsUK1),
            42 => Ok(TrainsUK2),
            43 => Ok(USCensus),
            44 => Ok(Uberlandia),
            45 => Ok(Wins),
            46 => Ok(YaleLanguages),
            _ => bail!("Invalid dataset index"),
        }
    }
}

pub struct CsvToPqOptions {
    pub rg_size: usize,
    pub is_dict_scope: bool,
}

impl Default for CsvToPqOptions {
    fn default() -> Self {
        Self {
            rg_size: parquet::file::properties::DEFAULT_MAX_ROW_GROUP_SIZE,
            is_dict_scope: false,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct BtrMetadata {
    matrix: Vec<Vec<(usize, usize)>>,
}

pub trait BenchmarkDataset {
    fn as_uncompressed(&self);
    fn compress_to_vortex(&self) -> Result<()>;
    fn write_as_parquet(&self, opt: CsvToPqOptions);
    fn write_as_vortex(&self) -> impl Future<Output = ()>;
    fn write_as_vortex_latest(&self) -> impl Future<Output = ()>;
    fn write_as_lance(&self) -> impl Future<Output = ()>;
    fn write_as_lance_v2_1(&self) -> impl Future<Output = ()>;
    fn ra_lance(&self, row_ids: Vec<usize>) -> impl Future<Output = Result<()>>;
    fn write_as_fff(&self, options: FileWriterOptions);
    /// With Wasm written in
    fn write_as_fff_wasm(&self);
    fn ra_fff(&self, row_ids: &[u64]) -> Result<()>;
    fn read_fff(&self) -> Result<()>;
    /// Read using Wasm
    fn read_fff_wasm(&self) -> Result<()>;
    fn write_as_orc(&self);
    fn write_as_btrblocks(&self);
    fn read_btrblocks(&self) -> Result<()>;
    fn read_parquet(&self) -> Result<()>;
    fn ra_parquet(&self, row_ids: Vec<usize>) -> impl Future<Output = Result<()>>;
    fn read_vortex(&self) -> impl Future<Output = Result<()>>;
    fn read_vortex_latest(&self) -> impl Future<Output = Result<()>>;
    fn ra_vortex(&self, row_ids: Vec<usize>) -> impl Future<Output = Result<()>>;
    fn read_lance(&self) -> impl Future<Output = Result<()>>;
    fn read_orc(&self) -> Result<()>;
    fn ra_orc(&self, row_id: usize) -> Result<()>;
    fn ra_nimble(&self, row_id: usize) -> Result<()>;
    fn list_files(&self, file_type: FileType) -> Vec<PathBuf>;
    fn directory_location(&self) -> PathBuf;
}

#[derive(Debug)]
pub enum BenchmarkDatasets {
    CFB(CFBDataset),
    PBI(PBIDataset),
    LAION(LaionDataset),
    TPCH(TPCHDataset),
    CLICKBENCH(ClickBenchDataset),
}

impl BenchmarkDatasets {
    pub fn num_rows(&self) -> usize {
        self.write_as_parquet(CsvToPqOptions::default());
        let pq_files = self.list_files(FileType::Parquet);
        let first_file = pq_files.first().unwrap();
        // use Parquet reader to get the number of rows
        let file = File::open(first_file).unwrap();
        let parquet_reader =
            parquet::file::reader::SerializedFileReader::new(file).expect("Unable to read file");
        let row_group_metadata =
            parquet::file::reader::FileReader::metadata(&parquet_reader).row_groups();
        let mut total_num_rows = 0;

        for group_metadata in row_group_metadata {
            total_num_rows += group_metadata.num_rows();
        }
        total_num_rows as usize
    }
    pub fn remove_parquet(&self) {
        for f in self.list_files(FileType::Parquet) {
            std::fs::remove_file(f)
                .map_err(|e| info!("Failed to remove parquet: {}", e))
                .ok();
        }
    }

    pub fn remove_fff(&self) {
        for f in self.list_files(FileType::FFF) {
            std::fs::remove_file(f)
                .map_err(|e| info!("Failed to remove FFF: {}", e))
                .ok();
        }
    }

    fn write_fff(&self, options: FileWriterOptions) {
        self.write_as_parquet(CsvToPqOptions::default());
        let wasm_flag = options.write_built_in_wasm();
        for f in self.list_files(FileType::Parquet) {
            let batches = parquet_into_batches(f.clone(), Default::default()).unwrap();
            let output_fname = f
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .strip_suffix(".parquet")
                .unwrap();
            let compressed = idempotent(
                &path_for_file_type(
                    self,
                    output_fname,
                    format!("fff{}", if wasm_flag { "wasm" } else { "" }).as_str(),
                ),
                |output_path| {
                    let fff = std::fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(output_path)
                        .unwrap();
                    write_fff(&batches, &fff, options.clone()).unwrap();
                    Ok(())
                },
            )
            .expect("Failed to compress to parquet");

            let fff_size = std::fs::OpenOptions::new()
                .read(true)
                .open(compressed.clone())
                .unwrap()
                .metadata()
                .unwrap()
                .size();
            error!(
                "Max Chunk size in FFF: {}B",
                get_max_chunk_size(Arc::new(
                    std::fs::OpenOptions::new()
                        .read(true)
                        .open(compressed)
                        .unwrap()
                ))
                .unwrap()
            );

            error!(
                "FFF{} size: {}, {}B",
                if wasm_flag { "wasm" } else { "" },
                format_size(fff_size as u64, DECIMAL),
                fff_size
            );
            if PRINT_CR {
                error!(
                    "FFF CR: {:.2}",
                    fff_size as f64 / get_arrow_size(self, output_fname) as f64
                );
            }
        }
    }

    async fn _write_as_lance(&self, options: LanceFileWriterOptions) {
        self.write_as_parquet(CsvToPqOptions::default());
        for f in self.list_files(FileType::Parquet) {
            let options_local = options.clone();
            // info!("Compressing {} to lance", f.to_str().unwrap());
            let output_fname = f
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .strip_suffix(".parquet")
                .unwrap()
                .to_string();
            let compressed = idempotent_async(
                &path_for_file_type(self, &output_fname, "lancev2_1"),
                |output_path| async move {
                    let batches = parquet_into_batches(f.clone(), Default::default()).unwrap();
                    write_lance(&batches, output_path.to_str().unwrap(), true, options_local).await
                },
            )
            .await
            .unwrap();

            let lance_dir_bytes_exact = get_size(compressed).unwrap();
            let lance_dir_size = humansize::format_size(lance_dir_bytes_exact, DECIMAL);

            error!("Lance size: {}, {}B", lance_dir_size, lance_dir_bytes_exact);
            if PRINT_CR {
                error!(
                    "Lance CR: {:.2}",
                    lance_dir_bytes_exact as f64 / get_arrow_size(self, &output_fname) as f64
                );
            }
        }
    }
}

impl BenchmarkDataset for BenchmarkDatasets {
    fn as_uncompressed(&self) {
        match self {
            BenchmarkDatasets::CFB(_dataset) => {
                CFBDataset::download_all();
            }
            BenchmarkDatasets::PBI(pbidataset) => {
                pbidataset.download_bzip();
                pbidataset.unzip();
            }
            BenchmarkDatasets::LAION(_laion) => {
                LaionDataset::download_all();
            }
            BenchmarkDatasets::TPCH(_) => {
                TPCHDataset::download_all();
            }
            BenchmarkDatasets::CLICKBENCH(_) => {
                ClickBenchDataset::download_all();
            }
        }
    }

    fn write_as_fff(&self, options: FileWriterOptions) {
        self.write_fff(options);
    }

    fn write_as_fff_wasm(&self) {
        self.write_fff(
            FileWriterOptionsBuilder::with_defaults()
                .write_built_in_wasm(true)
                .build(),
        );
    }

    fn read_fff(&self) -> Result<()> {
        for f in self.list_files(FileType::FFF) {
            info!("Reading fff file {}", f.to_str().unwrap());
            let start = Instant::now();
            let batches = read_fff(f, ReadFFFOpt::default()).unwrap();
            error!("Reading fff file took {}ms", start.elapsed().as_millis());
            drop(batches);
        }
        Ok(())
    }

    fn ra_fff(&self, row_ids: &[u64]) -> Result<()> {
        for f in self.list_files(FileType::FFFRa) {
            info!("Reading fff file {}", f.to_str().unwrap());
            let start = Instant::now();
            let batches = read_fff(
                f,
                ReadFFFOpt {
                    selection: Some(fff_poc::reader::Selection::RowIndexes(row_ids.to_vec())),
                    ..Default::default()
                },
            )
            .unwrap();
            error!(
                "Random access fff file took {}ms",
                start.elapsed().as_millis()
            );
            drop(batches);
        }
        Ok(())
    }

    fn read_fff_wasm(&self) -> Result<()> {
        for f in self.list_files(FileType::FFFWasm) {
            info!("Reading fffwasm file {}", f.to_str().unwrap());
            let aot_wasm =
                "/home/xinyu/fff-devel/target/wasm32-wasip1/opt-size-lvl3/fff_ude_example_fff.cwasm";
            let rt = Runtime::try_new_from_aot(&fs::read(aot_wasm)?)?;
            let wasm_rts = HashMap::from([(WASMId(0), rt.into())]);
            let start = Instant::now();
            let batches = read_fff_aot_wasm(f, wasm_rts).unwrap();
            error!(
                "Reading fffwasm file took {}ms",
                start.elapsed().as_millis()
            );
            drop(batches);
        }
        Ok(())
    }

    fn compress_to_vortex(&self) -> Result<()> {
        self.write_as_parquet(CsvToPqOptions::default());
        for f in self.list_files(FileType::Parquet) {
            // info!("Compressing and writing {} to vortex", f.to_str().unwrap());
            let from_vortex = compress_parquet_to_vortex(f.as_path()).unwrap();
            let vx_size = from_vortex.nbytes();

            error!(
                "Vortex size: {}, {}B",
                format_size(vx_size as u64, DECIMAL),
                vx_size
            );
        }
        Ok(())
    }

    fn write_as_parquet(&self, opt: CsvToPqOptions) {
        self.as_uncompressed();
        if matches!(
            self,
            BenchmarkDatasets::LAION(_) | BenchmarkDatasets::CLICKBENCH(_)
        ) {
            // Laion's downloaded data is just Parquet
            for f in self.list_files(FileType::Parquet) {
                let pq_size = std::fs::File::open(f.clone())
                    .unwrap()
                    .metadata()
                    .unwrap()
                    .size();
                error!(
                    "Parquet size: {}, {}B",
                    format_size(pq_size, DECIMAL),
                    pq_size
                );
                if PRINT_CR {
                    error!(
                        "Parquet CR: {:.2}",
                        pq_size as f64
                            / get_arrow_size(
                                self,
                                f.file_name()
                                    .unwrap()
                                    .to_str()
                                    .unwrap()
                                    .strip_suffix(".parquet")
                                    .unwrap()
                                    .to_string()
                                    .as_str()
                            ) as f64
                    );
                }
            }
            return;
        }
        // if matches!(self, BenchmarkDatasets::LAION(_)) {
        //     for f in self.list_files(FileType::Parquet) {
        //         let compressed = idempotent(&f, |_output_path| {
        //             rewrite_parquet_via_mine(f.clone(), &f, opt.rg_size)
        //         })
        //         .expect("Failed to compress to parquet");
        //         let pq_size = compressed.metadata().unwrap().size();
        //         error!(
        //             "Parquet size: {}, {}B",
        //             format_size(pq_size, DECIMAL),
        //             pq_size
        //         );
        //     }
        //     return;
        // }
        for f in self.list_files(FileType::Csv) {
            let output_fname = f
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .strip_suffix(".csv")
                .unwrap()
                .to_string();
            let compressed = idempotent(
                &path_for_file_type(self, &output_fname, "parquet"),
                |output_path| match self {
                    BenchmarkDatasets::CFB(_) => {
                        write_csv_as_parquet(f, output_path, ",", "", opt.rg_size, false)
                    }
                    BenchmarkDatasets::PBI(_) => write_csv_as_parquet(
                        f,
                        output_path,
                        "|",
                        "null",
                        opt.rg_size,
                        opt.is_dict_scope,
                    ),
                    BenchmarkDatasets::LAION(_) => {
                        unreachable!()
                    }
                    BenchmarkDatasets::TPCH(_) => {
                        write_csv_as_parquet(f, output_path, ",", "", opt.rg_size, false)
                    }
                    BenchmarkDatasets::CLICKBENCH(_) => {
                        unreachable!()
                    }
                },
            )
            .expect("Failed to compress to parquet");
            let pq_size = compressed.metadata().unwrap().size();
            error!(
                "Parquet size: {}, {}B",
                format_size(pq_size, DECIMAL),
                pq_size
            );
            if PRINT_CR {
                error!(
                    "Parquet CR: {:.2}",
                    pq_size as f64 / get_arrow_size(self, &output_fname) as f64
                );
            }
        }
    }

    fn read_parquet(&self) -> Result<()> {
        self.write_as_parquet(CsvToPqOptions::default());
        for f in self.list_files(FileType::Parquet) {
            info!("Reading parquet file {}", f.to_str().unwrap());
            let start = Instant::now();
            let file = File::open(f)?;
            let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
            let reader = builder.build()?;
            // for batch in reader {
            //     let _batch = batch?;
            // }
            let _batches = reader.collect::<Result<Vec<RecordBatch>, _>>().unwrap();
            error!(
                "Reading parquet file took {}ms",
                start.elapsed().as_millis()
            );
        }
        Ok(())
    }

    async fn ra_parquet(&self, row_ids: Vec<usize>) -> Result<()> {
        self.write_as_parquet(CsvToPqOptions::default());
        for f in self.list_files(FileType::Parquet) {
            info!("Reading parquet file {}", f.to_str().unwrap());
            let start = Instant::now();
            let _batches = take_parquet(
                f.as_path(),
                &row_ids.iter().map(|id| *id as u64).collect::<Vec<_>>(),
            )
            .await
            .unwrap();
            error!(
                "Random access parquet file took {}ms",
                start.elapsed().as_millis()
            );
        }
        Ok(())
    }

    async fn ra_vortex(&self, row_ids: Vec<usize>) -> Result<()> {
        self.write_as_parquet(CsvToPqOptions::default());
        for f in self.list_files(FileType::Vortex) {
            let start = Instant::now();
            let _batches = take_vortex_tokio(
                f.as_path(),
                &row_ids.iter().map(|id| *id as u64).collect::<Vec<_>>(),
            )
            .await
            .unwrap()
            .into_arrow()
            .unwrap();
            // println!("{}", batches.tree_display());
            error!(
                "Random access vortex file took {}ms",
                start.elapsed().as_millis()
            );
        }
        Ok(())
    }

    fn write_as_orc(&self) {
        self.write_as_parquet(CsvToPqOptions::default());
        for f in self.list_files(FileType::Parquet) {
            // info!("Compressing and writing {} to vortex", f.to_str().unwrap());
            let output_fname = f
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .strip_suffix(".parquet")
                .unwrap()
                .to_string();
            let compressed = idempotent(
                &path_for_file_type(self, &output_fname, "orc"),
                |output_path| {
                    let batches = parquet_into_batches(f, Default::default()).unwrap();
                    write_orc(
                        batches.as_slice(),
                        output_path.as_os_str().to_str().unwrap(),
                    )
                },
            )
            .unwrap();
            let orc_size = std::fs::File::open(compressed)
                .unwrap()
                .metadata()
                .unwrap()
                .size();

            error!(
                "ORC size: {}, {}B",
                format_size(orc_size as u64, DECIMAL),
                orc_size
            );
            if PRINT_CR {
                error!(
                    "ORC CR: {:.2}",
                    orc_size as f64 / get_arrow_size(self, &output_fname) as f64
                );
            }
        }
    }

    fn read_orc(&self) -> Result<()> {
        for f in self.list_files(FileType::Orc) {
            info!("Reading ORC file {}", f.to_str().unwrap());
            let start = Instant::now();
            read_orc(f.to_str().unwrap(), None).unwrap();
            error!("Reading ORC file took {}ms", start.elapsed().as_millis());
        }
        Ok(())
    }

    fn ra_orc(&self, row_id: usize) -> Result<()> {
        // hacky code to call the C++ executable.
        // /home/xinyu/nimble/build/Release/fff-bench/selection_orc row_id
        let path = self.list_files(FileType::Orc).into_iter().next().unwrap();
        // replace "/orc/" with "/orc_cpp/"
        let path = path.to_str().unwrap().replace("/orc/", "/orc_cpp/");
        let output = Command::new("/home/xinyu/nimble/build/Release/fff-bench/selection_orc")
            .arg(row_id.to_string())
            .arg(path.as_str())
            .output()
            .unwrap();
        println!("{}", String::from_utf8(output.stdout).unwrap());
        error!("{}", String::from_utf8(output.stderr).unwrap());
        Ok(())
    }

    fn ra_nimble(&self, row_id: usize) -> Result<()> {
        // hacky code to call the C++ executable.
        // /home/xinyu/nimble/build/Release/fff-bench/selection_nimble row_id
        let path = self.list_files(FileType::Orc).into_iter().next().unwrap();
        // replace "/orc/" with "/nimble_uncompressed/"
        let path = path.to_str().unwrap().replace("/orc/", "/nimble_uncomp/");
        let path = path.replace(".orc", ".nimble");
        let output = Command::new("/home/xinyu/nimble/build/Release/fff-bench/selection_nimble")
            .arg(row_id.to_string())
            .arg(path.as_str())
            .output()
            .unwrap();
        println!("{}", String::from_utf8(output.stdout).unwrap());
        error!("{}", String::from_utf8(output.stderr).unwrap());
        Ok(())
    }

    async fn read_vortex(&self) -> Result<()> {
        for f in self.list_files(FileType::Vortex) {
            info!("Reading vortex file {}", f.to_str().unwrap());
            let start = Instant::now();

            read_vortex(f, vortex_file::Projection::All).await?;
            error!("Reading vortex file took {}ms", start.elapsed().as_millis());
        }
        Ok(())
    }

    async fn read_lance(&self) -> Result<()> {
        self._write_as_lance(LanceFileWriterOptions::default())
            .await;
        for f in self.list_files(FileType::Lance) {
            info!("Reading lance file {}", f.to_str().unwrap());
            let start = Instant::now();
            let _batches = read_lance(f.to_str().unwrap(), None, None, true)
                .await
                .unwrap();
            error!("Reading lance file took {}ms", start.elapsed().as_millis());
        }
        Ok(())
    }

    async fn ra_lance(&self, row_ids: Vec<usize>) -> Result<()> {
        self._write_as_lance(LanceFileWriterOptions::default())
            .await;
        for f in self.list_files(FileType::Lance) {
            info!("Random access lance file {}", f.to_str().unwrap());
            let start = Instant::now();
            let _batches = read_lance(f.to_str().unwrap(), None, Some(row_ids.clone()), true)
                .await
                .unwrap();
            error!(
                "Random access lance file took {}ms",
                start.elapsed().as_millis()
            );
        }
        Ok(())
    }

    async fn write_as_vortex(&self) {
        self.write_as_parquet(CsvToPqOptions::default());
        for f in self.list_files(FileType::Parquet) {
            // info!("Compressing and writing {} to vortex", f.to_str().unwrap());
            let output_fname = f
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .strip_suffix(".parquet")
                .unwrap()
                .to_string();
            let compressed = idempotent_async(
                &path_for_file_type(self, &output_fname, "vortex"),
                |output_path| async move {
                    let batches = parquet_into_batches(f, Default::default()).unwrap();
                    let write = tokio::fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(output_path)
                        .await
                        .unwrap();
                    write_vortex(&batches, write).await
                },
            )
            .await
            .unwrap();
            let vx_size = std::fs::File::open(compressed)
                .unwrap()
                .metadata()
                .unwrap()
                .size();

            error!(
                "Vortex size: {}, {}B",
                format_size(vx_size as u64, DECIMAL),
                vx_size
            );
            if PRINT_CR {
                error!(
                    "Vortex CR: {:.2}",
                    vx_size as f64 / get_arrow_size(self, &output_fname) as f64
                );
            }
        }
    }
    async fn write_as_vortex_latest(&self) {
        todo!()
    }

    async fn read_vortex_latest(&self) -> Result<()> {
        todo!()
    }

    async fn write_as_lance(&self) {
        self._write_as_lance(LanceFileWriterOptions::default())
            .await
    }

    async fn write_as_lance_v2_1(&self) {
        let opts = lance_file::v2::writer::FileWriterOptions {
            format_version: Some(LanceFileVersion::V2_1),
            ..Default::default()
        };
        self._write_as_lance(opts).await
    }

    fn write_as_btrblocks(&self) {
        self.write_as_parquet(CsvToPqOptions::default());
        for f in self.list_files(FileType::Parquet) {
            // info!("Compressing and writing {} to vortex", f.to_str().unwrap());
            let output_fname = f
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .strip_suffix(".parquet")
                .unwrap()
                .to_string();
            let compressed = idempotent(
                &path_for_file_type(self, &output_fname, "btr"),
                |output_path| {
                    let batches =
                        parquet_into_batches(f, PqToBatchesOptions::with_batch_size(usize::MAX))
                            .unwrap();
                    let compress_blk_sizes = write_btrblocks(
                        batches.as_slice(),
                        output_path.as_os_str().to_str().unwrap(),
                    )
                    .unwrap();
                    let file = File::create(path_for_btr_sizes(self, &output_fname)).unwrap();
                    let writer = BufWriter::new(file);
                    let data_wrapper = BtrMetadata {
                        matrix: compress_blk_sizes,
                    };
                    bincode::serialize_into(writer, &data_wrapper)?;
                    Ok(())
                },
            )
            .unwrap();
            let orc_size = std::fs::File::open(compressed)
                .unwrap()
                .metadata()
                .unwrap()
                .size();

            error!(
                "BtrBlocks size: {}, {}B",
                format_size(orc_size as u64, DECIMAL),
                orc_size
            );
            if PRINT_CR {
                error!(
                    "BtrBlocks CR: {:.2}",
                    orc_size as f64 / get_arrow_size(self, &output_fname) as f64
                );
            }
        }
    }

    fn read_btrblocks(&self) -> Result<()> {
        panic!(
            "comment off here due to anonymous request and it is hard for us to solve the dep issue"
        );
        // for f in self.list_files(FileType::BtrBlocks) {
        //     let pq_paths = self.list_files(FileType::Parquet);
        //     assert!(pq_paths.len() == 1);
        //     let file_reader = std::fs::File::open(pq_paths.into_iter().next().unwrap()).unwrap();
        //     let reader = dictscope_bench::reader::ParquetReader::new(file_reader);
        //     let batch = reader.read().unwrap();
        //     let block_types = batch
        //         .columns()
        //         .iter()
        //         .filter(|column| {
        //             dictscope_bench::column::SUPPORTED_DTYPES
        //                 .contains(&discriminant(column.data_type()))
        //         })
        //         .map(|column| {
        //             let dict_column = dictscope_bench::column::Column::new(column);
        //             dict_column
        //                 .slice_with_step(crate::BTR_ENC_BLOCK_SIZE)
        //                 .map(|slice| slice.data_type())
        //                 .collect::<Vec<_>>()
        //                 .into_iter()
        //         })
        //         .flatten()
        //         .collect::<Vec<_>>();
        //     let start = Instant::now();
        //     info!("Reading BtrBlocks file {}", f.to_str().unwrap());
        //     let file = File::open(path_for_btr_sizes(
        //         self,
        //         &f.to_str().unwrap().strip_suffix(".btr").unwrap(),
        //     ))?;
        //     let reader = BufReader::new(file);
        //     let compress_sizes: BtrMetadata = bincode::deserialize_from(reader).unwrap();
        //     let (compressed_sizes, compressed_null_sizes): (Vec<_>, Vec<_>) = compress_sizes
        //         .matrix
        //         .iter()
        //         .flatten()
        //         .map(|x| (x.0 as usize, x.1 as usize))
        //         .unzip();
        //     let mut compressor = dictscope_bench::compress::BtrCompressor::new();
        //     compressor.set_disable_dict(false);
        //     compressor
        //         .decompress_batch_from_file(
        //             f.to_str().unwrap().to_string(),
        //             &compressed_sizes,
        //             &compressed_null_sizes,
        //             &block_types,
        //         )
        //         .unwrap();
        //     error!(
        //         "Reading BtrBlocks file took {}ms",
        //         start.elapsed().as_millis()
        //     );
        // }
        // Ok(())
    }

    fn list_files(&self, types: FileType) -> Vec<PathBuf> {
        match self {
            BenchmarkDatasets::CFB(dataset) => dataset.list_files(types),
            BenchmarkDatasets::PBI(pbidataset) => pbidataset.list_files(types),
            BenchmarkDatasets::LAION(pbidataset) => pbidataset.list_files(types),
            BenchmarkDatasets::TPCH(pbidataset) => pbidataset.list_files(types),
            BenchmarkDatasets::CLICKBENCH(pbidataset) => pbidataset.list_files(types),
        }
    }

    fn directory_location(&self) -> PathBuf {
        match self {
            BenchmarkDatasets::CFB(_dataset) => "data".to_data_path(),
            BenchmarkDatasets::PBI(pbidataset) => {
                "PBI".to_data_path().join(pbidataset.dataset_name())
            }
            BenchmarkDatasets::LAION(_dataset) => "laion".to_data_path(),
            BenchmarkDatasets::TPCH(_dataset) => "tpch".to_data_path(),
            BenchmarkDatasets::CLICKBENCH(_dataset) => "clickbench".to_data_path(),
        }
    }
}

fn path_for_file_type(
    dataset: &impl BenchmarkDataset,
    output_fname: &str,
    file_type: &str,
) -> PathBuf {
    dataset
        .directory_location()
        .join(file_type)
        .join(format!("{}.{}", output_fname, file_type))
}

fn path_for_btr_sizes(dataset: &impl BenchmarkDataset, output_fname: &str) -> PathBuf {
    dataset
        .directory_location()
        .join("btr")
        .join(format!("{}.{}", output_fname, "btrmeta"))
}

fn get_arrow_size(dataset: &impl BenchmarkDataset, output_fname: &str) -> u64 {
    parquet_into_batches(
        path_for_file_type(dataset, output_fname, "parquet"),
        Default::default(),
    )
    .unwrap()
    .iter()
    .map(|rb| rb.get_array_memory_size() as u64)
    .sum::<u64>()
}
