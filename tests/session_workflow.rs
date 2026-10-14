use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use uuid::Uuid;

#[test]
fn snapshots_stay_off_base_until_finalized() {
    let root = std::env::temp_dir().join(format!("cloud-sync-e2e-{}", Uuid::new_v4()));
    let config = root.join("config");
    let state = root.join("state");
    let remote = root.join("remote.git");
    let work = root.join("work");
    fs::create_dir_all(&work).unwrap();
    assert!(
        git(&root, ["init", "--bare", remote.to_str().unwrap()])
            .status
            .success()
    );
    fs::write(work.join("README"), "first\n").unwrap();
    fs::write(work.join("artifact.bin"), b"a\0b").unwrap();

    assert!(
        cloud_sync(
            &config,
            &state,
            ["add", work.to_str().unwrap(), remote.to_str().unwrap()]
        )
        .status
        .success()
    );
    let base_before = git_text(&work, ["rev-parse", "main"]);
    assert!(
        !git(&work, ["ls-files", "--error-unmatch", "artifact.bin"])
            .status
            .success()
    );

    fs::write(work.join("README"), "first\nsecond\n").unwrap();
    assert!(
        cloud_sync(&config, &state, ["sync", "work"])
            .status
            .success()
    );
    assert_eq!(base_before, git_text(&work, ["rev-parse", "main"]));
    assert!(
        !git_text(
            &remote,
            [
                "for-each-ref",
                "--format=%(refname)",
                "refs/heads/cloud-sync/active"
            ]
        )
        .is_empty()
    );

    let other = root.join("other");
    assert!(
        git(
            &root,
            ["clone", remote.to_str().unwrap(), other.to_str().unwrap()]
        )
        .status
        .success()
    );
    assert!(git(&other, ["checkout", "main"]).status.success());
    fs::write(other.join("remote.txt"), "remote\n").unwrap();
    assert!(git(&other, ["add", "remote.txt"]).status.success());
    assert!(
        git(
            &other,
            [
                "-c",
                "user.name=Other",
                "-c",
                "user.email=other@localhost",
                "commit",
                "-m",
                "remote change"
            ]
        )
        .status
        .success()
    );
    assert!(git(&other, ["push", "origin", "main"]).status.success());

    assert!(
        cloud_sync(&config, &state, ["finalize", "work"])
            .status
            .success()
    );
    assert_ne!(base_before, git_text(&work, ["rev-parse", "main"]));
    assert_eq!(
        git_text(&work, ["status", "--porcelain"]),
        "?? artifact.bin"
    );
    assert_eq!(git_text(&remote, ["show", "main:README"]), "first\nsecond");
    assert_eq!(git_text(&remote, ["show", "main:remote.txt"]), "remote");
    assert_eq!(
        fs::read_to_string(work.join("remote.txt")).unwrap(),
        "remote\n"
    );
    assert!(
        git_text(
            &remote,
            [
                "for-each-ref",
                "--format=%(refname)",
                "refs/heads/cloud-sync/active"
            ]
        )
        .is_empty()
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn restore_creates_backup_and_conflict_can_be_resolved() {
    let root = std::env::temp_dir().join(format!("cloud-sync-conflict-{}", Uuid::new_v4()));
    let config = root.join("config");
    let state = root.join("state");
    let remote = root.join("remote.git");
    let work = root.join("work");
    let other = root.join("other");
    fs::create_dir_all(&work).unwrap();
    assert!(
        git(&root, ["init", "--bare", remote.to_str().unwrap()])
            .status
            .success()
    );
    fs::write(work.join("README"), "base\n").unwrap();
    assert!(
        cloud_sync(
            &config,
            &state,
            ["add", work.to_str().unwrap(), remote.to_str().unwrap()]
        )
        .status
        .success()
    );

    fs::write(work.join("README"), "local\n").unwrap();
    assert!(
        cloud_sync(&config, &state, ["sync", "work"])
            .status
            .success()
    );
    assert!(
        git(
            &root,
            ["clone", remote.to_str().unwrap(), other.to_str().unwrap()]
        )
        .status
        .success()
    );
    assert!(git(&other, ["checkout", "main"]).status.success());
    fs::write(other.join("README"), "remote\n").unwrap();
    assert!(git(&other, ["add", "README"]).status.success());
    assert!(
        git(
            &other,
            [
                "-c",
                "user.name=Other",
                "-c",
                "user.email=other@localhost",
                "commit",
                "-m",
                "remote conflict"
            ]
        )
        .status
        .success()
    );
    assert!(git(&other, ["push", "origin", "main"]).status.success());
    assert!(
        !cloud_sync(&config, &state, ["finalize", "work"])
            .status
            .success()
    );

    let conflicts = cloud_sync(&config, &state, ["conflicts"]);
    assert!(conflicts.status.success());
    let conflict_id = String::from_utf8(conflicts.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let shown = cloud_sync(&config, &state, ["conflict", "show", &conflict_id]);
    assert!(shown.status.success());
    assert!(String::from_utf8(shown.stdout).unwrap().contains("README"));
    assert!(
        cloud_sync(
            &config,
            &state,
            ["conflict", "resolve", &conflict_id, "local"]
        )
        .status
        .success()
    );
    assert_eq!(git_text(&remote, ["show", "main:README"]), "local");

    fs::write(work.join("README"), "temporary\n").unwrap();
    assert!(
        cloud_sync(&config, &state, ["restore", "work", "HEAD", "README"])
            .status
            .success()
    );
    assert_eq!(fs::read_to_string(work.join("README")).unwrap(), "local\n");
    let backups = cloud_sync(&config, &state, ["backup", "list", "work"]);
    assert!(backups.status.success());
    assert!(
        String::from_utf8(backups.stdout)
            .unwrap()
            .contains("cloud-sync/recovery")
    );
    let history = cloud_sync(&config, &state, ["history", "work"]);
    assert!(history.status.success());
    assert!(
        String::from_utf8(history.stdout)
            .unwrap()
            .contains("resolve conflict")
    );

    fs::remove_dir_all(root).unwrap();
}

fn cloud_sync<const N: usize>(config: &Path, state: &Path, args: [&str; N]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cloud-sync"))
        .args(args)
        .env("XDG_CONFIG_HOME", config)
        .env("XDG_STATE_HOME", state)
        .output()
        .unwrap()
}

fn git<const N: usize>(directory: &Path, args: [&str; N]) -> Output {
    Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .output()
        .unwrap()
}

fn git_text<const N: usize>(directory: &Path, args: [&str; N]) -> String {
    let output = git(directory, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
