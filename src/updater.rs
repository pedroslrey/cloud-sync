use crate::paths::AppPaths;
use anyhow::{Context, Result, bail};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const RELEASE_API: &str = "https://api.github.com/repos/pedroslrey/cloud-sync/releases/latest";
const RELEASE_PREFIX: &str = "https://github.com/pedroslrey/cloud-sync/releases/download/";

#[derive(Clone, Debug)]
pub struct UpdateInfo {
    pub current: Version,
    pub available: Version,
    pub tag: String,
    binary_url: String,
    checksum_url: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

pub fn check() -> Result<Option<UpdateInfo>> {
    ensure_supported_target()?;
    let output = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--location",
            "--write-out",
            "\n%{http_code}",
            "--header",
            "Accept: application/vnd.github+json",
            "--header",
            "X-GitHub-Api-Version: 2022-11-28",
            "--user-agent",
            "cloud-sync-updater",
            RELEASE_API,
        ])
        .output()
        .context("could not execute curl; install curl to check for updates")?;
    if !output.status.success() {
        bail!(
            "update check failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let response =
        String::from_utf8(output.stdout).context("GitHub returned a non-UTF-8 response")?;
    let (body, status) = response
        .rsplit_once('\n')
        .context("GitHub response did not include an HTTP status")?;
    match status {
        "200" => parse_release(body.as_bytes()),
        "404" => Ok(None),
        _ => bail!("GitHub release check returned HTTP {status}"),
    }
}

pub fn install(info: &UpdateInfo, paths: &AppPaths) -> Result<PathBuf> {
    validate_release_url(&info.binary_url)?;
    validate_release_url(&info.checksum_url)?;
    paths.ensure()?;
    let update_dir = paths
        .state_dir
        .join("updates")
        .join(info.available.to_string());
    fs::create_dir_all(&update_dir)?;
    let candidate = update_dir.join(asset_name()?);
    let checksum = update_dir.join(format!("{}.sha256", asset_name()?));
    download(&info.binary_url, &candidate)?;
    download(&info.checksum_url, &checksum)?;
    verify_checksum(&candidate, &checksum)?;
    fs::set_permissions(&candidate, fs::Permissions::from_mode(0o755))?;

    let home = std::env::var_os("HOME").context("HOME is not set")?;
    let binary_dir = PathBuf::from(home).join(".local/bin");
    fs::create_dir_all(&binary_dir)?;
    let installed = binary_dir.join("cloud-sync");
    if !installed.exists() {
        bail!(
            "cloud-sync is not installed at {}; run install-service first",
            installed.display()
        );
    }
    let previous = binary_dir.join("cloud-sync.previous");
    let replacement = binary_dir.join("cloud-sync.new");
    fs::copy(&installed, &previous).context("could not back up installed binary")?;
    fs::copy(&candidate, &replacement).context("could not stage update")?;
    fs::set_permissions(&replacement, fs::Permissions::from_mode(0o755))?;

    systemctl(["stop", "cloud-sync.service"])?;
    if let Err(error) = replace_and_restart(&replacement, &installed) {
        let rollback = rollback(&previous, &installed);
        return match rollback {
            Ok(()) => Err(error.context("update failed; previous version restored")),
            Err(rollback_error) => Err(error.context(format!(
                "update failed and rollback also failed: {rollback_error:#}"
            ))),
        };
    }
    Ok(installed)
}

pub fn notify_available(info: &UpdateInfo) {
    let _ = Command::new("notify-send")
        .args([
            "Cloud Sync update available",
            &format!(
                "Version {} is available. Run: cloud-sync update install",
                info.available
            ),
        ])
        .status();
}

fn parse_release(bytes: &[u8]) -> Result<Option<UpdateInfo>> {
    let release: GithubRelease =
        serde_json::from_slice(bytes).context("invalid GitHub release response")?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let available =
        Version::parse(release.tag_name.trim_start_matches('v')).with_context(|| {
            format!(
                "release tag is not a semantic version: {}",
                release.tag_name
            )
        })?;
    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
    if available <= current {
        return Ok(None);
    }
    let asset = asset_name()?;
    let checksum = format!("{asset}.sha256");
    let binary_url = release
        .assets
        .iter()
        .find(|candidate| candidate.name == asset)
        .map(|candidate| candidate.browser_download_url.clone())
        .with_context(|| format!("release {} does not contain {asset}", release.tag_name))?;
    let checksum_url = release
        .assets
        .iter()
        .find(|candidate| candidate.name == checksum)
        .map(|candidate| candidate.browser_download_url.clone())
        .with_context(|| format!("release {} does not contain {checksum}", release.tag_name))?;
    validate_release_url(&binary_url)?;
    validate_release_url(&checksum_url)?;
    Ok(Some(UpdateInfo {
        current,
        available,
        tag: release.tag_name,
        binary_url,
        checksum_url,
    }))
}

fn asset_name() -> Result<&'static str> {
    ensure_supported_target()?;
    Ok("cloud-sync-linux-x86_64")
}

