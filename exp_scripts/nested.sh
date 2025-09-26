#!/bin/bash

# Function to run the command and log the output with iteration count
run_and_log() {
    local iteration_count=$1
    local extra_args=$2

    for i in $(seq 1 $iteration_count); do
        echo "Iteration $i" >> nested.log
        cargo run --example nested_exp --release $extra_args -- ra >> nested.log 
        echo -e "\n" >> nested.log
    done
}

# Remove existing log file if it exists
rm -f nested.log

# Run the first set of commands
echo "Running without additional features..." >> nested.log
run_and_log 5

# Run the second set of commands
echo "Running with list-offsets-pushdown feature..." >> nested.log
run_and_log 5 "--features list-offsets-pushdown"

echo "Script execution completed. Check nested.log for the output."