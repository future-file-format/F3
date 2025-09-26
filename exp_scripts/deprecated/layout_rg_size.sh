#!/bin/bash

# Directory to store results
RESULTS_DIR="results/rg_size"
mkdir -p "$RESULTS_DIR"

# Maximum number of parallel jobs
MAX_JOBS=10

# Function to run the command
# export TMPDIR=/mnt/nvme0n1/xinyu/tmp
run_command() {
    local dataset=$1
    cargo run --example bench_mem --release -- "$dataset" 2>&1 | tee "$RESULTS_DIR/bench_mem_${dataset}.log"
}

# Export the function so it can be used by parallel jobs
export -f run_command

# Loop through datasets 0 to 46
for dataset in {0..46}; do
    # Run the command in the background
    run_command "$dataset" &

    # Limit the number of parallel jobs
    if [[ $(jobs -r -p | wc -l) -ge $MAX_JOBS ]]; then
        # Wait for one job to finish if we've reached the limit
        wait -n
    fi
done

# Wait for all remaining background jobs to finish
wait

echo "All commands completed."
send_email layout_rg_size