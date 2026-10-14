use crate::config::atomic_write;
use crate::paths::AppPaths;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// A base16 color scheme with 16 hex colors.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Base16Scheme {
    pub name: String,
    pub base00: String,
    pub base01: String,
    pub base02: String,
    pub base03: String,
    pub base04: String,
    pub base05: String,
    pub base06: String,
    pub base07: String,
    pub base08: String,
    pub base09: String,
    pub base0a: String,
    pub base0b: String,
    pub base0c: String,
    pub base0d: String,
    pub base0e: String,
    pub base0f: String,
}

/// Theme configuration persisted alongside the main config.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub active: String,
    pub schemes: Vec<Base16Scheme>,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            active: "dark".into(),
            schemes: vec![default_dark(), default_light()],
        }
    }
}

impl ThemeConfig {
    pub fn path(paths: &AppPaths) -> PathBuf {
        paths
            .config_file
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("theme.toml")
    }

    pub fn load(paths: &AppPaths) -> Result<Self> {
        let path = Self::path(paths);
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(&path)
            .with_context(|| format!("could not read {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("invalid theme config in {}", path.display()))
    }

    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        paths.ensure()?;
        let path = Self::path(paths);
        let text = toml::to_string_pretty(self).context("could not serialize theme config")?;
        atomic_write(&path, text.as_bytes())
    }

    pub fn active_scheme(&self) -> &Base16Scheme {
        self.schemes
            .iter()
            .find(|s| s.name == self.active)
            .unwrap_or_else(|| {
                self.schemes
                    .first()
                    .expect("theme config must have at least one scheme")
            })
    }

    pub fn set_active(&mut self, name: &str) -> Result<()> {
        if !self.schemes.iter().any(|s| s.name == name) {
            let available: Vec<_> = self.schemes.iter().map(|s| s.name.as_str()).collect();
            bail!("unknown theme: {name}\navailable: {}", available.join(", "));
        }
        self.active = name.into();
        Ok(())
    }

    pub fn list_names(&self) -> Vec<&str> {
        self.schemes.iter().map(|s| s.name.as_str()).collect()
    }
}

/// Default dark scheme — muted, low-contrast dark background.
pub fn default_dark() -> Base16Scheme {
    Base16Scheme {
        name: "dark".into(),
        base00: "#1d1f21".into(), // background
        base01: "#282a2e".into(), // lighter background
        base02: "#373b41".into(), // selection
        base03: "#969896".into(), // comments
        base04: "#b4b7b4".into(), // dark foreground
        base05: "#c5c8c6".into(), // foreground
        base06: "#e0e0e0".into(), // light foreground
        base07: "#ffffff".into(), // lightest
        base08: "#cc6666".into(), // red
        base09: "#de935f".into(), // orange
        base0a: "#f0c674".into(), // yellow
        base0b: "#b5bd68".into(), // green
        base0c: "#8abeb7".into(), // cyan
        base0d: "#81a2be".into(), // blue
        base0e: "#b294bb".into(), // purple
        base0f: "#a3685a".into(), // brown
    }
}

/// Default light scheme — clean, high-contrast light background.
pub fn default_light() -> Base16Scheme {
    Base16Scheme {
        name: "light".into(),
        base00: "#f8f8f8".into(), // background
        base01: "#e8e8e8".into(), // lighter background
        base02: "#d8d8d8".into(), // selection
        base03: "#8e908c".into(), // comments
        base04: "#585858".into(), // dark foreground
        base05: "#383a42".into(), // foreground
        base06: "#282a2e".into(), // light foreground
        base07: "#1d1f21".into(), // darkest
        base08: "#e45649".into(), // red
        base09: "#d19a66".into(), // orange
        base0a: "#c18401".into(), // yellow
        base0b: "#50a14f".into(), // green
        base0c: "#0184bc".into(), // cyan
        base0d: "#4078f2".into(), // blue
        base0e: "#a626a4".into(), // purple
        base0f: "#986801".into(), // brown
    }
}
