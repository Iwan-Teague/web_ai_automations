//! Disk-backed reference matrix store.
//!
//! Layout: `<base_dir>/<state_name>.png` (greyscale 8-bit PNG).
//!
//! The base dir is `assets/macos/arena.ai/calibration/` relative to the working directory
//! by default — gitignored, per-machine. Each entry is one calibrated UI
//! state captured at a known-good moment (see `walkthrough` module).

use std::path::{Path, PathBuf};

use crate::image_matrix::types::GrayMatrix;

#[derive(Clone, Debug)]
pub struct CalibrationStore {
    base_dir: PathBuf,
}

impl CalibrationStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// Default location: `assets/macos/arena.ai/calibration/` relative to CWD.
    pub fn default_arena() -> Self {
        Self::new(
            PathBuf::from("assets")
                .join("macos")
                .join("arena.ai")
                .join("calibration"),
        )
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    pub fn path_for(&self, state: &str) -> PathBuf {
        self.base_dir.join(format!("{state}.png"))
    }

    pub fn exists(&self, state: &str) -> bool {
        self.path_for(state).exists()
    }

    /// Save a `GrayMatrix` as 8-bit greyscale PNG. Creates the base dir
    /// if missing. Returns the on-disk path.
    pub fn save(&self, state: &str, matrix: &GrayMatrix) -> Result<PathBuf, String> {
        std::fs::create_dir_all(&self.base_dir)
            .map_err(|e| format!("create_dir_all {}: {e}", self.base_dir.display()))?;
        let path = self.path_for(state);
        let img = image::GrayImage::from_raw(
            matrix.width as u32,
            matrix.height as u32,
            matrix.data.clone(),
        )
        .ok_or_else(|| {
            format!(
                "GrayImage::from_raw shape mismatch ({}×{} != {} bytes)",
                matrix.width,
                matrix.height,
                matrix.data.len()
            )
        })?;
        img.save(&path)
            .map_err(|e| format!("png save {}: {e}", path.display()))?;
        Ok(path)
    }

    /// Load a saved reference back into a `GrayMatrix`. Uses the standard
    /// `.rgbm` sidecar cache: a fresh `.rgbm` next to the PNG is loaded as
    /// raw bytes and converted to gray, skipping PNG decode entirely.
    /// Stale or missing sidecars are rebuilt automatically on first load.
    pub fn load(&self, state: &str) -> Result<GrayMatrix, String> {
        let path = self.path_for(state);
        let path_str = path.to_string_lossy().into_owned();
        crate::image_matrix::io::load_gray_from_image_cached(&path_str)
            .map_err(|e| format!("matrix load {}: {e}", path.display()))
    }

    /// List all calibrated state names (file stems without `.png`).
    pub fn list(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(read) = std::fs::read_dir(&self.base_dir) {
            for entry in read.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("png") {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        out.push(stem.to_string());
                    }
                }
            }
        }
        out.sort();
        out
    }
}
