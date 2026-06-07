//! Shared fixture for the config-loader integration tests. Uniquely named
//! (`config_loader_support`) to avoid colliding with other agents' tests/ modules.
#![allow(dead_code)]

use std::io::Write;

use tempfile::NamedTempFile;

pub fn write_config(contents: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(contents.as_bytes()).unwrap();
    file
}
