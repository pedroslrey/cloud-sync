use crate::config::{Config, RepositoryConfig};
use crate::filter::{Filter, FilterDecision};
use crate::git::{self, SYNC_AUTHOR_EMAIL, SYNC_AUTHOR_NAME};
use crate::paths::AppPaths;
use crate::state::{AppState, ConflictRecord, SessionState};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use fs2::FileExt;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;
use walkdir::WalkDir;

pub fn snapshot(
    config: &Config,
    state: &mut AppState,
    paths: &AppPaths,
    repository: &RepositoryConfig,
) -> Result<Option<String>> {
    paths.ensure()?;
    let _lock = repository_lock(paths, &repository.id)?;
    ensure_base_checked_out(repository)?;
    git::run(
        &repository.path,
        ["fetch", &repository.remote_name, "--prune"],
    )?;
    reject_other_active_sessions(config, repository)?;

    if state
        .repositories
        .get(&repository.id)
        .and_then(|repo| repo.active_session.as_ref())
        .is_none()
    {
        let session = begin(config, repository)?;
        state.repository_mut(&repository.id).active_session = Some(session);
        state.save(paths)?;
    } else {
        roll_after_manual_commit(config, state, paths, repository)?;
    }

    let session = state
        .repositories
        .get(&repository.id)
        .and_then(|repo| repo.active_session.clone())
        .context("session state disappeared")?;
    let commit = create_snapshot(config, paths, repository, &session)?;
    if let Some(commit) = &commit {
        let remote_ref = format!("{}:refs/heads/{}", session.branch, session.branch);
        git::run(
            &repository.path,
            ["push", &repository.remote_name, &remote_ref],
        )?;
        let repository_state = state.repository_mut(&repository.id);
        repository_state.last_snapshot_at = Some(Utc::now());
        if let Some(active) = repository_state.active_session.as_mut() {
            active.last_snapshot_commit = Some(commit.clone());
        }
        repository_state.last_error = None;
        state.save(paths)?;
    }
    Ok(commit)
}

pub fn finalize(
    config: &Config,
    state: &mut AppState,
    paths: &AppPaths,
    repository: &RepositoryConfig,
) -> Result<Option<String>> {
    let _ = snapshot(config, state, paths, repository)?;
    let _lock = repository_lock(paths, &repository.id)?;
    let session = match state
        .repositories
        .get(&repository.id)
        .and_then(|repo| repo.active_session.clone())
    {
        Some(session) => session,
        None => return Ok(None),
    };
    let session_commit = session
        .last_snapshot_commit
        .clone()
        .unwrap_or_else(|| session.base_commit.clone());
    git::run(
        &repository.path,
        ["fetch", &repository.remote_name, "--prune"],
    )?;
    let local_base = git::run(
        &repository.path,
        ["rev-parse", &format!("refs/heads/{}", session.base_branch)],
    )?;
    let remote_ref = format!(
        "refs/remotes/{}/{}",
        repository.remote_name, session.base_branch
    );
    let remote_base = git::run(&repository.path, ["rev-parse", &remote_ref])
        .unwrap_or_else(|_| local_base.clone());
    if local_base != session.base_commit {
        bail!(
            "local base branch was rewritten during session; session preserved at {}",
            session.branch
        );
    }
    if !git::succeeds(
        &repository.path,
        [
            "merge-base",
            "--is-ancestor",
            &session.base_commit,
            &remote_base,
        ],
    ) {
        bail!(
            "remote base branch was rewritten during session; session preserved at {}",
            session.branch
        );
    }
    if session_commit == session.base_commit && remote_base == session.base_commit {
        close_session(state, paths, repository)?;
        return Ok(None);
    }
    let tree = if remote_base == session.base_commit {
        git::run(
            &repository.path,
            ["rev-parse", &format!("{session_commit}^{{tree}}")],
        )?
    } else if session_commit == session.base_commit {
        git::run(
            &repository.path,
            ["rev-parse", &format!("{remote_base}^{{tree}}")],
        )?
    } else {
        merge_tree_or_record(
            state,
            paths,
            repository,
            &session.base_commit,
            &session_commit,
            &remote_base,
        )?
    };
    let message = format!(
        "cloud-sync: session {} on {}",
        session.started_at.format("%Y-%m-%d %H:%M"),
        config.device_id
    );
    let final_commit = commit_tree(&repository.path, &tree, &remote_base, &message)?;
    if !git::succeeds(&repository.path, ["diff", "--quiet", &session_commit, "--"]) {
        bail!("tracked files changed during finalization; session preserved for another attempt");
    }
    git::run(
        &repository.path,
        [
            "update-ref",
            &format!("refs/heads/{}", session.base_branch),
            &final_commit,
            &local_base,
        ],
    )?;
    git::run(&repository.path, ["reset", "--hard", &session.base_branch])?;
    git::run(
        &repository.path,
        [
            "push",
            &repository.remote_name,
            &format!("{}:{}", session.base_branch, session.base_branch),
        ],
    )?;
    let completed_branch = format!("cloud-sync/completed/{}/{}", config.device_id, session.id);
    git::run(
        &repository.path,
        ["branch", &completed_branch, &session_commit],
    )?;
    let _ = git::run(
        &repository.path,
        [
            "push",
            &repository.remote_name,
            &format!("{completed_branch}:{completed_branch}"),
            &format!(":{}", session.branch),
        ],
    );
    let _ = git::run(&repository.path, ["branch", "-D", &session.branch]);
    close_session(state, paths, repository)?;
    let _ = cleanup_completed(config, repository);
    Ok(Some(final_commit))
}

