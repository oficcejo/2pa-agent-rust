//! Versioned prompt artifacts.
//!
//! The strategy prompt used to be compiled into the binary with
//! `include_str!`, so changing a single threshold required a full rebuild and
//! there was no way to tell which revision produced a given decision.
//!
//! Artifacts fix both problems: every prompt is content-addressed, stored on
//! disk with an explicit version, and every decision records the version and
//! hash it ran against. Publishing a new version does not automatically make
//! it active — activation is a separate, auditable step so a candidate can be
//! evaluated before it ever reaches live traffic.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// A prompt revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptArtifact {
    pub name: String,
    pub version: String,
    pub hash: String,
    pub created_ms: i64,
    /// Free-form provenance note, e.g. "baseline seeded from binary".
    #[serde(default)]
    pub note: String,
}

/// On-disk index describing every published version and which one is active.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptIndex {
    #[serde(default)]
    pub active: String,
    #[serde(default)]
    pub versions: BTreeMap<String, PromptArtifact>,
}

/// Resolved active prompt ready to be injected into a request.
#[derive(Debug, Clone)]
pub struct ActivePrompt {
    pub name: String,
    pub version: String,
    pub hash: String,
    pub content: String,
}

pub fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())[..16].to_string()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct PromptArtifactStore {
    root: PathBuf,
}

