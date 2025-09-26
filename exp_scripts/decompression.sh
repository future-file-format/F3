#!/bin/bash

# First run cargo run --example bench --release -- compressed-size 2>&1 | tee compress_bench.log
# If already runned, don't forget to run cargo build --example bench --release 

# Function to run the command and log the output with iteration count
run_and_log() {
    local iteration_count=$1
    local extra_args=$2

    for i in $(seq 1 $iteration_count); do
        echo "Iteration $i" >> decomp.log
        # for format in "btrblocks"; do
        for format in "fff" "fffwasm" "parquet" "vortex" "lance" "btrblocks"; do
            sudo sync
            echo 3 | sudo tee /proc/sys/vm/drop_caches > /dev/null
            ./target/release/examples/bench scan-time $format 2>&1 | tee -a decomp.log 
        done
        echo -e "\n" >> decomp.log
    done
}

# Remove existing log file if it exists
rm -f decomp.log

# Run the first set of commands
echo "Running without additional features..." >> decomp.log
run_and_log 5

echo "Script execution completed. Check decomp.log for the output."