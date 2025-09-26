# Convert Map type into List of Struct in a Parquet file

import pyarrow.parquet as pq
import pyarrow as pa

# Read the Parquet file
table = pq.read_table("/mnt/nvme0n1/xinyu/laion/parquet/merged_8M.parquet")

# Extract the Map column
map_column = table.column("exif")

# Convert Map to List of Struct
struct_list = []
for map_item in map_column:
    if map_item is not None:
        struct_list.append([{"key": k, "value": v} for [k, v] in map_item.as_py()])

# Create a new schema with the List of Struct
struct_schema = pa.list_(
    pa.struct([
        pa.field("key", pa.string()),
        pa.field("value", pa.string())
    ])
)

# Create a new Arrow array for the List of Struct
struct_array = pa.array(struct_list, type=struct_schema)

# Create a new table with the converted column
new_table = table.set_column(
    table.schema.get_field_index("exif"),
    "list_of_struct",
    struct_array
)

# Write the new table to a Parquet file
pq.write_table(new_table, "/mnt/nvme0n1/xinyu/laion/parquet/merged_8M_new.parquet")