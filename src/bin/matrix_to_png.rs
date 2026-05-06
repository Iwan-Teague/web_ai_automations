//! Rebuild an inspectable PNG from a saved `.rgbm` matrix file.
//!
//! ```
//! cargo run --bin matrix_to_png -- <matrix.rgbm> <output.png>
//! cargo run --bin matrix_to_png -- --from-png <source.png> <output.rgbm>
//! cargo run --bin matrix_to_png -- --crop-png <source.png> <x> <y> <w> <h> <output.png> [scale_divisor]
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use image::imageops::FilterType;
use web_ai_automation::image_matrix::io::{
    load_rgb_matrix_from_image, rebuild_png_from_rgb_matrix_file, save_rgb_matrix_binary,
};

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() == 4 && argv[1] == "--from-png" {
        let png_path = PathBuf::from(&argv[2]);
        let matrix_path = PathBuf::from(&argv[3]);
        return match load_rgb_matrix_from_image(&png_path.to_string_lossy())
            .and_then(|m| save_rgb_matrix_binary(&m, &matrix_path))
        {
            Ok(()) => {
                println!("saved matrix → {}", matrix_path.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("png_to_matrix failed: {e}");
                ExitCode::from(1)
            }
        };
    }

    if (argv.len() == 8 || argv.len() == 9) && argv[1] == "--crop-png" {
        let source = PathBuf::from(&argv[2]);
        let parse_num = |idx: usize, name: &str| -> Result<u32, String> {
            argv[idx]
                .parse::<u32>()
                .map_err(|e| format!("invalid {name}: {e}"))
        };
        let x = match parse_num(3, "x") {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(2);
            }
        };
        let y = match parse_num(4, "y") {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(2);
            }
        };
        let w = match parse_num(5, "w") {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(2);
            }
        };
        let h = match parse_num(6, "h") {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(2);
            }
        };
        let output = PathBuf::from(&argv[7]);
        let scale_divisor = if argv.len() == 9 {
            match argv[8].parse::<u32>() {
                Ok(v) if v > 0 => v,
                Ok(_) => {
                    eprintln!("invalid scale_divisor: must be > 0");
                    return ExitCode::from(2);
                }
                Err(e) => {
                    eprintln!("invalid scale_divisor: {e}");
                    return ExitCode::from(2);
                }
            }
        } else {
            1
        };

        return match image::open(&source) {
            Ok(img) => {
                let cropped = img.crop_imm(x, y, w, h);
                let final_img = if scale_divisor > 1 {
                    cropped.resize_exact(w / scale_divisor, h / scale_divisor, FilterType::Nearest)
                } else {
                    cropped
                };
                match final_img.save(&output) {
                    Ok(()) => {
                        println!("cropped PNG → {}", output.display());
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("save crop failed: {e}");
                        ExitCode::from(1)
                    }
                }
            }
            Err(e) => {
                eprintln!("open crop source failed: {e}");
                ExitCode::from(1)
            }
        };
    }

    if argv.len() != 3 {
        eprintln!("usage: cargo run --bin matrix_to_png -- <matrix.rgbm> <output.png>");
        eprintln!("       cargo run --bin matrix_to_png -- --from-png <source.png> <output.rgbm>");
        eprintln!(
            "       cargo run --bin matrix_to_png -- --crop-png <source.png> <x> <y> <w> <h> <output.png> [scale_divisor]"
        );
        return ExitCode::from(2);
    }

    let matrix_path = PathBuf::from(&argv[1]);
    let png_path = PathBuf::from(&argv[2]);

    match rebuild_png_from_rgb_matrix_file(&matrix_path, &png_path) {
        Ok(()) => {
            println!("rebuilt PNG → {}", png_path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("matrix_to_png failed: {e}");
            ExitCode::from(1)
        }
    }
}
