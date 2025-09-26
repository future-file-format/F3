use fff_bench::config;
use serde_json::Value;

fn sum_compressed_bytes(json_data: &Value, column_index: usize) -> Vec<u64> {
    let row_groups = json_data["row_groups"].as_array().unwrap();

    row_groups
        .iter()
        .map(|row_group| {
            row_group["columns"][column_index]["pages"]
                .as_array()
                .unwrap()
                .iter()
                .map(|page| page["compressed_bytes"].as_u64().unwrap())
                .sum::<u64>()
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let column_index = args[1].parse::<usize>().unwrap();
    // gen the json file from the parquet file by running `parquet-layout merged_8M.parquet > merged_8M.json`
    let json_file =
        std::fs::File::open(config::get_base_data_path().join("laion/parquet/merged_8M.json"))
            .unwrap();
    let json_str = std::io::read_to_string(json_file).unwrap();

    let json_data: Value = serde_json::from_str(&json_str).unwrap();
    let total = sum_compressed_bytes(&json_data, column_index);
    println!(
        "Average column chunk size for column {}: {}",
        column_index,
        total.iter().sum::<u64>() / total.len() as u64
    );
}
