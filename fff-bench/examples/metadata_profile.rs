/// Profile the time breakdown for n_col = 1000 and 100000.
use std::{collections::HashSet, sync::Arc};

use anyhow::Result;
use fff_poc::reader::FileReaderV2Builder;
use itertools::Itertools;
use rand::{rngs::StdRng, Rng, SeedableRng};

#[tokio::main]
async fn main() -> Result<()> {
    let data_dir = "data_8rows";
    // create data_dir if not exists
    std::fs::create_dir_all(data_dir)?;
    let num_columns = std::env::args().nth(1).unwrap().parse::<usize>().unwrap();
    let fff = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(format!("{data_dir}/{}.fff", num_columns))?;

    // create 10 random unique numbers between 0 and num_columns
    let projections: Vec<usize> = {
        let mut rng = StdRng::seed_from_u64(42);
        let mut unique_numbers = HashSet::new();
        while unique_numbers.len() < 10 {
            let num = rng.gen_range(0..num_columns);
            unique_numbers.insert(num);
        }
        unique_numbers.into_iter().sorted().collect::<Vec<_>>()
    };
    // ---- Test FFF ----
    let f = Arc::new(fff);
    for _ in 0..10000 {
        let mut reader = FileReaderV2Builder::new(f.clone())
            .with_projections(fff_poc::reader::Projection::LeafColumnIndexes(
                projections.clone(),
            ))
            .build()
            .unwrap();
        let _result = reader.read_file().unwrap();
    }
    Ok(())
}
