#!/bin/bash
# Unzip all the .gz file from https://homepages.cwi.nl/~boncz/RealNest/tables_655360.tar

# Define the base directory
base_dir="/public/xinyu/RealNest/tables_655360"

# Find all the data.jsonl.gz files under the base directory
find "$base_dir" -name "data.jsonl.gz" | while read gz_file; do
  # Get the directory of the gzipped file
  dir=$(dirname "$gz_file")

  # Define the output file path
  output_file="$dir/data.jsonl"

  # Unzip the file
  echo "Extracting $gz_file to $output_file"
  gunzip -c "$gz_file" > "$output_file"

  # Optional: If you want to remove the original .gz file after extraction, uncomment the following line:
  # rm "$gz_file"
done