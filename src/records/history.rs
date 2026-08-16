use crate::records::schema::AnalysisRecord;
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

pub fn list_record_paths(dir: &Path) -> Vec<PathBuf> {
    if !dir.is_dir() {
        return Vec::new();
    }
    let mut paths = Vec::new();

    // Check root of dir
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("json") {
                paths.push(path);
            }
        }
    }

    // Also check pending/ subfolder if exists
    let pending_dir = dir.join("pending");
    if pending_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&pending_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("json") {
                    paths.push(path);
                }
            }
        }
    }

    paths.sort_by(|a, b| {
        let ma = fs::metadata(a).and_then(|m| m.modified()).ok();
        let mb = fs::metadata(b).and_then(|m| m.modified()).ok();
        mb.cmp(&ma)
    });
    paths.dedup();
    paths
}

pub fn load_record(path: &Path) -> Option<AnalysisRecord> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn save_record(dir: &Path, record: &AnalysisRecord) -> Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let filename = format!("{}_{}_{}.json",
        record.meta.timestamp_local_iso.replace(':', "-"),
        record.meta.symbol.replace('/', "-"),
        record.meta.timeframe,
    );
    let path = dir.join(filename);
    let json_text = serde_json::to_string_pretty(record)?;
    fs::write(&path, json_text)?;
    Ok(path)
}

pub fn delete_record(dir: &Path, record_id: &str) -> bool {
    let target = format!("{}.json", record_id.trim_end_matches(".json"));
    let path1 = dir.join(&target);
    let path2 = dir.join("pending").join(&target);
    let mut deleted = false;
    if path1.is_file() {
        deleted |= fs::remove_file(path1).is_ok();
    }
    if path2.is_file() {
        deleted |= fs::remove_file(path2).is_ok();
    }
    deleted
}
