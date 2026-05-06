//! Screen capture via the cross-platform `screenshots` crate.
//!
//! Coordinates passed in are in *logical* pixels — the same coordinate
//! space `enigo` uses for mouse moves and `screenshots`'s `display_info`
//! reports.
//!
//! On Retina displays the crate captures at **physical** pixel resolution
//! (e.g., a logical 60×55 capture returns a 120×110 image). We detect
//! that scale factor at runtime and downsample by **block-averaging**
//! pixels back to logical resolution. Result: regardless of DPI, the
//! returned `RgbMatrix`/`GrayMatrix` matches the logical region the
//! caller asked for, so templates calibrated at logical resolution
//! match their on-screen counterparts.
//!
//! macOS: the first call triggers a *Screen Recording* permission
//! prompt. Until granted, every capture returns an error.

use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};

use screenshots::Screen;

use crate::image_matrix::convert::pixel_to_gray;
use crate::image_matrix::types::{GrayMatrix, RgbMatrix};

static SCALE_LOGGED: AtomicBool = AtomicBool::new(false);

/// Capture a screen region as an RgbMatrix at *logical* resolution.
pub fn capture_rgb_matrix(x1: i32, y1: i32, x2: i32, y2: i32) -> Result<RgbMatrix, Box<dyn Error>> {
    if x2 <= x1 || y2 <= y1 {
        return Err(format!("Invalid capture coordinates: ({x1},{y1})→({x2},{y2})").into());
    }
    let req_w = (x2 - x1) as u32;
    let req_h = (y2 - y1) as u32;

    let screen = primary_screen()?;
    let image = screen
        .capture_area(x1, y1, req_w, req_h)
        .map_err(|e| format!("capture_area failed: {e}"))?;

    let (sx, sy) = log_scale_once(image.width(), image.height(), req_w, req_h);

    let mut data = Vec::with_capacity((req_w * req_h) as usize);
    for y in 0..req_h {
        for x in 0..req_w {
            let (r, g, b) = if sx == 1 && sy == 1 {
                let p = image.get_pixel(x, y);
                (p[0], p[1], p[2])
            } else {
                let mut rs = 0u32;
                let mut gs = 0u32;
                let mut bs = 0u32;
                for by in 0..sy {
                    for bx in 0..sx {
                        let p = image.get_pixel(x * sx + bx, y * sy + by);
                        rs += p[0] as u32;
                        gs += p[1] as u32;
                        bs += p[2] as u32;
                    }
                }
                let n = sx * sy;
                ((rs / n) as u8, (gs / n) as u8, (bs / n) as u8)
            };
            data.push([r, g, b]);
        }
    }
    Ok(RgbMatrix {
        width: req_w as usize,
        height: req_h as usize,
        data,
    })
}

/// Capture a screen region directly as a GrayMatrix at *logical* resolution.
pub fn capture_gray_matrix(
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
) -> Result<GrayMatrix, Box<dyn Error>> {
    if x2 <= x1 || y2 <= y1 {
        return Err(format!("Invalid capture coordinates: ({x1},{y1})→({x2},{y2})").into());
    }
    let req_w = (x2 - x1) as u32;
    let req_h = (y2 - y1) as u32;

    let screen = primary_screen()?;
    let image = screen
        .capture_area(x1, y1, req_w, req_h)
        .map_err(|e| format!("capture_area failed: {e}"))?;

    let (sx, sy) = log_scale_once(image.width(), image.height(), req_w, req_h);

    let mut data = Vec::with_capacity((req_w * req_h) as usize);
    for y in 0..req_h {
        for x in 0..req_w {
            let (r, g, b) = if sx == 1 && sy == 1 {
                let p = image.get_pixel(x, y);
                (p[0], p[1], p[2])
            } else {
                let mut rs = 0u32;
                let mut gs = 0u32;
                let mut bs = 0u32;
                for by in 0..sy {
                    for bx in 0..sx {
                        let p = image.get_pixel(x * sx + bx, y * sy + by);
                        rs += p[0] as u32;
                        gs += p[1] as u32;
                        bs += p[2] as u32;
                    }
                }
                let n = sx * sy;
                ((rs / n) as u8, (gs / n) as u8, (bs / n) as u8)
            };
            data.push(pixel_to_gray(r, g, b));
        }
    }
    Ok(GrayMatrix {
        width: req_w as usize,
        height: req_h as usize,
        data,
    })
}

/// Logical (width, height) of the primary display.
pub fn primary_screen_size() -> Result<(i32, i32), Box<dyn Error>> {
    let s = primary_screen()?;
    let info = &s.display_info;
    Ok((info.width as i32, info.height as i32))
}

fn primary_screen() -> Result<Screen, Box<dyn Error>> {
    Screen::all()?
        .into_iter()
        .next()
        .ok_or_else(|| "No screens found".into())
}

/// Compute and one-time-log the physical/logical scale factor.
fn log_scale_once(actual_w: u32, actual_h: u32, req_w: u32, req_h: u32) -> (u32, u32) {
    let sx = (actual_w / req_w.max(1)).max(1);
    let sy = (actual_h / req_h.max(1)).max(1);
    if !SCALE_LOGGED.swap(true, Ordering::Relaxed) {
        eprintln!(
            "[capture] DPI scale detected: requested {}×{}, returned {}×{} → block-average {}×{}",
            req_w, req_h, actual_w, actual_h, sx, sy,
        );
    }
    (sx, sy)
}
