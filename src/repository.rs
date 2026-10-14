use crate::config::{Config, RepositoryConfig};
use crate::filter::{Filter, FilterDecision, detect_presets};
use crate::git;
use crate::paths::AppPaths;
use anyhow::{Context, Result, bail};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;
use walkdir::WalkDir;

pub fn add(
    config: &mut Config,
    paths: &AppPaths,
    folder: &Path,
    remote: &str,
) -> Result<RepositoryConfig> {
    let folder = absolute(folder)?;
    if config.repositories.iter().any(|repo| repo.path == folder) {
        bail!("folder is already registered: {}", folder.display());
    }
    fs::create_dir_all(&folder)
        .with_context(|| format!("could not create {}", folder.display()))?;
    let remote_has_refs = git::remote_has_refs(remote)?;
    let folder_empty = fs::read_dir(&folder)?.next().is_none();

    if folder_empty && remote_has_refs {
        let output = Command::new("git")
            .args(["clone", remote])
            .arg(&folder)
            .output()
            .context("could not clone repository")?;
        if !output.status.success() {
            bail!(
                "git clone failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    } else if !git::is_repository(&folder) {
        git::run(&folder, ["init"])?;
        git::run(&folder, ["remote", "add", "origin", remote])?;
        if remote_has_refs {
            git::run(&folder, ["fetch", "origin"])?;
            bail!(
                "local files and the remote both contain content; repository initialized and fetched, but manual reconciliation is required"
            );
        }
    } else {
        configure_remote(&folder, remote)?;
    }

    if remote_has_refs {
        git::run(&folder, ["fetch", "origin", "--prune"])?;
    }
    let base_branch = if remote_has_refs {
        git::remote_default_branch(&folder, "origin")?
    } else {
        "main".into()
    };
    let repository = RepositoryConfig {
        id: unique_id(config, &folder),
        path: folder.clone(),
        remote: remote.into(),
        remote_name: "origin".into(),
        base_branch,
        presets: detect_presets(&folder),
        ..Default::default()
    };

    if !remote_has_refs && !git::has_commits(&folder) {
        stage_initial_files(&repository, config.defaults.skip_untracked_binaries)?;
        if !git::is_clean(&folder)? {
            git::commit(&folder, "cloud-sync: initial import")?;
            git::run(&folder, ["branch", "-M", &repository.base_branch])?;
            git::run(&folder, ["push", "-u", "origin", &repository.base_branch])?;
        }
    }

    config.repositories.push(repository.clone());
    config.save(paths)?;
    Ok(repository)
}

fn configure_remote(folder: &Path, remote: &str) -> Result<()> {
    match git::run(folder, ["remote", "get-url", "origin"]) {
        Ok(existing) if existing == remote => Ok(()),
        Ok(existing) => {
            bail!("existing origin points to {existing}; refusing to replace it with {remote}")
        }
        Err(_) => {
            git::run(folder, ["remote", "add", "origin", remote])?;
            Ok(())
        }
    }
}

fn stage_initial_files(repository: &RepositoryConfig, skip_binary: bool) -> Result<()> {
    let filter = Filter::from_repository(repository)?;
    let mut included: Vec<OsString> = Vec::new();
    for entry in WalkDir::new(&repository.path).follow_links(false) {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type().is_file() && !entry.file_type().is_symlink() {
            continue;
        }
        let relative = path.strip_prefix(&repository.path)?;
        if relative.starts_with(".git") {
            continue;
        }
        if matches!(
            filter.decision(&repository.path, relative, skip_binary)?,
            FilterDecision::Include
        ) && !git::succeeds(
            &repository.path,
            [
                "check-ignore",
                "-q",
                "--",
                relative.to_string_lossy().as_ref(),
            ],
        ) {
            included.push(relative.as_os_str().to_owned());
        }
    }
    git::add_paths(&repository.path, &included, false)?;
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
    git::add_paths(&repository.path, &forced, true)
}

fn unique_id(config: &Config, folder: &Path) -> String {
    let candidate = folder
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("repository");
    if config.repositories.iter().all(|repo| repo.id != candidate) {
        return candidate.into();
    }
    format!(
        "{}-{}",
        candidate,
        &Uuid::new_v4().simple().to_string()[..8]
    )
}

fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()?.join(path))
}
