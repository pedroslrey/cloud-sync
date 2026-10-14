use crate::config::{Config, RepositoryConfig};
use crate::git;
use crate::paths::AppPaths;
use crate::session;
use crate::state::AppState;
use anyhow::{Result, bail};
use chrono::Utc;
use uuid::Uuid;

pub fn history(repository: &RepositoryConfig, file: Option<&str>, limit: usize) -> Result<String> {
    let limit = limit.clamp(1, 500).to_string();
    let mut args = vec![
        "log",
        "--date=short",
        "--pretty=format:%h  %ad  %an  %s",
        "-n",
        &limit,
    ];
    if let Some(file) = file {
        args.extend(["--", file]);
    }
    git::run(&repository.path, args)
}

pub fn show(repository: &RepositoryConfig, revision: &str, file: Option<&str>) -> Result<String> {
    validate_revision(revision)?;
    match file {
        Some(file) => git::run(&repository.path, ["show", &format!("{revision}:{file}")]),
        None => git::run(
            &repository.path,
            ["show", "--stat", "--summary", "--decorate", revision],
        ),
    }
}

pub fn restore(
    config: &Config,
    state: &mut AppState,
    paths: &AppPaths,
    repository: &RepositoryConfig,
    revision: &str,
    file: Option<&str>,
) -> Result<String> {
    validate_revision(revision)?;
    let recovery = create_recovery(config, state, paths, repository)?;
    match file {
        Some(file) => {
            git::run(
                &repository.path,
                ["restore", "--source", revision, "--", file],
            )?;
        }
        None => {
            git::run(
                &repository.path,
                [
                    "restore",
                    "--source",
                    revision,
                    "--staged",
                    "--worktree",
                    "--",
                    ".",
                ],
            )?;
        }
    }
    Ok(recovery)
}

pub fn list_backups(repository: &RepositoryConfig) -> Result<String> {
    git::run(
        &repository.path,
        [
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname:short)  %(committerdate:iso8601)  %(subject)",
            "refs/heads/cloud-sync",
            &format!("refs/remotes/{}/cloud-sync", repository.remote_name),
        ],
    )
}

pub fn show_backup(repository: &RepositoryConfig, reference: &str) -> Result<String> {
    validate_revision(reference)?;
    git::run(
        &repository.path,
        ["show", "--stat", "--summary", "--decorate", reference],
    )
}

fn create_recovery(
    config: &Config,
    state: &mut AppState,
    paths: &AppPaths,
    repository: &RepositoryConfig,
) -> Result<String> {
    let _ = session::snapshot(config, state, paths, repository)?;
    let commit = state
        .repositories
        .get(&repository.id)
        .and_then(|repo| repo.active_session.as_ref())
        .and_then(|session| session.last_snapshot_commit.as_ref())
        .cloned()
        .unwrap_or(git::current_commit(&repository.path)?);
    let reference = format!(
        "cloud-sync/recovery/{}/{}-{}",
        config.device_id,
        Utc::now().format("%Y%m%d-%H%M%S"),
        &Uuid::new_v4().simple().to_string()[..8]
    );
    git::run(&repository.path, ["branch", &reference, &commit])?;
    git::run(
        &repository.path,
        [
            "push",
            &repository.remote_name,
            &format!("{reference}:{reference}"),
        ],
    )?;
    Ok(reference)
}

fn validate_revision(revision: &str) -> Result<()> {
    if revision.is_empty() || revision.starts_with('-') {
        bail!("invalid revision: {revision}");
    }
    Ok(())
}
