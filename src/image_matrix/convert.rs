use crate::image_matrix::types::{GrayMatrix, RgbMatrix};

/// Convert RgbMatrix → GrayMatrix using integer BT.601 weights.
///
/// Y ≈ (77·R + 150·G + 29·B) >> 8   (max error vs. float: ±1 LSB)
pub fn rgb_to_gray(src: &RgbMatrix) -> GrayMatrix {
    let mut dst = GrayMatrix::new(src.width, src.height);
    for (i, px) in src.data.iter().enumerate() {
        dst.data[i] = pixel_to_gray(px[0], px[1], px[2]);
    }
    dst
}

/// Convert GrayMatrix → RgbMatrix by setting R = G = B = luma.
/// Useful for saving a greyscale matrix as a PNG for debugging.
pub fn gray_to_rgb(src: &GrayMatrix) -> RgbMatrix {
    let data = src.data.iter().map(|&v| [v, v, v]).collect();
    RgbMatrix {
        width: src.width,
        height: src.height,
        data,
    }
}

/// Convert a single RGB pixel to a greyscale luminance byte.
#[inline(always)]
pub fn pixel_to_gray(r: u8, g: u8, b: u8) -> u8 {
    ((77u16 * r as u16 + 150u16 * g as u16 + 29u16 * b as u16) >> 8) as u8
}
