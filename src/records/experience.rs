use crate::records::schema::ExperienceEntry;
use chrono::NaiveDateTime;
use regex::Regex;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub struct ExperienceReader {
    experience_dir: PathBuf,
}

impl ExperienceReader {
    pub fn new<P: AsRef<Path>>(experience_dir: P) -> Self {
        Self {
            experience_dir: experience_dir.as_ref().to_path_buf(),
        }
    }

    pub fn read_top5(&self, cycle_position: &str) -> Vec<ExperienceEntry> {
        let base_dir = self.experience_dir.join(cycle_position);
        let mut candidates: Vec<(i64, String, PathBuf)> = Vec::new();

        let ts_re = Regex::new(r"(\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2})").unwrap();

        for (case_type, subdir_name) in [("success", "success_cases"), ("failure", "failure_cases")] {
            let subdir = base_dir.join(subdir_name);
            if !subdir.is_dir() {
                continue;
            }
            if let Ok(entries) = fs::read_dir(subdir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("json") {
                        if let Some(fname) = path.file_name().and_then(|n| n.to_str()) {
                            if let Some(caps) = ts_re.captures(fname) {
                                if let Some(ts_str) = caps.get(1) {
                                    if let Ok(ndt) = NaiveDateTime::parse_from_str(ts_str.as_str(), "%Y-%m-%d_%H-%M-%S") {
                                        let ts_ms = ndt.and_utc().timestamp_millis();
                                        candidates.push((ts_ms, case_type.to_string(), path));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        candidates.sort_by(|a, b| b.0.cmp(&a.0));
        if candidates.len() > 5 {
            candidates.truncate(5);
        }

        let mut entries = Vec::new();
        for (ts_ms, case_type, path) in candidates {
            if let Ok(content_str) = fs::read_to_string(&path) {
                if let Ok(content_val) = serde_json::from_str::<Value>(&content_str) {
                    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                    entries.push(ExperienceEntry {
                        filename,
                        case_type,
                        cycle_position: cycle_position.to_string(),
                        timestamp_ms: ts_ms,
                        content: content_val,
                    });
                }
            }
        }

        entries
    }

    pub fn read_for_stage2(
        &self,
        cycle_position: &str,
        direction: &str,
        patterns: &[String],
        max_entries: usize,
    ) -> Vec<ExperienceEntry> {
        let entries = self.read_top5(cycle_position);
        if entries.is_empty() {
            return Vec::new();
        }

        let dir_norm = direction.trim().to_lowercase();
        let pattern_set: HashSet<String> = patterns.iter().map(|p| p.trim().to_lowercase()).collect();

        let mut scored: Vec<(i32, i64, ExperienceEntry)> = entries.into_iter().map(|entry| {
            let mut score = 0;
            if let Some(obj) = entry.content.as_object() {
                if let Some(ent_dir) = obj.get("direction").and_then(|v| v.as_str()) {
                    if !dir_norm.is_empty() && ent_dir.trim().to_lowercase() == dir_norm {
                        score += 2;
                    }
                }
                if let Some(arr) = obj.get("detected_patterns").and_then(|v| v.as_array()) {
                    for p in arr {
                        if let Some(p_str) = p.as_str() {
                            if pattern_set.contains(&p_str.trim().to_lowercase()) {
                                score += 1;
                            }
                        }
                    }
                }
            }
            (score, entry.timestamp_ms, entry)
        }).collect();

        scored.sort_by(|a, b| {
            b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1))
        });

        let cap = max_entries.min(10);
        scored.into_iter().take(cap).map(|(_, _, entry)| entry).collect()
    }
}
