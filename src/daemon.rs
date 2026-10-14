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
    let mut watched: HashSet<String> = HashSet::new();
    for repository in &config.repositories {
        watcher
            .watch(&repository.path, RecursiveMode::Recursive)
            .with_context(|| format!("could not watch {}", repository.path.display()))?;
        watched.insert(repository.id.clone());
        if let Err(error) = startup_sync(repository) {
            eprintln!("{}: startup sync failed: {error:#}", repository.id);
        }
    }

    let mut config = config;
    let mut config_mtime = mtime(&paths.config_file);
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
        // Reload config.toml when it changes so folders added or removed
        // while the daemon runs take effect without a service restart.
        if mtime(&paths.config_file) != config_mtime {
            config_mtime = mtime(&paths.config_file);
            match Config::load(&paths) {
                Ok(new_config) => {
                    reconcile_watches(&mut watcher, &mut watched, &mut dirty, &config, &new_config);
                    config = new_config;
                }
                Err(error) => eprintln!("could not reload config: {error:#}"),
            }
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
                } else {
                    state.repository_mut(&repository.id).last_error = None;
                }
                state.repository_mut(&repository.id).last_remote_check_at = Some(Utc::now());
                state.save(&paths)?;
            }
        }
    }
}

fn mtime(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// After a config reload, start watching newly added repositories and stop
/// watching removed ones. New repositories also get a startup sync.
fn reconcile_watches(
    watcher: &mut RecommendedWatcher,
    watched: &mut HashSet<String>,
    dirty: &mut HashMap<String, Instant>,
    old_config: &Config,
    new_config: &Config,
) {
    for repository in &new_config.repositories {
        if watched.contains(&repository.id) {
            continue;
        }
        match watcher.watch(&repository.path, RecursiveMode::Recursive) {
            Ok(()) => {
                watched.insert(repository.id.clone());
                if let Err(error) = startup_sync(repository) {
                    eprintln!("{}: startup sync failed: {error:#}", repository.id);
                }
            }
            Err(error) => eprintln!(
                "{}: could not watch {}: {error:#}",
                repository.id,
                repository.path.display()
            ),
        }
    }
    for repository in &old_config.repositories {
        let still_present = new_config
            .repositories
            .iter()
            .any(|r| r.id == repository.id);
        if !still_present {
            let _ = watcher.unwatch(&repository.path);
            watched.remove(&repository.id);
            dirty.remove(&repository.id);
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