fn ensure_supported_target() -> Result<()> {
    if std::env::consts::OS != "linux" || std::env::consts::ARCH != "x86_64" {
        bail!(
            "release updates are not available for {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
    }
    Ok(())
}

fn validate_release_url(url: &str) -> Result<()> {
    if !url.starts_with(RELEASE_PREFIX) {
        bail!("refusing unexpected release URL: {url}");
    }
    Ok(())
}

fn download(url: &str, destination: &Path) -> Result<()> {
    let status = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--output",
        ])
        .arg(destination)
        .arg(url)
        .status()
        .context("could not execute curl")?;
    if !status.success() {
        bail!("could not download {url}");
    }
    Ok(())
}

fn verify_checksum(binary: &Path, checksum_file: &Path) -> Result<()> {
    let expected_text =
        fs::read_to_string(checksum_file).context("could not read release checksum")?;
    let expected = expected_text
        .split_whitespace()
        .next()
        .context("release checksum is empty")?;
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("release checksum is malformed");
    }
    let bytes = fs::read(binary).context("could not read downloaded binary")?;
    let actual = format!("{:x}", Sha256::digest(bytes));
    if !actual.eq_ignore_ascii_case(expected) {
        bail!("release checksum mismatch");
    }
    Ok(())
}

fn replace_and_restart(replacement: &Path, installed: &Path) -> Result<()> {
    fs::rename(replacement, installed).context("could not activate update")?;
    systemctl(["start", "cloud-sync.service"])?;
    systemctl(["is-active", "--quiet", "cloud-sync.service"])
        .context("updated service did not become active")
}

fn rollback(previous: &Path, installed: &Path) -> Result<()> {
    let rollback = installed.with_extension("rollback");
    fs::copy(previous, &rollback).context("could not stage rollback")?;
    fs::set_permissions(&rollback, fs::Permissions::from_mode(0o755))?;
    fs::rename(rollback, installed).context("could not restore previous binary")?;
    systemctl(["start", "cloud-sync.service"])?;
    systemctl(["is-active", "--quiet", "cloud-sync.service"])
}

fn systemctl<const N: usize>(args: [&str; N]) -> Result<()> {
    let status = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .status()
        .context("could not execute systemctl")?;
    if !status.success() {
        bail!("systemctl --user {} failed", args.join(" "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_current_and_older_releases() {
        let current = env!("CARGO_PKG_VERSION");
        let json =
            format!(r#"{{"tag_name":"v{current}","draft":false,"prerelease":false,"assets":[]}}"#);
        assert!(parse_release(json.as_bytes()).unwrap().is_none());
    }

    #[test]
    fn discovers_expected_release_assets() {
        let json = br#"{
            "tag_name":"v99.0.0",
            "draft":false,
            "prerelease":false,
            "assets":[
                {"name":"cloud-sync-linux-x86_64","browser_download_url":"https://github.com/pedroslrey/cloud-sync/releases/download/v99.0.0/cloud-sync-linux-x86_64"},
                {"name":"cloud-sync-linux-x86_64.sha256","browser_download_url":"https://github.com/pedroslrey/cloud-sync/releases/download/v99.0.0/cloud-sync-linux-x86_64.sha256"}
            ]
        }"#;
        let update = parse_release(json).unwrap().unwrap();
        assert_eq!(update.available, Version::new(99, 0, 0));
    }

    #[test]
    fn rejects_release_assets_from_other_hosts() {
        assert!(validate_release_url("https://example.com/cloud-sync").is_err());
    }

    #[test]
    fn verifies_download_checksum() {
        let root = std::env::temp_dir().join(format!("cloud-sync-update-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let binary = root.join("binary");
        let checksum = root.join("binary.sha256");
        fs::write(&binary, b"cloud-sync").unwrap();
        fs::write(
            &checksum,
            format!("{:x}  binary\n", Sha256::digest(b"cloud-sync")),
        )
        .unwrap();
        assert!(verify_checksum(&binary, &checksum).is_ok());
        fs::write(&binary, b"modified").unwrap();
        assert!(verify_checksum(&binary, &checksum).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