fn merge_tree_or_record(
    state: &mut AppState,
    paths: &AppPaths,
    repository: &RepositoryConfig,
    base: &str,
    local: &str,
    remote: &str,
) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(&repository.path)
        .args(["merge-tree", "--write-tree", remote, local])
        .output()
        .context("could not execute git merge-tree")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if output.status.success() {
        return stdout
            .lines()
            .next()
            .map(str::to_owned)
            .context("git did not produce a reconciled tree");
    }
    let mut conflict_paths = stdout
        .lines()
        .filter_map(|line| {
            let (metadata, path) = line.split_once('\t')?;
            let mut fields = metadata.split_whitespace();
            let _mode = fields.next()?;
            let _object = fields.next()?;
            let stage = fields.next()?;
            matches!(stage, "1" | "2" | "3").then(|| path.to_owned())
        })
        .collect::<Vec<_>>();
    conflict_paths.sort();
    conflict_paths.dedup();
    let existing = state.conflicts.values().find(|conflict| {
        conflict.repository_id == repository.id
            && conflict.local_commit == local
            && conflict.remote_commit == remote
            && conflict.resolved_at.is_none()
    });
    let id = existing
        .map(|conflict| conflict.id.clone())
        .unwrap_or_else(|| {
            let id = Uuid::new_v4().simple().to_string();
            state.conflicts.insert(
                id.clone(),
                ConflictRecord {
                    id: id.clone(),
                    repository_id: repository.id.clone(),
                    base_commit: base.to_owned(),
                    local_commit: local.to_owned(),
                    remote_commit: remote.to_owned(),
                    paths: conflict_paths,
                    detected_at: Utc::now(),
                    resolved_at: None,
                    resolution: None,
                },
            );
            id
        });
    state.save(paths)?;
    bail!("merge conflict recorded as {id}; run cloud-sync conflict show {id}")
}

fn cleanup_completed(config: &Config, repository: &RepositoryConfig) -> Result<()> {
    let prefix = format!(
        "refs/remotes/{}/cloud-sync/completed/{}",
        repository.remote_name, config.device_id
    );
    let refs = git::run(
        &repository.path,
        [
            "for-each-ref",
            "--format=%(refname:short)|%(committerdate:unix)",
            &prefix,
        ],
    )?;
    let cutoff = Utc::now().timestamp()
        - (config
            .defaults
            .completed_session_retention_days
            .saturating_mul(86_400) as i64);
    let remote_prefix = format!("{}/", repository.remote_name);
    for line in refs.lines() {
        let Some((reference, timestamp)) = line.rsplit_once('|') else {
            continue;
        };
        if timestamp.parse::<i64>().is_ok_and(|value| value < cutoff)
            && let Some(branch) = reference.strip_prefix(&remote_prefix)
        {
            let _ = git::run(
                &repository.path,
                ["push", &repository.remote_name, &format!(":{branch}")],
            );
            let _ = git::run(&repository.path, ["branch", "-D", branch]);
        }
    }
    Ok(())
}

