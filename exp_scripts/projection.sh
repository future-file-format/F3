#!/bin/bash

OUT_FILE="proj_8rows.log"
# OUT_FILE="proj_8rows_f3unchecked.log"
# OUT_FILE="proj.log"

# Define array for formats
FORMATS=("fff" "parquet" "lance" "orc" "vortex")

# Function to run the command and log the output with iteration count
run_and_log() {
    local format="$1"
    local num_columns="$2"
    local iteration_count="$3"

    for i in $(seq 1 $iteration_count); do
        echo "Iteration $i - Format: $format, Columns: $num_columns" >> $OUT_FILE
        sudo sync
        echo 3 | sudo tee /proc/sys/vm/drop_caches > /dev/null
        sleep 0.05
        ./target/release/examples/metadata_test_v2 test --format "$format" --num-columns "$num_columns" >> $OUT_FILE 
        # sleep 1
        echo -e "\n" >> $OUT_FILE
    done
}

# Remove existing log file if it exists
rm -f $OUT_FILE
cargo build --example metadata_test_v2 --release
# Run tests for each combination of format and columns
for format in "${FORMATS[@]}"; do
    echo "Testing format: $format" >> $OUT_FILE
    for num_columns in 2333 10 20 100 1000 5000 10000 20000 50000 100000; do
        echo "Testing with $num_columns columns..." >> $OUT_FILE
        run_and_log "$format" "$num_columns" 20
    done
    echo -e "\n" >> $OUT_FILE
done

echo "Script execution completed. Check $OUT_FILE for the output."