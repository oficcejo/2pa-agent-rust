use std::path::PathBuf;

pub fn project_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub fn config_dir() -> PathBuf {
    project_root().join("config")
}

pub fn settings_json_path() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn prompt_dir() -> PathBuf {
    project_root().join("prompt_engineering")
}

pub fn experience_dir() -> PathBuf {
    project_root().join("experience")
}

pub fn records_dir() -> PathBuf {
    project_root().join("records")
}

pub fn pending_records_dir() -> PathBuf {
    records_dir().join("pending")
}

pub fn ensure_dirs() {
    let _ = std::fs::create_dir_all(config_dir());
    let _ = std::fs::create_dir_all(records_dir());
    let _ = std::fs::create_dir_all(pending_records_dir());
    let _ = std::fs::create_dir_all(experience_dir());
}
