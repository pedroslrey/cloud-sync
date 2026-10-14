use crate::config::{Config, RepositoryConfig};
use crate::git::{self, SYNC_AUTHOR_EMAIL, SYNC_AUTHOR_NAME};
use crate::paths::AppPaths;
use crate::state::{AppState, ConflictRecord};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Copy, Debug)]
pub enum Resolution {
    Local,
    Remote,
    Newest,
    KeepBoth,
}

pub fn list(state: &AppState, repository: Option<&str>) -> String {
    let conflicts: Vec<_> = state
        .conflicts
        .values()
        .filter(|conflict| conflict.resolved_at.is_none())
        .filter(|conflict| repository.is_none_or(|id| id == conflict.repository_id))
        .collect();
    if conflicts.is_empty() {
        return String::new();
    }
    conflicts
        .iter()
        .map(|conflict| {
            let files = if conflict.paths.is_empty() {
                "  unknown paths".into()
            } else {
                conflict
                    .paths
                    .iter()
                    .map(|p| format!("  {p}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            format!(
                "{} ({})\n  detected: {}\n{}\n  resolve: cloud-sync conflict resolve {} <local|remote|newest|keep-both>",
                conflict.id,
                conflict.repository_id,
                conflict.detected_at.format("%Y-%m-%d %H:%M"),
                files,
                conflict.id,
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn show(config: &Config, state: &AppState, id: &str) -> Result<String> {
    let conflict = state
        .conflicts
        .get(id)
        .with_context(|| format!("unknown conflict: {id}"))?;
    let repository = config.repository(&conflict.repository_id)?;
    let paths_display = if conflict.paths.is_empty() {
        "unknown".to_owned()
    } else {
        conflict
            .paths
            .iter()
            .map(|p| format!("  {p}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let status = if conflict.resolved_at.is_some() {
        format!(
            "resolved ({})",
            conflict.resolution.as_deref().unwrap_or("unknown")
        )
    } else {
        "unresolved — remote version is active on disk".into()
    };
    let mut output = format!(
        "Conflict {id}\n\
         Repository: {}\n\
         Status: {status}\n\
         Detected: {}\n\
         Conflicting files:\n\
         {paths_display}\n",
        conflict.repository_id,
        conflict.detected_at.format("%Y-%m-%d %H:%M"),
    );
    let mut args = vec![
        "diff",
        "--no-ext-diff",
        "--no-color",
        "--stat",
        &conflict.remote_commit,
        &conflict.local_commit,
    ];
    if !conflict.paths.is_empty() {
        args.push("--");
        args.extend(conflict.paths.iter().map(String::as_str));
    }
    let stat = git::run(&repository.path, args.clone()).unwrap_or_default();
    if !stat.is_empty() {
        output.push_str("\nChanges (remote -> local):\n");
        output.push_str(&stat);
        output.push('\n');
    }
    // Full diff
    args[3] = "--";
    let mut diff_args = vec![
        "diff",
        "--no-ext-diff",
        "--no-color",
        &conflict.remote_commit,
        &conflict.local_commit,
    ];
    if !conflict.paths.is_empty() {
        diff_args.push("--");
        diff_args.extend(conflict.paths.iter().map(String::as_str));
    }
    let diff = git::run(&repository.path, diff_args).unwrap_or_default();
    if !diff.is_empty() {
        output.push_str("\nDiff:\n");
        output.push_str(&diff);
        output.push('\n');
    }
    if conflict.resolved_at.is_none() {
        output.push_str(&format!(
            "\nTo resolve, run one of:\n\
             \n  cloud-sync conflict resolve {id} local     # use your version\
             \n  cloud-sync conflict resolve {id} remote    # keep the current (remote) version\
             \n  cloud-sync conflict resolve {id} newest    # use whichever was edited last\
             \n  cloud-sync conflict resolve {id} keep-both # keep remote, save local as .cloud-sync-local\n"
        ));
    }
    Ok(output)
}

pub fn resolve(
    config: &Config,
    state: &mut AppState,
    paths: &AppPaths,
    id: &str,
    resolution: Resolution,
) -> Result<String> {
    let conflict = state
        .conflicts
        .get(id)
        .cloned()
        .with_context(|| format!("unknown conflict: {id}"))?;
    if conflict.resolved_at.is_some() {
        bail!("conflict {id} is already resolved");
    }
    let repository = config.repository(&conflict.repository_id)?.clone();
    git::run(
        &repository.path,
        ["fetch", &repository.remote_name, "--prune"],
    )?;
    // The base branch may have advanced past the original conflict commits
    // because finalization now succeeds with deferred conflicts. We resolve
    // on top of the current base head.
    let current_base = git::run(
        &repository.path,
        [
            "rev-parse",
            &format!("refs/heads/{}", repository.base_branch),
        ],
    )?;
    let selected = match resolution {
        Resolution::Newest => {
            if commit_time(&repository, &conflict.local_commit)?
                >= commit_time(&repository, &conflict.remote_commit)?
            {
                Resolution::Local
            } else {
                Resolution::Remote
            }
        }
        other => other,
    };
    // If choosing remote, the base branch already contains the remote version
    // from the deferred finalization — nothing to change on disk.
    if matches!(selected, Resolution::Remote) {
        let record = state
            .conflicts
            .get_mut(id)
            .context("conflict state disappeared")?;
        record.resolved_at = Some(Utc::now());
        record.resolution = Some(resolution_name(resolution).into());
        state.save(paths)?;
        return Ok(current_base);
    }
    let recovery = format!(
        "cloud-sync/recovery/{}/conflict-{}-local",
        config.device_id, id
    );
    if !git::succeeds(
        &repository.path,
        ["show-ref", "--verify", &format!("refs/heads/{recovery}")],
    ) {
        git::run(
            &repository.path,
            ["branch", &recovery, &conflict.local_commit],
        )?;
        let _ = git::run(
            &repository.path,
            [
                "push",
                &repository.remote_name,
                &format!("{recovery}:{recovery}"),
            ],
        );
    }
    let tree = resolved_tree(paths, &repository, &conflict, selected)?;
    let message = format!(
        "cloud-sync: resolve conflict {id} using {}",
        resolution_name(resolution)
    );
    let commit = git::run_with_env(
        &repository.path,
        ["commit-tree", &tree, "-p", &current_base, "-m", &message],
        &[
            ("GIT_AUTHOR_NAME", SYNC_AUTHOR_NAME),
            ("GIT_AUTHOR_EMAIL", SYNC_AUTHOR_EMAIL),
            ("GIT_COMMITTER_NAME", SYNC_AUTHOR_NAME),
            ("GIT_COMMITTER_EMAIL", SYNC_AUTHOR_EMAIL),
        ],
    )?;
    git::run(
        &repository.path,
        [
            "update-ref",
            &format!("refs/heads/{}", repository.base_branch),
            &commit,
            &current_base,
        ],
    )?;
    git::run(&repository.path, ["reset", "--hard", &commit])?;
    if matches!(resolution, Resolution::KeepBoth) {
        materialize_local_copies(&repository, &conflict)?;
    }
    git::run(
        &repository.path,
        [
            "push",
            &repository.remote_name,
            &format!("{}:{}", repository.base_branch, repository.base_branch),
        ],
    )?;
    if let Some(session) = state.repository_mut(&repository.id).active_session.take() {
        let _ = git::run(
            &repository.path,
            [
                "push",
                &repository.remote_name,
                &format!(":{}", session.branch),
            ],
        );
        let _ = git::run(&repository.path, ["branch", "-D", &session.branch]);
    }
    let record = state
        .conflicts
        .get_mut(id)
        .context("conflict state disappeared")?;
    record.resolved_at = Some(Utc::now());
    record.resolution = Some(resolution_name(resolution).into());
    state.save(paths)?;
    Ok(commit)
}

fn resolved_tree(
    paths: &AppPaths,
    repository: &RepositoryConfig,
    conflict: &ConflictRecord,
    resolution: Resolution,
) -> Result<String> {
    let index = paths
        .state_dir
        .join(format!("conflict-index-{}", conflict.id));
    let _ = fs::remove_file(&index);
    let index_value = index.to_string_lossy().to_string();
    git::run_with_env(
        &repository.path,
        [
            "read-tree",
            "-m",
            &conflict.base_commit,
            &conflict.remote_commit,
            &conflict.local_commit,
        ],
        &[("GIT_INDEX_FILE", &index_value)],
    )?;
    let unmerged = git::run_with_env(
        &repository.path,
        ["ls-files", "-u"],
        &[("GIT_INDEX_FILE", &index_value)],
    )?;
    let mut conflict_paths = unmerged
        .lines()
        .filter_map(|line| line.split_once('\t').map(|(_, path)| path.to_owned()))
        .collect::<Vec<_>>();
    conflict_paths.sort();
    conflict_paths.dedup();
    let stage = if matches!(resolution, Resolution::Local) {
        "3"
    } else {
        "2"
    };
    for path in conflict_paths {
        let entries = git::run_with_env(
            &repository.path,
            ["ls-files", "--stage", "--", &path],
            &[("GIT_INDEX_FILE", &index_value)],
        )?;
        let selected = entries.lines().find_map(|line| {
            let (metadata, _) = line.split_once('\t')?;
            let mut fields = metadata.split_whitespace();
            let mode = fields.next()?;
            let object = fields.next()?;
            let entry_stage = fields.next()?;
            (entry_stage == stage).then(|| (mode.to_owned(), object.to_owned()))
        });
        if let Some((mode, object)) = selected {
            git::run_with_env(
                &repository.path,
                [
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    &mode,
                    &object,
                    &path,
                ],
                &[("GIT_INDEX_FILE", &index_value)],
            )?;
        } else {
            git::run_with_env(
                &repository.path,
                ["update-index", "--force-remove", "--", &path],
                &[("GIT_INDEX_FILE", &index_value)],
            )?;
        }
    }
    let tree = git::run_with_env(
        &repository.path,
        ["write-tree"],
        &[("GIT_INDEX_FILE", &index_value)],
    )?;
    let _ = fs::remove_file(index);
    Ok(tree)
}

fn materialize_local_copies(
    repository: &RepositoryConfig,
    conflict: &ConflictRecord,
) -> Result<()> {
    for path in &conflict.paths {
        let output = Command::new("git")
            .arg("-C")
            .arg(&repository.path)
            .args(["show", &format!("{}:{path}", conflict.local_commit)])
            .output()?;
        if !output.status.success() {
            continue;
        }
        let destination = repository
            .path
            .join(PathBuf::from(format!("{path}.cloud-sync-local")));
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(destination, output.stdout)?;
    }
    Ok(())
}

fn commit_time(repository: &RepositoryConfig, commit: &str) -> Result<i64> {
    Ok(git::run(&repository.path, ["show", "-s", "--format=%ct", commit])?.parse()?)
}

fn resolution_name(resolution: Resolution) -> &'static str {
    match resolution {
        Resolution::Local => "local",
        Resolution::Remote => "remote",
        Resolution::Newest => "newest",
        Resolution::KeepBoth => "keep-both",
    }
}
