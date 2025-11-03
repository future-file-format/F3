use anyhow::Context;
use anyhow::Result;
use std::process::Command;
use std::{env, path::Path};

fn main() -> Result<()> {
    // Use relative path because of https://github.com/rust-lang/cargo/issues/3946
    let flatc = flatc_rust::Flatc::from_path(
        Path::new(&env::var("CARGO_MANIFEST_DIR")?).join("../third_party/flatbuffers/flatc"),
    );

    // compile flatc if it does not exist
    if flatc.check().is_err() && !Command::new("sh")
            .current_dir(Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join(".."))
            .args(["scripts/install_flatc.sh"])
            .status()
            .with_context(|| "install_flatc.sh failed")?
            .success() {
        anyhow::bail!("install_flatc.sh failed");
    }

    flatc
        .check()
        .with_context(|| "flatc not found under third_party/flatbuffers!")?;

    println!(
        "cargo:rerun-if-changed={}/../format",
        &env::var("CARGO_MANIFEST_DIR")?
    );
    // Use relative path because of https://github.com/rust-lang/cargo/issues/3946
    let format_dir =
        Path::new(&env::var("CARGO_MANIFEST_DIR").with_context(|| "no manifest dir in env")?)
            .join("../format");
    let fbs_path_bufs = format_dir
        .read_dir()
        .with_context(|| "format dir not found")?
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().unwrap() == "fbs")
        .collect::<Vec<_>>();
    flatc.run(flatc_rust::Args {
        inputs: &fbs_path_bufs
            .iter()
            .map(|p| p.as_path())
            .collect::<Vec<&Path>>(),
        out_dir: Path::new(env::var("OUT_DIR").unwrap().as_str()),
        extra: &["--filename-suffix", ""],
        ..Default::default()
    })?;

    Ok(())
}
