#!/usr/bin/env python3

# This script is used to extract the unique dependencies from the Cargo.toml and Cargo.lock files.
# For a xxx project proposal.
import sys
import toml

def main():
    # Load Cargo.toml
    try:
        with open("Cargo.toml", "r") as f:
            cargo_toml = toml.load(f)
    except Exception as e:
        sys.exit(f"Error reading Cargo.toml: {e}")

    # Extract dependency names from [workspace.dependencies]
    workspace_deps = set()
    ws = cargo_toml.get("workspace", {})
    if "dependencies" in ws:
        workspace_deps = set(ws["dependencies"].keys())
    else:
        sys.exit("No [workspace.dependencies] found in Cargo.toml")

    # Load Cargo.lock
    try:
        with open("Cargo.lock", "r") as f:
            cargo_lock = toml.load(f)
    except Exception as e:
        sys.exit(f"Error reading Cargo.lock: {e}")

    # Set to hold unique git URLs
    git_urls = set()

    # Iterate over packages in Cargo.lock
    for pkg in cargo_lock.get("package", []):
        name = pkg.get("name", "")
        source = pkg.get("source", "")
        # Check if this package is one of the workspace dependencies and has a git source
        if name in workspace_deps:
            if source.startswith("git+"):
                # Extract the URL part (strip the "git+" prefix and remove the commit hash)
                url = source.split("#")[0]
                if url.startswith("git+"):
                    url = url[len("git+"):]
                git_urls.add(url)
            elif source == "registry+https://github.com/rust-lang/crates.io-index":
                git_urls.add(f"https://crates.io/crates/{name}")
                

    # Output each unique git URL
    for url in sorted(git_urls):
        print(url)

if __name__ == "__main__":
    main()

'''
https://crates.io/crates/anyhow
https://crates.io/crates/arrow
https://crates.io/crates/arrow-array
https://crates.io/crates/arrow-buffer
https://crates.io/crates/arrow-cast
https://crates.io/crates/arrow-data
https://crates.io/crates/arrow-ipc
https://crates.io/crates/arrow-json
https://crates.io/crates/arrow-schema
https://crates.io/crates/bytemuck
https://crates.io/crates/byteorder
https://crates.io/crates/bytes
https://crates.io/crates/chrono
https://crates.io/crates/clap
https://crates.io/crates/criterion
https://crates.io/crates/fastlanes
https://crates.io/crates/flatbuffers
https://crates.io/crates/flatc-rust
https://crates.io/crates/flexbuffers
https://crates.io/crates/fs_extra
https://crates.io/crates/futures
https://crates.io/crates/futures-executor
https://crates.io/crates/futures-util
https://crates.io/crates/humansize
https://crates.io/crates/lazy_static
https://crates.io/crates/log
https://crates.io/crates/lz4_flex
https://crates.io/crates/object_store
https://crates.io/crates/parquet
https://crates.io/crates/pco
https://crates.io/crates/pprof
https://crates.io/crates/rand
https://crates.io/crates/reqwest
https://crates.io/crates/snafu
https://crates.io/crates/strum
https://crates.io/crates/strum_macros
https://crates.io/crates/tempfile
https://crates.io/crates/tokio
https://crates.io/crates/uniffi_core
https://crates.io/crates/wasi-common
https://crates.io/crates/wasm-bindgen
https://crates.io/crates/wasmtime
https://github.com/apache/arrow-rs.git
https://github.com/apache/datafusion.git
https://github.com/lancedb/lance.git
https://github.com/spiraldb/vortex.git
https://github.com/RoaringBitmap/roaring-rs.git'''