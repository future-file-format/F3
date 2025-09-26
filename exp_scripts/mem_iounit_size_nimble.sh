#!/bin/bash
# What is the size of each column chunk in Nimble?

CMD="/home/xinyu/nimble/build/Release/fff-bench/projection_nimble_laion"
INPUT="/mnt/nvme0n1/xinyu/laion/nimble/merged_8M.nimble"
OUTPUT="results/nimble_col_size_laion.txt"
CSV_OUTPUT="results/nimble_col_size_laion.csv"

# Clear the output files
> $OUTPUT
> $CSV_OUTPUT

# Add CSV header
echo "col,avg_stream_size,avg_gap" > $CSV_OUTPUT

# Run the command for each column
for col in {0..9}; do
    # Run command and save output
    $CMD $INPUT $col | tee -a $OUTPUT
done

# Process the output using Python
python3 << EOF > $CSV_OUTPUT
import re

def process_output(filename):
    with open(filename, 'r') as f:
        content = f.read()
    
    # Split the content into runs (each run starts with "Projecting columns:")
    runs = content.split("Projecting columns:")
    
    results = []
    
    # Skip the first empty part if exists
    for run in runs[1:]:
        lines = run.strip().split('\n')
        
        # First line after "Projecting columns:" contains the column name
        col_name = lines[0].strip()
        
        # Extract all streamStarts and streamSizes
        stream_starts = []
        stream_sizes = []
        
        for line in lines:
            if "streamStart:" in line:
                parts = line.split()
                start = int(parts[1])
                stream_starts.append(start)
            
            if "streamSize:" in line:
                parts = line.split()
                size = int(parts[3])
                stream_sizes.append(size)
        
        # Calculate averages
        avg_stream_size = sum(stream_sizes) / len(stream_sizes) if stream_sizes else 0
        
        # Calculate gaps between consecutive stream starts
        gaps = []
        for i in range(len(stream_starts) - 1):
            gaps.append(stream_starts[i+1] - stream_starts[i])
        
        avg_gap = sum(gaps) / len(gaps) if gaps else 0
        
        results.append(f"{col_name},{avg_stream_size:.2f},{avg_gap:.2f}")
    
    return results

# Process the output file
print("col,avg_stream_size,avg_gap")  # Header

results = process_output("$OUTPUT")
for result in results:
    print(result)
EOF