fn begin(config: &Config, repository: &RepositoryConfig) -> Result<SessionState> {
    let base_commit = git::run(
        &repository.path,
        [
            "rev-parse",
            &format!("refs/heads/{}", repository.base_branch),
        ],
    )?;
    let id = Uuid::new_v4().simple().to_string();
    let branch = format!("cloud-sync/active/{}/{}", config.device_id, id);
    git::run(&repository.path, ["branch", &branch, &base_commit])?;
    Ok(SessionState {
        id,
        device_id: config.device_id.clone(),
        branch,
        base_branch: repository.base_branch.clone(),
        base_commit,
        started_at: Utc::now(),
        last_snapshot_commit: None,
    })
}

fn roll_after_manual_commit(
    config: &Config,
    state: &mut AppState,
    paths: &AppPaths,
    repository: &RepositoryConfig,
) -> Result<()> {
    let session = state
        .repositories
        .get(&repository.id)
        .and_then(|repo| repo.active_session.clone())
        .context("session state disappeared")?;
    let current_base = git::run(
        &repository.path,
        [
            "rev-parse",
            &format!("refs/heads/{}", repository.base_branch),
        ],
    )?;
    if current_base == session.base_commit {
        return Ok(());
    }
    if !git::succeeds(
        &repository.path,
        [
            "merge-base",
            "--is-ancestor",
            &session.base_commit,
            &current_base,
        ],
    ) {
        bail!(
            "base branch was rewritten during an active session; session preserved at {}",
            session.branch
        );
    }
    if let Some(snapshot) = &session.last_snapshot_commit {
        let checkpoint = format!(
            "cloud-sync/completed/{}/{}-manual",
            config.device_id, session.id
        );
        git::run(&repository.path, ["branch", &checkpoint, snapshot])?;
        let _ = git::run(
            &repository.path,
            [
                "push",
                &repository.remote_name,
                &format!("{checkpoint}:{checkpoint}"),
                &format!(":{}", session.branch),
            ],
        );
    }
    let _ = git::run(&repository.path, ["branch", "-D", &session.branch]);
    state.repository_mut(&repository.id).active_session = Some(begin(config, repository)?);
    state.save(paths)
}

fn create_snapshot(
    config: &Config,
    paths: &AppPaths,
    repository: &RepositoryConfig,
    session: &SessionState,
) -> Result<Option<String>> {
    let parent = session
        .last_snapshot_commit
        .as_ref()
        .unwrap_or(&session.base_commit);
    let index = paths.state_dir.join(format!("index-{}", repository.id));
    if index.exists() {
        fs::remove_file(&index).with_context(|| format!("could not reset {}", index.display()))?;
    }
    let index_value = index.to_string_lossy().to_string();
    git::run_with_env(
        &repository.path,
        ["read-tree", parent],
        &[("GIT_INDEX_FILE", &index_value)],
    )?;
    git::run_with_env(
        &repository.path,
        ["add", "-u", "--", "."],
        &[("GIT_INDEX_FILE", &index_value)],
    )?;
    stage_untracked(config, repository, &index_value)?;
    let tree = git::run_with_env(
        &repository.path,
        ["write-tree"],
        &[("GIT_INDEX_FILE", &index_value)],
    )?;
    let parent_tree = git::run(
        &repository.path,
        ["rev-parse", &format!("{parent}^{{tree}}")],
    )?;
    let _ = fs::remove_file(index);
    if tree == parent_tree {
        return Ok(None);
    }
    let message = format!(
        "cloud-sync snapshot: {} {}",
        config.device_id,
        Utc::now().to_rfc3339()
    );
    let commit = commit_tree(&repository.path, &tree, parent, &message)?;
    git::run(
        &repository.path,
        [
            "update-ref",
            &format!("refs/heads/{}", session.branch),
            &commit,
            parent,
        ],
    )?;
    Ok(Some(commit))
}

