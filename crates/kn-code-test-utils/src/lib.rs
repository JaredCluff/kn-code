//! Test utilities for kn-code
//!
//! Note: This crate intentionally uses blocking std::fs operations
//! since test utilities run in synchronous test contexts.

#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use tempfile::TempDir;

pub struct TestWorkspace {
    pub dir: TempDir,
}

impl TestWorkspace {
    pub fn new() -> Self {
        let dir = TempDir::new().expect("Failed to create temp directory");
        Self { dir }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }

    pub fn create_file(&self, path: &str, content: &str) -> PathBuf {
        let full_path = self.dir.path().join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).expect("Failed to create parent directory");
        }
        std::fs::write(&full_path, content).expect("Failed to write test file");
        full_path
    }
}

impl Default for TestWorkspace {
    fn default() -> Self {
        Self::new()
    }
}

pub fn mock_json_response(body: &str) -> Option<serde_json::Value> {
    serde_json::from_str(body).ok()
}

pub fn try_create_file(dir: &Path, path: &str, content: &str) -> std::io::Result<PathBuf> {
    let full_path = dir.join(path);
    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&full_path, content)?;
    Ok(full_path)
}
