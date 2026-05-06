use std::error::Error;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use image::{Rgb, RgbImage};

use crate::image_matrix::convert::{pixel_to_gray, rgb_to_gray};
use crate::image_matrix::types::{GrayMatrix, RgbMatrix};

// ── Load ──────────────────────────────────────────────────────────────────────

/// Load an RgbMatrix from any image file that the `image` crate supports.
pub fn load_rgb_matrix_from_image(path: &str) -> Result<RgbMatrix, Box<dyn Error>> {
    let img = image::open(path)?.to_rgb8();
    let (w, h) = img.dimensions();
    let (width, height) = (w as usize, h as usize);

    let mut data = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let px = img.get_pixel(x as u32, y as u32);
            data.push([px[0], px[1], px[2]]);
        }
    }
    Ok(RgbMatrix {
        width,
        height,
        data,
    })
}

/// Load a GrayMatrix from an image file.
/// The image is converted to greyscale on load — no intermediate RgbMatrix held in memory.
pub fn load_gray_from_image(path: &str) -> Result<GrayMatrix, Box<dyn Error>> {
    let img = image::open(path)?.to_rgb8();
    let (w, h) = img.dimensions();
    let (width, height) = (w as usize, h as usize);

    let mut data = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let px = img.get_pixel(x as u32, y as u32);
            data.push(pixel_to_gray(px[0], px[1], px[2]));
        }
    }
    Ok(GrayMatrix {
        width,
        height,
        data,
    })
}

/// Load a template/reference as greyscale, preferring a cached RGB matrix file.
///
/// The PNG remains the human-inspectable source. The sidecar `.rgbm` stores
/// decoded RGB pixels, avoiding PNG decode on later loads. If the PNG is newer
/// than the sidecar, the sidecar is rebuilt automatically.
pub fn load_gray_from_image_cached(path: &str) -> Result<GrayMatrix, Box<dyn Error>> {
    let png_path = Path::new(path);
    let matrix_path = png_path.with_extension("rgbm");

    if matrix_path.exists() && !is_stale(&matrix_path, png_path)? {
        let rgb = load_rgb_matrix_binary(&matrix_path)?;
        return Ok(rgb_to_gray(&rgb));
    }

    let rgb = load_rgb_matrix_from_image(path)?;
    save_rgb_matrix_binary(&rgb, &matrix_path)?;
    Ok(rgb_to_gray(&rgb))
}

/// Load an RgbMatrix from a JSON file (serde_json).
pub fn load_rgb_matrix_from_json(path: &str) -> Result<RgbMatrix, Box<dyn Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let matrix: RgbMatrix = serde_json::from_reader(reader)?;
    Ok(matrix)
}

/// Load a GrayMatrix from a JSON file (serde_json).
pub fn load_gray_matrix_from_json(path: &str) -> Result<GrayMatrix, Box<dyn Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let matrix: GrayMatrix = serde_json::from_reader(reader)?;
    Ok(matrix)
}

// ── Save ──────────────────────────────────────────────────────────────────────

/// Save an RgbMatrix as a PNG file.
/// Creates parent directories automatically.
pub fn save_rgb_matrix_as_image(matrix: &RgbMatrix, path: &str) -> Result<(), Box<dyn Error>> {
    let path = Path::new(path);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut img = RgbImage::new(matrix.width as u32, matrix.height as u32);
    for y in 0..matrix.height {
        for x in 0..matrix.width {
            let px = matrix.get(x, y);
            img.put_pixel(x as u32, y as u32, Rgb(px));
        }
    }
    img.save(path)?;
    Ok(())
}

/// Save an RgbMatrix in compact project-native binary format.
///
/// Format:
/// - magic: `RGBM1`
/// - little-endian u32 width
/// - little-endian u32 height
/// - raw row-major RGB bytes
pub fn save_rgb_matrix_binary(matrix: &RgbMatrix, path: &Path) -> Result<(), Box<dyn Error>> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut file = BufWriter::new(File::create(path)?);
    file.write_all(b"RGBM1")?;
    file.write_all(&(matrix.width as u32).to_le_bytes())?;
    file.write_all(&(matrix.height as u32).to_le_bytes())?;
    for px in &matrix.data {
        file.write_all(px)?;
    }
    Ok(())
}

/// Load a compact project-native RgbMatrix file.
pub fn load_rgb_matrix_binary(path: &Path) -> Result<RgbMatrix, Box<dyn Error>> {
    let mut file = BufReader::new(File::open(path)?);
    let mut magic = [0u8; 5];
    file.read_exact(&mut magic)?;
    if &magic != b"RGBM1" {
        return Err(format!("{} is not an RGBM1 matrix file", path.display()).into());
    }

    let mut w = [0u8; 4];
    let mut h = [0u8; 4];
    file.read_exact(&mut w)?;
    file.read_exact(&mut h)?;
    let width = u32::from_le_bytes(w) as usize;
    let height = u32::from_le_bytes(h) as usize;
    let byte_len = width
        .checked_mul(height)
        .and_then(|px| px.checked_mul(3))
        .ok_or_else(|| format!("{} has impossible dimensions", path.display()))?;

    let mut raw = vec![0u8; byte_len];
    file.read_exact(&mut raw)?;
    let data = raw
        .chunks_exact(3)
        .map(|px| [px[0], px[1], px[2]])
        .collect();

    Ok(RgbMatrix {
        width,
        height,
        data,
    })
}

/// Rebuild a PNG from a saved `.rgbm` matrix file for inspection.
pub fn rebuild_png_from_rgb_matrix_file(
    matrix_path: &Path,
    png_path: &Path,
) -> Result<(), Box<dyn Error>> {
    let matrix = load_rgb_matrix_binary(matrix_path)?;
    save_rgb_matrix_as_image(&matrix, &png_path.to_string_lossy())
}

/// Save a GrayMatrix as a PNG file (expands to R=G=B for display).
pub fn save_gray_matrix_as_image(matrix: &GrayMatrix, path: &str) -> Result<(), Box<dyn Error>> {
    let rgb = crate::image_matrix::convert::gray_to_rgb(matrix);
    save_rgb_matrix_as_image(&rgb, path)
}

/// Save an RgbMatrix as a JSON file (serde_json, human-readable).
pub fn save_rgb_matrix_as_json(matrix: &RgbMatrix, path: &str) -> Result<(), Box<dyn Error>> {
    let path = Path::new(path);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let file = File::create(path)?;
    let writer = BufWriter::new(file);
    serde_json::to_writer(writer, matrix)?;
    Ok(())
}

fn is_stale(candidate: &Path, source: &Path) -> Result<bool, Box<dyn Error>> {
    let candidate_modified = fs::metadata(candidate)?.modified()?;
    let source_modified = fs::metadata(source)?.modified()?;
    Ok(candidate_modified < source_modified)
}

// `rgb_to_gray` is re-exported via its module; keep the import live so future
// io routines that round-trip through gray have it available.
#[allow(dead_code)]
fn _ensure_rgb_to_gray_in_scope(m: &RgbMatrix) -> GrayMatrix {
    rgb_to_gray(m)
}
