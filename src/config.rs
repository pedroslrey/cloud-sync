use crate::paths::AppPaths;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub device_id: String,
    pub defaults: SyncSettings,
    pub repositories: Vec<RepositoryConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncSettings {
    pub inactivity_seconds: u64,
    pub remote_check_seconds: u64,
    pub maximum_sync_seconds: Option<u64>,
    pub completed_session_retention_days: u64,
    pub skip_untracked_binaries: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RepositoryConfig {
    pub id: String,
    pub path: PathBuf,
    pub remote: String,
    pub remote_name: String,
    pub base_branch: String,
    pub presets: Vec<String>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub inactivity_seconds: Option<u64>,
    pub remote_check_seconds: Option<u64>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            device_id: default_device_id(),
            defaults: SyncSettings::default(),
            repositories: Vec::new(),
        }
    }
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            inactivity_seconds: 300,
            remote_check_seconds: 900,
            maximum_sync_seconds: None,
            completed_session_retention_days: 7,
            skip_untracked_binaries: true,
        }
    }
}

impl Default for RepositoryConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            path: PathBuf::new(),
            remote: String::new(),
            remote_name: "origin".into(),
            base_branch: "main".into(),
            presets: Vec::new(),
            include: Vec::new(),
            exclude: Vec::new(),
            inactivity_seconds: None,
            remote_check_seconds: None,
        }
    }
}

impl Config {
    pub fn load(paths: &AppPaths) -> Result<Self> {
        if !paths.config_file.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(&paths.config_file)
            .with_context(|| format!("could not read {}", paths.config_file.display()))?;
        let config: Self = toml::from_str(&text)
            .with_context(|| format!("invalid configuration in {}", paths.config_file.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        self.validate()?;
        paths.ensure()?;
        let text = toml::to_string_pretty(self).context("could not serialize configuration")?;
        atomic_write(&paths.config_file, text.as_bytes())
    }

    pub fn validate(&self) -> Result<()> {
        for (index, repo) in self.repositories.iter().enumerate() {
            if repo.id.is_empty() || repo.remote.is_empty() || repo.base_branch.is_empty() {
                bail!("repository entry {} has an empty required field", index + 1);
            }
            if !repo.path.is_absolute() {
                bail!("repository {} path must be absolute", repo.id);
            }
            for other in self.repositories.iter().skip(index + 1) {
                if repo.path == other.path
                    || repo.path.starts_with(&other.path)
                    || other.path.starts_with(&repo.path)
                {
                    bail!(
                        "repository paths overlap: {} and {}",
                        repo.path.display(),
                        other.path.display()
                    );
                }
            }
        }
        Ok(())
    }

    pub fn repository(&self, selector: &str) -> Result<&RepositoryConfig> {
        self.repositories
            .iter()
            .find(|repo| repo.id == selector || repo.path == Path::new(selector))
            .with_context(|| format!("unknown repository: {selector}"))
    }

    pub fn repository_mut(&mut self, selector: &str) -> Result<&mut RepositoryConfig> {
        self.repositories
            .iter_mut()
            .find(|repo| repo.id == selector || repo.path == Path::new(selector))
            .with_context(|| format!("unknown repository: {selector}"))
    }
}

fn default_device_id() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "device".into())
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes)
        .with_context(|| format!("could not write {}", temporary.display()))?;
    fs::rename(&temporary, path)
        .with_context(|| format!("could not replace {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_planned_timing() {
        let settings = SyncSettings::default();
        assert_eq!(settings.inactivity_seconds, 300);
        assert_eq!(settings.remote_check_seconds, 900);
        assert_eq!(settings.maximum_sync_seconds, None);
        assert_eq!(settings.completed_session_retention_days, 7);
    }

    #[test]
    fn rejects_overlapping_roots() {
        let config = Config {
            repositories: vec![
                RepositoryConfig {
                    id: "one".into(),
                    path: "/tmp/a".into(),
                    remote: "x".into(),
                    ..Default::default()
                },
                RepositoryConfig {
                    id: "two".into(),
                    path: "/tmp/a/b".into(),
                    remote: "y".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }
}
