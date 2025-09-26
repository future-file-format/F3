#!/bin/bash

LOG_FILE="random_access.log"

# export TMPDIR=/mnt/nvme0n1/xinyu/tmp
# Function to run the command and log the output with iteration count
run_and_log() {
    local iteration_count=$1
    local extra_args=$2

    for i in $(seq 1 $iteration_count); do
        echo "Iteration $i" >> $LOG_FILE
        sudo sync
        echo 3 | sudo tee /proc/sys/vm/drop_caches > /dev/null
        ./target/release/examples/bench random-access -- $extra_args 2>&1 | tee -a $LOG_FILE 
        echo -e "\n" >> $LOG_FILE
    done
}
cargo build --example bench --release 
cmake --build /home/xinyu/nimble/build/Release --config Release --target selection_nimble selection_orc
# Remove existing log file if it exists
rm -f $LOG_FILE

# Run the first set of commands
echo "Running without additional features..." >> $LOG_FILE
for dataset in tpch clickbench core bi classic geo log ml; do
    echo "Start data set $dataset" >> $LOG_FILE
    run_and_log 10 $dataset
done

echo "Script execution completed. Check $LOG_FILE for the output."