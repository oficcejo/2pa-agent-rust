use rust_embed::RustEmbed;
use std::fs;
use std::path::Path;

#[derive(RustEmbed)]
#[folder = "prompt_engineering/"]
pub struct EmbeddedPrompts;

pub fn get_prompt_file(name: &str, base_dir: Option<&Path>) -> String {
    // 1. Try loading from filesystem if base_dir or local prompt_engineering exists
    if let Some(dir) = base_dir {
        let p = dir.join(name);
        if let Ok(content) = fs::read_to_string(&p) {
            return content;
        }
    }
    let local_path = Path::new("prompt_engineering").join(name);
    if let Ok(content) = fs::read_to_string(&local_path) {
        return content;
    }

    // 2. Fallback to embedded prompt assets
    if let Some(file) = EmbeddedPrompts::get(name) {
        if let Ok(content) = std::str::from_utf8(file.data.as_ref()) {
            return content.to_string();
        }
    }

    String::new()
}