impl PromptArtifactStore {
    /// `root` is the directory that holds one subdirectory per artifact name.
    pub fn new<P: AsRef<Path>>(root: P) -> Self {
        Self { root: root.as_ref().to_path_buf() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn name_dir(&self, name: &str) -> PathBuf {
        self.root.join(sanitise(name))
    }

    fn index_path(&self, name: &str) -> PathBuf {
        self.name_dir(name).join("index.json")
    }

    fn version_path(&self, name: &str, version: &str) -> PathBuf {
        self.name_dir(name).join(format!("{}.txt", sanitise(version)))
    }

    fn read_index(&self, name: &str) -> PromptIndex {
        fs::read_to_string(self.index_path(name))
            .ok()
            .and_then(|t| serde_json::from_str::<PromptIndex>(&t).ok())
            .unwrap_or_default()
    }

    fn write_index(&self, name: &str, index: &PromptIndex) -> Result<()> {
        let dir = self.name_dir(name);
        fs::create_dir_all(&dir)
            .with_context(|| format!("创建 prompt artifact 目录失败: {}", dir.display()))?;
        let text = serde_json::to_string_pretty(index)?;
        let path = self.index_path(name);
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Ensure an artifact exists, seeding version `v1` from the shipped binary
    /// content on first run. Returns the active prompt.
    pub fn ensure_seeded(&self, name: &str, builtin_content: &str) -> Result<ActivePrompt> {
        let mut index = self.read_index(name);
        if index.versions.is_empty() || index.active.is_empty() {
            let version = "v1".to_string();
            self.write_version(name, &version, builtin_content)?;
            index.versions.insert(
                version.clone(),
                PromptArtifact {
                    name: name.to_string(),
                    version: version.clone(),
                    hash: content_hash(builtin_content),
                    created_ms: now_ms(),
                    note: "baseline seeded from compiled default".to_string(),
                },
            );
            index.active = version;
            self.write_index(name, &index)?;
        }
        self.active(name)?.context("prompt artifact 初始化后仍无法读取激活版本")
    }

    fn write_version(&self, name: &str, version: &str, content: &str) -> Result<()> {
        let dir = self.name_dir(name);
        fs::create_dir_all(&dir)?;
        let path = self.version_path(name, version);
        let tmp = path.with_extension("txt.tmp");
        fs::write(&tmp, content)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Read the currently active prompt.
    pub fn active(&self, name: &str) -> Result<Option<ActivePrompt>> {
        let index = self.read_index(name);
        if index.active.is_empty() {
            return Ok(None);
        }
        let version = index.active.clone();
        let path = self.version_path(name, &version);
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Ok(None),
        };
        let hash = index
            .versions
            .get(&version)
            .map(|a| a.hash.clone())
            .unwrap_or_else(|| content_hash(&content));
        Ok(Some(ActivePrompt { name: name.to_string(), version, hash, content }))
    }

    pub fn index(&self, name: &str) -> PromptIndex {
        self.read_index(name)
    }

    /// Publish a candidate as a new, *inactive* version. Returns its version.
    ///
    /// Publishing never changes live behaviour; call `activate` explicitly.
    pub fn publish(&self, name: &str, content: &str, note: &str) -> Result<String> {
        let mut index = self.read_index(name);
        let hash = content_hash(content);

        // Content-addressed: republishing identical content returns the
        // existing version instead of creating a duplicate.
        if let Some(existing) = index.versions.values().find(|a| a.hash == hash) {
            return Ok(existing.version.clone());
        }

        let version = format!("v{}", index.versions.len() + 1);
        self.write_version(name, &version, content)?;
        index.versions.insert(
            version.clone(),
            PromptArtifact {
                name: name.to_string(),
                version: version.clone(),
                hash,
                created_ms: now_ms(),
                note: note.to_string(),
            },
        );
        self.write_index(name, &index)?;
        Ok(version)
    }

    /// Promote a published version to active. This is the only operation that
    /// changes live prompt behaviour.
    pub fn activate(&self, name: &str, version: &str) -> Result<ActivePrompt> {
        let mut index = self.read_index(name);
        anyhow::ensure!(
            index.versions.contains_key(version),
            "prompt 版本 {} 不存在，拒绝激活",
            version
        );
        index.active = version.to_string();
        self.write_index(name, &index)?;
        self.active(name)?.context("激活后无法读取 prompt")
    }

    /// Revert to the previously published version, if one exists.
    pub fn rollback(&self, name: &str) -> Result<Option<ActivePrompt>> {
        let index = self.read_index(name);
        let mut versions: Vec<&String> = index.versions.keys().collect();
        versions.sort();
        let Some(pos) = versions.iter().position(|v| **v == index.active) else {
            return Ok(None);
        };
        if pos == 0 {
            return Ok(None);
        }
        let prev = versions[pos - 1].clone();
        Ok(Some(self.activate(name, &prev)?))
    }

    pub fn content(&self, name: &str, version: &str) -> Option<String> {
        fs::read_to_string(self.version_path(name, version)).ok()
    }
}

fn sanitise(raw: &str) -> String {
    let safe: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if safe.is_empty() { "default".to_string() } else { safe }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("okx-artifact-{tag}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn seeding_creates_and_activates_v1() {
        let root = temp_root("seed");
        let store = PromptArtifactStore::new(&root);
        let active = store.ensure_seeded("strategy_v1", "PROMPT BODY").unwrap();
        assert_eq!(active.version, "v1");
        assert_eq!(active.content, "PROMPT BODY");
        assert_eq!(active.hash, content_hash("PROMPT BODY"));

        // Idempotent: a second call keeps v1.
        let again = store.ensure_seeded("strategy_v1", "PROMPT BODY").unwrap();
        assert_eq!(again.version, "v1");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn publish_does_not_change_active_prompt() {
        let root = temp_root("publish");
        let store = PromptArtifactStore::new(&root);
        store.ensure_seeded("s", "A").unwrap();

        let v2 = store.publish("s", "B", "candidate").unwrap();
        assert_eq!(v2, "v2");
        // Still serving A.
        assert_eq!(store.active("s").unwrap().unwrap().content, "A");

        store.activate("s", "v2").unwrap();
        assert_eq!(store.active("s").unwrap().unwrap().content, "B");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn publish_is_content_addressed() {
        let root = temp_root("dedupe");
        let store = PromptArtifactStore::new(&root);
        store.ensure_seeded("s", "A").unwrap();
        let v2 = store.publish("s", "B", "").unwrap();
        assert_eq!(v2, "v2");
        // Same body again resolves to the same version, no v3.
        assert_eq!(store.publish("s", "B", "").unwrap(), "v2");
        // Re-seeding baseline content resolves back to v1.
        assert_eq!(store.publish("s", "A", "").unwrap(), "v1");
        assert_eq!(store.index("s").versions.len(), 2);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn activate_rejects_unknown_version() {
        let root = temp_root("unknown");
        let store = PromptArtifactStore::new(&root);
        store.ensure_seeded("s", "A").unwrap();
        assert!(store.activate("s", "v99").is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rollback_returns_to_previous_version() {
        let root = temp_root("rollback");
        let store = PromptArtifactStore::new(&root);
        store.ensure_seeded("s", "A").unwrap();
        store.publish("s", "B", "").unwrap();
        store.activate("s", "v2").unwrap();
        let reverted = store.rollback("s").unwrap().unwrap();
        assert_eq!(reverted.version, "v1");
        assert_eq!(reverted.content, "A");
        // Already at the oldest version: nothing to roll back to.
        assert!(store.rollback("s").unwrap().is_none());
        let _ = fs::remove_dir_all(&root);
    }
}
