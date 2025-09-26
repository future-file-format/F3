#!/bin/bash
# This script should be deprecated

# Define the base command
BASE_CMD="/home/xinyu/arrow-rs/target/release/parquet-layout"

# Loop through each file and run the command
for rg_size in 65536 131072 262144 524288 1048576; do
    INPUT_FILE="/mnt/nvme0n1/xinyu/laion/parquet/0001_rg${rg_size}.parquet"
    OUTPUT_FILE="results/rg_size_laion/chunk_size_rg${rg_size}.json"
    
    # Run the command and save the output to the JSON file
    $BASE_CMD "$INPUT_FILE" > "$OUTPUT_FILE"
    
    echo "Generated $OUTPUT_FILE"
done