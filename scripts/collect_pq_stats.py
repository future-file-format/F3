import subprocess
import json
import os
from collections import defaultdict

def run_command(command):
    """Run a shell command and return its output. Handle errors gracefully."""
    try:
        result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, shell=True)
        if result.returncode != 0:
            print(f"Command failed: {command}")
            print(f"Error: {result.stderr}")
            return None
        return result.stdout
    except Exception as e:
        print(f"Exception occurred while running command: {command}")
        print(f"Exception: {str(e)}")
        return None

def extract_metadata(output):
    """Extract 'version' and 'created by' from the parquet-schema output."""
    metadata = {}
    for line in output.splitlines():
        if line.startswith("version:"):
            metadata["version"] = line.split(":")[1].strip()
        elif line.startswith("created by:"):
            metadata["created_by"] = line.split(":")[1].strip()
    return metadata

def extract_layout_info(output):
    """Extract unique 'encoding', 'page_type', and 'compression' from the parquet-layout output."""
    layout_info = defaultdict(set)
    try:
        data = json.loads(output)
    except json.JSONDecodeError as e:
        print(f"Failed to parse JSON output: {e}")
        return None
    
    for row_group in data.get("row_groups", []):
        for column in row_group.get("columns", []):
            for page in column.get("pages", []):
                layout_info["encoding"].add(page.get("encoding"))
                layout_info["page_type"].add(page.get("page_type"))
                layout_info["compression"].add(page.get("compression"))
    
    # Convert sets to lists for JSON serialization
    for key in layout_info:
        layout_info[key] = list(layout_info[key])
    
    return layout_info

def main():
    parquet_files = [f for f in os.listdir() if f.endswith('.parquet')]
    results = {}

    for parquet_file in parquet_files:
        print(f"Processing {parquet_file}...")
        file_info = {}

        # Extract metadata using parquet-schema
        schema_output = run_command(f"parquet-schema {parquet_file}")
        if schema_output:
            file_info.update(extract_metadata(schema_output))
        else:
            print(f"Skipping metadata extraction for {parquet_file} due to errors.")
            continue  # Skip to the next file if metadata extraction fails

        # Extract layout info using parquet-layout
        layout_output = run_command(f"parquet-layout {parquet_file}")
        if layout_output:
            layout_info = extract_layout_info(layout_output)
            if layout_info:
                file_info.update(layout_info)
            else:
                print(f"Skipping layout extraction for {parquet_file} due to JSON parsing errors.")
        else:
            print(f"Skipping layout extraction for {parquet_file} due to command errors.")

        results[parquet_file] = file_info

    # Save results to a JSON file
    with open("parquet_analysis.json", "w") as f:
        json.dump(results, f, indent=4)

    print("Analysis complete. Results saved to parquet_analysis.json.")

if __name__ == "__main__":
    main()