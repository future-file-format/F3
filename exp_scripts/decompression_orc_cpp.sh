#!/bin/bash

# Path to the program
# PROGRAM="/home/xinyu/nimble/build/Release/fff-bench/read_orc"
# OUTPUT_FILE="orc_read_times.csv"
# COMBINED_OUTPUT="combined_orc_read_times.csv"

PROGRAM="/home/xinyu/orc/build/Release/tools/src/read_orc"
OUTPUT_FILE="orc_read_times_original_cpp.csv"
COMBINED_OUTPUT="combined_orc_read_times_original_cpp.csv"
# Check if program exists
if [ ! -f "$PROGRAM" ]; then
    echo "Error: Program not found at $PROGRAM"
    exit 1
fi

# Remove previous combined output if it exists
if [ -f "$COMBINED_OUTPUT" ]; then
    rm "$COMBINED_OUTPUT"
fi

# Initialize combined file header
# We'll get the header from the first run, then add our 'i' column
RUN_COUNT=5

# Run the program multiple times
for i in $(seq 1 $RUN_COUNT); do
    echo "Running program: iteration $i of $RUN_COUNT..."
    
    # Remove previous output if exists
    if [ -f "$OUTPUT_FILE" ]; then
        rm "$OUTPUT_FILE"
    fi
    
    sudo sync
    echo 3 | sudo tee /proc/sys/vm/drop_caches > /dev/null

    # Run the program
    $PROGRAM
    
    # Check if the output file was created
    if [ ! -f "$OUTPUT_FILE" ]; then
        echo "Error: Program did not generate $OUTPUT_FILE on run $i"
        continue
    fi
    
    # For the first run, copy the header row and add our 'i' column to the combined file
    if [ $i -eq 1 ]; then
        head -n 1 "$OUTPUT_FILE" | awk '{print $0",i"}' > "$COMBINED_OUTPUT"
    fi
    
    # Add data rows with the run number to the combined file
    # Skip the header row from the second file onwards
    tail -n +2 "$OUTPUT_FILE" | awk -v run=$i '{print $0","run}' >> "$COMBINED_OUTPUT"
    
    echo "Added data from run $i to combined file"
done

echo "Process complete. Combined data available in results/$COMBINED_OUTPUT"

rm $OUTPUT_FILE
mv $COMBINED_OUTPUT "results/$COMBINED_OUTPUT"