use crate::config::{Config, RepositoryConfig};
use crate::git;
use crate::paths::AppPaths;
use crate::session;
use crate::state::AppState;
use anyhow::{Context, Result};
use chrono::Utc;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

pub fn run(config: Config, paths: AppPaths) -> Result<()> {
    let (sender, receiver) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = RecommendedWatcher::new(
        move |event| {
            let _ = sender.send(event);
        },
        notify::Config::default(),
    )?;
    for repository in &config.repositories {
        watcher
            .watch(&repository.path, RecursiveMode::Recursive)
            .with_context(|| format!("could not watch {}", repository.path.display()))?;
        startup_sync(repository)?;
    }

    let mut state = AppState::load(&paths)?;
    let mut notified_conflicts: HashSet<String> = HashSet::new();
    // Notify about any pre-existing unresolved conflicts on startup.
    notify_new_conflicts(&state, &mut notified_conflicts);
    let mut dirty: HashMap<String, Instant> = HashMap::new();
    loop {
        match receiver.recv_timeout(Duration::from_secs(1)) {
            Ok(Ok(event)) => mark_dirty(&config, &event, &mut dirty),
            Ok(Err(error)) => eprintln!("watch error: {error}"),
            Err(RecvTimeoutError::Disconnected) => anyhow::bail!("filesystem watcher stopped"),
            Err(RecvTimeoutError::Timeout) => {}
        }
        for repository in &config.repositories {
            if dirty.get(&repository.id).is_some_and(|changed| {
                changed.elapsed()
                    >= Duration::from_secs(
                        repository
                            .inactivity_seconds
                            .unwrap_or(config.defaults.inactivity_seconds),
                    )
            }) {
                match session::snapshot(&config, &mut state, &paths, repository) {
                    Ok(_) => {
                        dirty.remove(&repository.id);
                        notify_new_conflicts(&state, &mut notified_conflicts);
                    }
                    Err(error) => {
                        state.repository_mut(&repository.id).last_error = Some(error.to_string());
                        state.save(&paths)?;
                        eprintln!("{}: {error:#}", repository.id);
                    }
                }
            }
            let interval = repository
                .remote_check_seconds
                .unwrap_or(config.defaults.remote_check_seconds);
            let due = state
                .repositories
                .get(&repository.id)
                .and_then(|repo| repo.last_remote_check_at)
                .is_none_or(|last| (Utc::now() - last).num_seconds() >= interval as i64);
            if due {
                if let Err(error) = remote_check(repository, &state) {
                    state.repository_mut(&repository.id).last_error = Some(error.to_string());
                }
                state.repository_mut(&repository.id).last_remote_check_at = Some(Utc::now());
                state.save(&paths)?;
            }
        }
    }
}

fn mark_dirty(config: &Config, event: &Event, dirty: &mut HashMap<String, Instant>) {
    for repository in &config.repositories {
        if event
            .paths
            .iter()
            .any(|path| relevant(&repository.path, path))
        {
            dirty.insert(repository.id.clone(), Instant::now());
        }
    }
}

fn relevant(root: &Path, path: &Path) -> bool {
    path.starts_with(root)
        && !path
            .strip_prefix(root)
            .is_ok_and(|relative| relative.starts_with(".git"))
}

fn startup_sync(repository: &RepositoryConfig) -> Result<()> {
    git::run(
        &repository.path,
        ["fetch", &repository.remote_name, "--prune"],
    )?;
    if git::is_clean(&repository.path)? {
        let remote = format!("{}/{}", repository.remote_name, repository.base_branch);
        git::run(&repository.path, ["merge", "--ff-only", &remote])?;
    }
    Ok(())
}

fn remote_check(repository: &RepositoryConfig, state: &AppState) -> Result<()> {
    git::run(
        &repository.path,
        ["fetch", &repository.remote_name, "--prune"],
    )?;
    if state
        .repositories
        .get(&repository.id)
        .and_then(|repo| repo.active_session.as_ref())
        .is_none()
        && git::is_clean(&repository.path)?
    {
        let remote = format!("{}/{}", repository.remote_name, repository.base_branch);
        git::run(&repository.path, ["merge", "--ff-only", &remote])?;
    }
    Ok(())
}

/// Send a single desktop notification if there are unresolved conflicts that
/// haven't been notified yet during this daemon session.
fn notify_new_conflicts(state: &AppState, notified: &mut HashSet<String>) {
    let unresolved = state.unresolved_conflict_ids();
    let new: Vec<_> = unresolved.difference(notified).collect();
    if new.is_empty() {
        return;
    }
    let count = unresolved.len();
    let body = if count == 1 {
        "1 unresolved conflict. Run: cloud-sync conflicts".into()
    } else {
        format!("{count} unresolved conflicts. Run: cloud-sync conflicts")
    };
    let _ = Command::new("notify-send")
        .args(["Cloud Sync", &body])
        .status();
    // Mark all current unresolved conflicts as notified so we don't repeat.
    notified.extend(unresolved);
}
