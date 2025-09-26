#!/bin/bash

# Path to the program
PROGRAM="/home/xinyu/nimble/build/Release/fff-bench/projection_nimble"
COMBINED_OUTPUT="combined_nimble_projection_times.csv"

# Check if program exists
if [ ! -f "$PROGRAM" ]; then
    echo "Error: Program not found at $PROGRAM"
    exit 1
fi

# Remove previous combined output if it exists
if [ -f "$COMBINED_OUTPUT" ]; then
    rm "$COMBINED_OUTPUT"
fi

# Define array for number of columns
RUN_COUNT=5

# Run the program multiple times for each number of columns
for num_columns in 2333 10 20 100 1000 5000 10000 20000 50000 100000; do
    echo "Testing with $num_columns columns..."
    
    for i in $(seq 1 $RUN_COUNT); do
        echo "Running program: iteration $i of $RUN_COUNT for $num_columns columns..."
        
        # Remove previous output if exists
        OUTPUT_FILE="nimble_projection_times_${num_columns}.csv"
        if [ -f "$OUTPUT_FILE" ]; then
            rm "$OUTPUT_FILE"
        fi
        
        sudo sync
        echo 3 | sudo tee /proc/sys/vm/drop_caches > /dev/null
        
        # Run the program with the current number of columns
        $PROGRAM $num_columns
        
        # Check if the output file was created
        if [ ! -f "$OUTPUT_FILE" ]; then
            echo "Error: Program did not generate $OUTPUT_FILE on run $i"
            continue
        fi
        
        # For the first run of the first column set, copy the header row and add our columns to the combined file
        if [ $i -eq 1 ] && [ "$num_columns" = "2333" ]; then
            head -n 1 "$OUTPUT_FILE" | awk '{print $0",i,num_columns"}' > "$COMBINED_OUTPUT"
        fi
        
        # Add data rows with the run number and column count to the combined file
        # Skip the header row from the second file onwards
        tail -n +2 "$OUTPUT_FILE" | awk -v run=$i -v cols=$num_columns '{print $0","run","cols}' >> "$COMBINED_OUTPUT"
        
        echo "Added data from run $i for $num_columns columns to combined file"
        
        # Clean up individual output file
        rm "$OUTPUT_FILE"
    done
done

echo "Process complete. Combined data available in results/$COMBINED_OUTPUT"

mv $COMBINED_OUTPUT "results/$COMBINED_OUTPUT"