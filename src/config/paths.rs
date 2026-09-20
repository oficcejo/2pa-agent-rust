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

/// Resolved trade outcomes, one file per signal id.
pub fn outcomes_dir() -> PathBuf {
    records_dir().join("outcomes")
}

/// Versioned prompt artifacts.
pub fn prompt_artifacts_dir() -> PathBuf {
    prompt_dir().join("artifacts")
}

/// Historical market slice benchmark episodes for offline evaluation.
pub fn benchmark_episodes_dir() -> PathBuf {
    records_dir().join("benchmark_episodes")
}

/// Modular prompt engineering harness (core rules, skills, indicators).
pub fn harness_dir() -> PathBuf {
    prompt_dir().join("harness")
}

pub fn ensure_dirs() {
    let _ = std::fs::create_dir_all(config_dir());
    let _ = std::fs::create_dir_all(records_dir());
    let _ = std::fs::create_dir_all(pending_records_dir());
    let _ = std::fs::create_dir_all(experience_dir());
    let _ = std::fs::create_dir_all(outcomes_dir());
    let _ = std::fs::create_dir_all(prompt_artifacts_dir());
    let _ = std::fs::create_dir_all(benchmark_episodes_dir());
}
