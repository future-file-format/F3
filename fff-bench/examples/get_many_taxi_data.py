import pyarrow.parquet as pq
import requests
import os

PREFIX = "https://d37ci6vzurychx.cloudfront.net/trip-data/"
files = ["yellow_tripdata_2024-06.parquet","yellow_tripdata_2024-07.parquet", "yellow_tripdata_2024-08.parquet"]
os.makedirs("../data", exist_ok=True)

# Download files
for file in files:
    url = PREFIX + file
    response = requests.get(url)
    if response.status_code == 200:
        with open(f"../data/{file}", "wb") as f:
            f.write(response.content)
        print(f"Downloaded {file}")
    else:
        print(f"Failed to download {file}")
    

schema = pq.ParquetFile("../data/"+files[0]).schema_arrow
with pq.ParquetWriter("../data/combined.parquet", schema=schema) as writer:
    for file in files:
        writer.write_table(pq.read_table("../data/"+file, schema=schema))