use anyhow::{Context, Result};
use directories::ProjectDirs;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub config_file: PathBuf,
    pub state_dir: PathBuf,
    pub state_file: PathBuf,
    pub locks_dir: PathBuf,
    pub worktrees_dir: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let dirs = ProjectDirs::from("dev", "cloud-sync", "cloud-sync")
            .context("could not determine user configuration directories")?;
        let config_dir = dirs.config_dir().to_path_buf();
        let state_dir = dirs
            .state_dir()
            .unwrap_or_else(|| dirs.data_local_dir())
            .to_path_buf();
        Ok(Self {
            config_file: config_dir.join("config.toml"),
            state_file: state_dir.join("state.toml"),
            locks_dir: state_dir.join("locks"),
            worktrees_dir: state_dir.join("worktrees"),
            state_dir,
        })
    }

    pub fn ensure(&self) -> Result<()> {
        if let Some(parent) = self.config_file.parent() {
            fs::create_dir_all(parent).context("could not create configuration directory")?;
        }
        fs::create_dir_all(&self.state_dir).context("could not create state directory")?;
        fs::create_dir_all(&self.locks_dir).context("could not create lock directory")?;
        fs::create_dir_all(&self.worktrees_dir).context("could not create worktree directory")?;
        Ok(())
    }
}
