use crate::config::atomic_write;
use crate::paths::AppPaths;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppState {
    pub repositories: BTreeMap<String, RepositoryState>,
    pub conflicts: BTreeMap<String, ConflictRecord>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RepositoryState {
    pub active_session: Option<SessionState>,
    pub last_snapshot_at: Option<DateTime<Utc>>,
    pub last_remote_check_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionState {
    pub id: String,
    pub device_id: String,
    pub branch: String,
    pub base_branch: String,
    pub base_commit: String,
    pub started_at: DateTime<Utc>,
    pub last_snapshot_commit: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictRecord {
    pub id: String,
    pub repository_id: String,
    pub base_commit: String,
    pub local_commit: String,
    pub remote_commit: String,
    pub paths: Vec<String>,
    pub detected_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolution: Option<String>,
}

impl AppState {
    pub fn load(paths: &AppPaths) -> Result<Self> {
        if !paths.state_file.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(&paths.state_file)
            .with_context(|| format!("could not read {}", paths.state_file.display()))?;
        toml::from_str(&text)
            .with_context(|| format!("invalid state in {}", paths.state_file.display()))
    }

    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        paths.ensure()?;
        let text = toml::to_string_pretty(self).context("could not serialize state")?;
        atomic_write(&paths.state_file, text.as_bytes())
    }

    pub fn repository_mut(&mut self, id: &str) -> &mut RepositoryState {
        self.repositories.entry(id.to_owned()).or_default()
    }

    pub fn unresolved_conflict_ids(&self) -> HashSet<String> {
        self.conflicts
            .values()
            .filter(|c| c.resolved_at.is_none())
            .map(|c| c.id.clone())
            .collect()
    }
}
