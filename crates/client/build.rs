use std::path::Path;

const FONT_URL: &str = "https://raw.githubusercontent.com/googlefonts/noto-cjk/main/Sans/OTF/TraditionalChinese/NotoSansCJKtc-Regular.otf";
const FONT_PATH: &str = "assets/NotoSansTC-Regular.otf";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={FONT_PATH}");

    let font_path = Path::new(FONT_PATH);
    if font_path.exists() {
        return;
    }

    eprintln!("Downloading Noto Sans TC font...");

    if let Some(parent) = font_path.parent() {
        std::fs::create_dir_all(parent).expect("Failed to create assets directory");
    }

    // Try curl first (available on most systems)
    let status = std::process::Command::new("curl")
        .args(["-sL", "-o", FONT_PATH, FONT_URL])
        .status();

    match status {
        Ok(s) if s.success() => {
            let metadata =
                std::fs::metadata(font_path).expect("Font file not found after download");
            if metadata.len() < 1_000_000 {
                // Too small, probably an error page
                std::fs::remove_file(font_path).ok();
                panic!(
                    "Downloaded font is too small ({}B). \
                     Please download manually:\n  curl -sL -o {} {}",
                    metadata.len(),
                    FONT_PATH,
                    FONT_URL
                );
            }
            eprintln!(
                "Font downloaded successfully ({:.1}MB)",
                metadata.len() as f64 / 1_048_576.0
            );
        }
        _ => {
            panic!(
                "Failed to download font. Please download manually:\n  curl -sL -o {} {}",
                FONT_PATH, FONT_URL
            );
        }
    }
}
