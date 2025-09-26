use std::fs::File;
/// Time to project a column from the Laion fff file.
use std::sync::Arc;

use anyhow::Result;
use fff_bench::config;
use fff_poc::reader::FileReaderV2Builder;

#[tokio::main]
async fn main() -> Result<()> {
    let col_idx = std::env::args().nth(1).unwrap().parse::<usize>().unwrap();
    // for num_rows in [65536] {
    // for num_rows in [1000, 10000, 100000] {
    let start = std::time::Instant::now();
    let mut reader = FileReaderV2Builder::new(Arc::new(File::open(
        config::get_base_data_path().join("laion/fff/merged_8M_rg1048576.fff"),
    )?))
    .with_projections(fff_poc::reader::Projection::LeafColumnIndexes(vec![
        col_idx,
    ]))
    .build()
    .unwrap();
    let _result = reader.read_file().unwrap();
    let duration = start.elapsed();
    println!("Time taken: {:?}", duration);
    Ok(())
}