fn stage_untracked(config: &Config, repository: &RepositoryConfig, index: &str) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(&repository.path)
        .args(["ls-files", "--others", "--exclude-standard", "-z"])
        .output()
        .context("could not enumerate untracked files")?;
    if !output.status.success() {
        bail!(
            "could not enumerate untracked files: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let filter = Filter::from_repository(repository)?;
    let mut paths = Vec::new();
    for bytes in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
    {
        let relative = PathBuf::from(String::from_utf8_lossy(bytes).into_owned());
        if matches!(
            filter.decision(
                &repository.path,
                &relative,
                config.defaults.skip_untracked_binaries
            )?,
            FilterDecision::Include
        ) {
            paths.push(relative.into_os_string());
        }
    }
    stage_with_index(&repository.path, index, &paths, false)?;
    let mut forced = Vec::new();
    for entry in WalkDir::new(&repository.path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| entry.file_name() != ".git")
    {
        let entry = entry?;
        if !entry.file_type().is_file() && !entry.file_type().is_symlink() {
            continue;
        }
        let relative = entry.path().strip_prefix(&repository.path)?;
        if filter.explicitly_includes(relative) {
            forced.push(relative.as_os_str().to_owned());
        }
    }
    stage_with_index(&repository.path, index, &forced, true)
}

fn stage_with_index(repository: &Path, index: &str, paths: &[OsString], force: bool) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repository)
        .arg("add")
        .env("GIT_INDEX_FILE", index);
    if force {
        command.arg("--force");
    }
    let output = command
        .arg("--")
        .args(paths)
        .output()
        .context("could not stage snapshot files")?;
    if !output.status.success() {
        bail!(
            "could not stage snapshot files: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn commit_tree(repository: &Path, tree: &str, parent: &str, message: &str) -> Result<String> {
    git::run_with_env(
        repository,
        ["commit-tree", tree, "-p", parent, "-m", message],
        &[
            ("GIT_AUTHOR_NAME", SYNC_AUTHOR_NAME),
            ("GIT_AUTHOR_EMAIL", SYNC_AUTHOR_EMAIL),
            ("GIT_COMMITTER_NAME", SYNC_AUTHOR_NAME),
            ("GIT_COMMITTER_EMAIL", SYNC_AUTHOR_EMAIL),
        ],
    )
}

fn reject_other_active_sessions(config: &Config, repository: &RepositoryConfig) -> Result<()> {
    let refs = git::run(
        &repository.path,
        [
            "for-each-ref",
            "--format=%(refname:short)",
            &format!("refs/remotes/{}/cloud-sync/active", repository.remote_name),
        ],
    )?;
    let own_prefix = format!(
        "{}/cloud-sync/active/{}/",
        repository.remote_name, config.device_id
    );
    if let Some(other) = refs
        .lines()
        .find(|reference| !reference.starts_with(&own_prefix))
    {
        bail!("unfinished session from another device exists: {other}");
    }
    Ok(())
}

fn ensure_base_checked_out(repository: &RepositoryConfig) -> Result<()> {
    let branch = git::current_branch(&repository.path)?;
    if branch != repository.base_branch {
        bail!(
            "repository {} is on branch {branch}; expected {}",
            repository.id,
            repository.base_branch
        );
    }
    Ok(())
}

fn repository_lock(paths: &AppPaths, id: &str) -> Result<File> {
    let lock_path = paths.locks_dir.join(format!("{id}.lock"));
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    file.try_lock_exclusive()
        .with_context(|| format!("repository {id} is already syncing"))?;
    Ok(file)
}

fn close_session(
    state: &mut AppState,
    paths: &AppPaths,
    repository: &RepositoryConfig,
) -> Result<()> {
    state.repository_mut(&repository.id).active_session = None;
    state.save(paths)
}
