# Helper script to exact columns from a Parque file.

import duckdb

# Connect to DuckDB (using an in-memory database here)
con = duckdb.connect()

# Read the Parquet file, selecting only the first 8 columns
df = con.execute("SELECT * FROM '/mnt/nvme0n1/xinyu/data/parquet/core.parquet' LIMIT 0").fetchdf()
# first_9_columns = list(df.columns[0:8])
# first_9_columns.extend(list(df.columns[12:20]))
first_9_columns = df.columns[12]

# Write the selected columns to a new Parquet file
        # SELECT {', '.join(first_9_columns)}
con.execute(f"""
    COPY (
        SELECT {first_9_columns}
        FROM '/mnt/nvme0n1/xinyu/data/parquet/core.parquet'
    ) TO '/mnt/nvme0n1/xinyu/tmp/12_str.parquet' (FORMAT PARQUET)
""")