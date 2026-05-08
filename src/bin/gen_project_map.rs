use std::path::PathBuf;
use web_ai_automation::project_map;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: gen_project_map <root_dir> <output_path>");
        std::process::exit(1);
    }
    let root = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    match project_map::generate(&root, &out) {
        Ok(map) => {
            for p in &map.paths {
                println!("{}", p.display());
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
