use crate::config::Config;
use crate::paths::AppPaths;
use crate::state::AppState;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;

/// Machine-readable status summary for widgets and the GUI.
#[derive(Clone, Debug, Serialize)]
pub struct StatusSummary {
    pub state: SyncState,
    pub repositories: usize,
    pub syncing: usize,
    pub conflicts: usize,
    pub errors: usize,
    pub last_sync: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncState {
    UpToDate,
    Syncing,
    Conflicts,
    Error,
}

pub fn summary(config: &Config, paths: &AppPaths) -> Result<StatusSummary> {
    let state = AppState::load(paths)?;

    let repositories = config.repositories.len();
    // Only count a repo as syncing while its lock is actually held by the
    // daemon or a manual sync — an open session just means "watching".
    let syncing = state
        .repositories
        .keys()
        .filter(|id| is_locked(paths, id))
        .count();
    let conflicts = state
        .conflicts
        .values()
        .filter(|c| c.resolved_at.is_none())
        .count();
    let errors = state
        .repositories
        .values()
        .filter(|r| r.last_error.is_some())
        .count();
    let last_sync = state
        .repositories
        .values()
        .filter_map(|r| r.last_snapshot_at)
        .max();

    let overall = if errors > 0 {
        SyncState::Error
    } else if conflicts > 0 {
        SyncState::Conflicts
    } else if syncing > 0 {
        SyncState::Syncing
    } else {
        SyncState::UpToDate
    };

    Ok(StatusSummary {
        state: overall,
        repositories,
        syncing,
        conflicts,
        errors,
        last_sync,
    })
}

/// True while a snapshot holds the repository's lock file.
fn is_locked(paths: &AppPaths, repo_id: &str) -> bool {
    let lock_path = paths.locks_dir.join(format!("{repo_id}.lock"));
    let Ok(file) = std::fs::File::open(&lock_path) else {
        return false;
    };
    file.try_lock_shared().is_err()
}
