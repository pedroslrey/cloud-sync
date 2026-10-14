use crate::config::RepositoryConfig;
use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

const COMMON_EXCLUDES: &[&str] = &[
    "**/*~",
    "**/*.swp",
    "**/*.swo",
    "**/.DS_Store",
    "**/.idea/**",
];

pub fn detect_presets(root: &Path) -> Vec<String> {
    let candidates = [
        ("rust", "Cargo.toml"),
        ("python", "pyproject.toml"),
        ("python", "requirements.txt"),
        ("node", "package.json"),
        ("latex", "main.tex"),
    ];
    let mut presets = Vec::new();
    for (preset, marker) in candidates {
        if root.join(marker).exists() && !presets.iter().any(|existing| existing == preset) {
            presets.push(preset.to_owned());
        }
    }
    if presets.iter().all(|preset| preset != "latex")
        && std::fs::read_dir(root)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "tex")
            })
    {
        presets.push("latex".into());
    }
    presets
}

pub struct Filter {
    includes: GlobSet,
    excludes: GlobSet,
}

impl Filter {
    pub fn from_repository(repository: &RepositoryConfig) -> Result<Self> {
        let includes = build_globs(repository.include.iter().map(String::as_str))?;
        let exclusions = COMMON_EXCLUDES
            .iter()
            .copied()
            .chain(
                repository
                    .presets
                    .iter()
                    .flat_map(|preset| preset_excludes(preset).iter().copied()),
            )
            .chain(repository.exclude.iter().map(String::as_str));
        Ok(Self {
            includes,
            excludes: build_globs(exclusions)?,
        })
    }

    pub fn explicitly_includes(&self, relative: &Path) -> bool {
        self.includes.is_match(relative)
    }

    pub fn decision(
        &self,
        root: &Path,
        relative: &Path,
        skip_binary: bool,
    ) -> Result<FilterDecision> {
        if self.explicitly_includes(relative) {
            return Ok(FilterDecision::Include);
        }
        if self.excludes.is_match(relative) {
            return Ok(FilterDecision::Exclude(
                "excluded by cloud-sync pattern".into(),
            ));
        }
        if skip_binary && is_binary(&root.join(relative))? {
            return Ok(FilterDecision::Exclude("untracked binary file".into()));
        }
        Ok(FilterDecision::Include)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FilterDecision {
    Include,
    Exclude(String),
}

fn build_globs<'a>(patterns: impl Iterator<Item = &'a str>) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder
            .add(Glob::new(pattern).with_context(|| format!("invalid filter pattern: {pattern}"))?);
    }
    builder.build().context("could not build file filters")
}

fn preset_excludes(preset: &str) -> &'static [&'static str] {
    match preset {
        "rust" => &["target/**"],
        "python" => &[
            "**/__pycache__/**",
            "**/*.pyc",
            ".venv/**",
            "venv/**",
            ".pytest_cache/**",
        ],
        "node" => &["node_modules/**", "dist/**", ".next/**", "coverage/**"],
        "latex" => &[
            "**/*.aux",
            "**/*.fls",
            "**/*.fdb_latexmk",
            "**/*.log",
            "**/*.synctex.gz",
            "**/*.toc",
            "**/*.out",
        ],
        _ => &[],
    }
}

pub fn is_binary(path: &Path) -> Result<bool> {
    if !path.is_file() {
        return Ok(false);
    }
    let mut file =
        File::open(path).with_context(|| format!("could not inspect {}", path.display()))?;
    let mut buffer = [0_u8; 8192];
    let count = file
        .read(&mut buffer)
        .with_context(|| format!("could not inspect {}", path.display()))?;
    let bytes = &buffer[..count];
    if bytes.contains(&0) {
        return Ok(true);
    }
    let controls = bytes
        .iter()
        .filter(|byte| **byte < 0x20 && !matches!(**byte, b'\n' | b'\r' | b'\t' | 0x0c | 0x08))
        .count();
    Ok(!bytes.is_empty() && controls * 100 / bytes.len() > 5)
}

pub fn normalized_relative(root: &Path, path: PathBuf) -> Option<PathBuf> {
    path.strip_prefix(root).ok().map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn binary_detection_handles_text_and_nul() {
        let root = std::env::temp_dir().join(format!("cloud-sync-filter-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("text"), "hello\n").unwrap();
        fs::write(root.join("binary"), b"a\0b").unwrap();
        assert!(!is_binary(&root.join("text")).unwrap());
        assert!(is_binary(&root.join("binary")).unwrap());
        fs::remove_dir_all(root).unwrap();
    }
}
