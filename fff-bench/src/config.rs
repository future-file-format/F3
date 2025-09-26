use std::path::PathBuf;
use std::sync::LazyLock;

/// Global configuration for the benchmark data paths
pub struct Config {
    /// Base directory where all benchmark data is stored
    pub base_data_path: PathBuf,
}

impl Config {
    /// Create a new config with the given base data path
    pub fn new(base_data_path: PathBuf) -> Self {
        Self { base_data_path }
    }

    /// Get the base data path, checking environment variable first, then falling back to default
    pub fn get_base_data_path() -> PathBuf {
        if let Ok(path) = std::env::var("FFF_BENCH_DATA_PATH") {
            PathBuf::from(path)
        } else {
            // Default fallback path
            PathBuf::from("/mnt/nvme0n1/xinyu/")
        }
    }
}

/// Global configuration instance
pub static CONFIG: LazyLock<Config> = LazyLock::new(|| Config::new(Config::get_base_data_path()));

/// Get the configured base data path
pub fn get_base_data_path() -> &'static PathBuf {
    &CONFIG.base_data_path
}
