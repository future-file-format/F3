import duckdb
import pyarrow.parquet as pq

# Connect to DuckDB (in-memory database)
con = duckdb.connect()

# Generate TPCH data at scale factor 10
con.execute("CALL dbgen(sf=10)")
# List of TPCH table names
tables = [
    "lineitem", "orders", "customer", "part", "supplier",
    "partsupp", "nation", "region"
]

# Export each table to Parquet format
for table in tables:
    # Create a relation from the table and export the entire relation as Arrow
    rel = con.table(table)
    relation_as_arrow = rel.arrow() # or .to_arrow_table()
    pq.write_table(relation_as_arrow, f'{table}.parquet', compression='snappy', row_group_size=1048576)
    # duckdb table
    con.execute(f"COPY {table} TO '{table}_duckdb.parquet' (FORMAT PARQUET, COMPRESSION SNAPPY, ROW_GROUP_SIZE 1048576)")
    con.execute(f'''-- Create a new table with the modified schema
CREATE TABLE {table}_double AS
SELECT
  l_orderkey,
  l_partkey,
  l_suppkey,
  l_linenumber,
  CAST(l_quantity AS DOUBLE) AS l_quantity,
  CAST(l_extendedprice AS DOUBLE) AS l_extendedprice,
  CAST(l_discount AS DOUBLE) AS l_discount,
  CAST(l_tax AS DOUBLE) AS l_tax,
  l_returnflag,
  l_linestatus,
  CAST(l_shipdate AS INTEGER) AS l_shipdate,
  CAST(l_commitdate AS INTEGER) AS l_commitdate,
  CAST(l_receiptdate AS INTEGER) AS l_receiptdate,
  l_shipinstruct,
  l_shipmode,
  l_comment
FROM '{table}_duckdb.parquet';

-- Write the new data to a parquet file
COPY {table}_double TO '{table}_double.parquet' (FORMAT PARQUET, COMPRESSION SNAPPY, ROW_GROUP_SIZE 1048576);''')
# Close the connection
con.close()

'''

'''