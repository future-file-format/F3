use anyhow::{Context, Result};
use std::{env, path::Path, process::Command};

fn main() -> Result<()> {
    println!("cargo:rerun-if-changed=fallback_scalar_aav_1024_uf1_unpack_src.cpp");
    if !Command::new(
        Path::new(&env::var("CARGO_MANIFEST_DIR").with_context(|| "no manifest dir in env")?)
            .join("./compile_unpack_wasm.sh"),
    )
    .status()
    .with_context(|| "compile_unpack_wasm.sh failed")?
    .success()
    {
        anyhow::bail!("compile_unpack_wasm.sh failed");
    }
    Ok(())
}
