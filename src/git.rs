use anyhow::{Context, Result, bail};
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Command, Output};

pub const SYNC_AUTHOR_NAME: &str = "Cloud Sync";
pub const SYNC_AUTHOR_EMAIL: &str = "cloud-sync@localhost";

pub fn run<I, S>(repository: &Path, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = command(repository, args)?;
    if !output.status.success() {
        bail!(
            "git failed in {}: {}",
            repository.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub fn run_with_env<I, S>(repository: &Path, args: I, env: &[(&str, &str)]) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut process = Command::new("git");
    process.arg("-C").arg(repository).args(args);
    for (key, value) in env {
        process.env(key, value);
    }
    let output = process.output().context("could not execute git")?;
    if !output.status.success() {
        bail!(
            "git failed in {}: {}",
            repository.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub fn succeeds<I, S>(repository: &Path, args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    command(repository, args).is_ok_and(|output| output.status.success())
}

fn command<I, S>(repository: &Path, args: I) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .context("could not execute git")
}

pub fn is_repository(path: &Path) -> bool {
    succeeds(path, ["rev-parse", "--is-inside-work-tree"])
}

pub fn has_commits(path: &Path) -> bool {
    succeeds(path, ["rev-parse", "--verify", "HEAD"])
}

pub fn current_commit(path: &Path) -> Result<String> {
    run(path, ["rev-parse", "HEAD"])
}

pub fn current_branch(path: &Path) -> Result<String> {
    run(path, ["symbolic-ref", "--short", "HEAD"])
}

pub fn is_clean(path: &Path) -> Result<bool> {
    Ok(run(path, ["status", "--porcelain=v1"])?.is_empty())
}

pub fn remote_has_refs(remote: &str) -> Result<bool> {
    let output = Command::new("git")
        .args(["ls-remote", "--heads", remote])
        .output()
        .context("could not query remote repository")?;
    if !output.status.success() {
        bail!(
            "could not query remote: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(!output.stdout.is_empty())
}

pub fn remote_default_branch(repository: &Path, remote: &str) -> Result<String> {
    if let Ok(reference) = run(
        repository,
        ["symbolic-ref", &format!("refs/remotes/{remote}/HEAD")],
    ) && let Some(branch) = reference.rsplit('/').next()
    {
        return Ok(branch.to_owned());
    }
    for branch in ["main", "master"] {
        if succeeds(
            repository,
            [
                "show-ref",
                "--verify",
                &format!("refs/remotes/{remote}/{branch}"),
            ],
        ) {
            return Ok(branch.into());
        }
    }
    current_branch(repository).or_else(|_| Ok("main".into()))
}

pub fn add_paths(repository: &Path, paths: &[OsString], force: bool) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut args = vec![OsString::from("add")];
    if force {
        args.push(OsString::from("--force"));
    }
    args.push(OsString::from("--"));
    args.extend(paths.iter().cloned());
    run(repository, args)?;
    Ok(())
}

pub fn commit(repository: &Path, message: &str) -> Result<String> {
    run_with_env(
        repository,
        ["commit", "--no-gpg-sign", "-m", message],
        &[
            ("GIT_AUTHOR_NAME", SYNC_AUTHOR_NAME),
            ("GIT_AUTHOR_EMAIL", SYNC_AUTHOR_EMAIL),
            ("GIT_COMMITTER_NAME", SYNC_AUTHOR_NAME),
            ("GIT_COMMITTER_EMAIL", SYNC_AUTHOR_EMAIL),
        ],
    )?;
    current_commit(repository)
}
