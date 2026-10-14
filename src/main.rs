use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use cloud_sync::config::Config;
use cloud_sync::paths::AppPaths;
use cloud_sync::repository;
use cloud_sync::session;
use cloud_sync::state::AppState;
use cloud_sync::{conflict, daemon, recovery};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser)]
#[command(version, about = "Git-backed personal folder synchronization")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Add {
        folder: PathBuf,
        repository: String,
    },
    Remove {
        repository: String,
    },
    List,
    Status {
        repository: Option<String>,
    },
    Sync {
        repository: String,
    },
    SyncAll,
    Finalize {
        repository: String,
    },
    FinalizeAll,
    Include {
        repository: String,
        pattern: String,
    },
    Exclude {
        repository: String,
        pattern: String,
    },
    History {
        repository: String,
        file: Option<String>,
        #[arg(short, long, default_value_t = 30)]
        limit: usize,
    },
    Show {
        repository: String,
        revision: String,
        file: Option<String>,
    },
    Restore {
        repository: String,
        revision: String,
        file: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    Backup {
        #[command(subcommand)]
        command: BackupCommands,
    },
    Conflicts {
        repository: Option<String>,
    },
    Conflict {
        #[command(subcommand)]
        command: ConflictCommands,
    },
    Completions {
        shell: Shell,
    },
    ConfigPath,
    Daemon,
    InstallService,
}

#[derive(Subcommand)]
enum BackupCommands {
    List {
        repository: String,
    },
    Show {
        repository: String,
        reference: String,
    },
    Restore {
        repository: String,
        reference: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum ConflictCommands {
    Show {
        id: String,
    },
    Resolve {
        id: String,
        #[arg(value_enum)]
        resolution: ConflictChoice,
    },
}

#[derive(Clone, clap::ValueEnum)]
enum ConflictChoice {
    Local,
    Remote,
    Newest,
    KeepBoth,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let paths = AppPaths::discover()?;
    paths.ensure()?;
    let mut config = Config::load(&paths)?;
    match cli.command {
        Commands::Add {
            folder,
            repository: remote,
        } => {
            let repo = repository::add(&mut config, &paths, &folder, &remote)?;
            println!("added {} ({})", repo.id, repo.path.display());
        }
        Commands::Remove {
            repository: selector,
        } => {
            let count = config.repositories.len();
            config
                .repositories
                .retain(|repo| repo.id != selector && repo.path != Path::new(&selector));
            if config.repositories.len() == count {
                bail!("unknown repository: {selector}");
            }
            config.save(&paths)?;
            println!("removed {selector} from cloud-sync configuration; files were not deleted");
        }
        Commands::List => {
            for repo in &config.repositories {
                println!(
                    "{}\t{}\t{}\t{}",
                    repo.id,
                    repo.path.display(),
                    repo.base_branch,
                    repo.remote
                );
            }
        }
        Commands::Status {
            repository: selector,
        } => status(&config, &paths, selector.as_deref())?,
        Commands::Sync {
            repository: selector,
        } => {
            let repo = config.repository(&selector)?.clone();
            let mut state = AppState::load(&paths)?;
            match session::snapshot(&config, &mut state, &paths, &repo)? {
                Some(commit) => println!("snapshot {commit}"),
                None => println!("no changes"),
            }
        }
        Commands::SyncAll => sync_all(&config, &paths)?,
        Commands::Finalize {
            repository: selector,
        } => {
            let repo = config.repository(&selector)?.clone();
            let mut state = AppState::load(&paths)?;
            match session::finalize(&config, &mut state, &paths, &repo)? {
                Some(commit) => println!("finalized {commit}"),
                None => println!("no session changes"),
            }
        }
        Commands::FinalizeAll => finalize_all(&config, &paths)?,
        Commands::Include {
            repository: selector,
            pattern,
        } => {
            let repo = config.repository_mut(&selector)?;
            if !repo.include.contains(&pattern) {
                repo.include.push(pattern.clone());
            }
            config.save(&paths)?;
            println!("included {pattern}");
        }
        Commands::Exclude {
            repository: selector,
            pattern,
        } => {
            let repo = config.repository_mut(&selector)?;
            if !repo.exclude.contains(&pattern) {
                repo.exclude.push(pattern.clone());
            }
            config.save(&paths)?;
            println!("excluded {pattern}");
        }
        Commands::History {
            repository: selector,
            file,
            limit,
        } => {
            let repo = config.repository(&selector)?;
            println!("{}", recovery::history(repo, file.as_deref(), limit)?);
        }
        Commands::Show {
            repository: selector,
            revision,
            file,
        } => {
            let repo = config.repository(&selector)?;
            println!("{}", recovery::show(repo, &revision, file.as_deref())?);
        }
        Commands::Restore {
            repository: selector,
            revision,
            file,
            yes,
        } => {
            if file.is_none() && !yes {
                bail!("whole-repository restore requires --yes");
            }
            let repo = config.repository(&selector)?.clone();
            let mut state = AppState::load(&paths)?;
            let backup = recovery::restore(
                &config,
                &mut state,
                &paths,
                &repo,
                &revision,
                file.as_deref(),
            )?;
            println!("restored from {revision}; previous state preserved at {backup}");
        }
        Commands::Backup { command } => match command {
            BackupCommands::List {
                repository: selector,
            } => {
                println!("{}", recovery::list_backups(config.repository(&selector)?)?);
            }
            BackupCommands::Show {
                repository: selector,
                reference,
            } => {
                println!(
                    "{}",
                    recovery::show_backup(config.repository(&selector)?, &reference)?
                );
            }
            BackupCommands::Restore {
                repository: selector,
                reference,
                yes,
            } => {
                if !yes {
                    bail!("backup restore requires --yes");
                }
                let repo = config.repository(&selector)?.clone();
                let mut state = AppState::load(&paths)?;
                let backup =
                    recovery::restore(&config, &mut state, &paths, &repo, &reference, None)?;
                println!("restored {reference}; previous state preserved at {backup}");
            }
        },
        Commands::Conflicts { repository } => {
            let state = AppState::load(&paths)?;
            let output = conflict::list(&state, repository.as_deref());
            println!(
                "{}",
                if output.is_empty() {
                    "no unresolved conflicts"
                } else {
                    &output
                }
            );
        }
        Commands::Conflict { command } => {
            let mut state = AppState::load(&paths)?;
            match command {
                ConflictCommands::Show { id } => {
                    println!("{}", conflict::show(&config, &state, &id)?)
                }
                ConflictCommands::Resolve { id, resolution } => {
                    let resolution = match resolution {
                        ConflictChoice::Local => conflict::Resolution::Local,
                        ConflictChoice::Remote => conflict::Resolution::Remote,
                        ConflictChoice::Newest => conflict::Resolution::Newest,
                        ConflictChoice::KeepBoth => conflict::Resolution::KeepBoth,
                    };
                    let commit = conflict::resolve(&config, &mut state, &paths, &id, resolution)?;
                    println!("resolved {id} as {commit}");
                }
            }
        }
        Commands::Completions { shell } => {
            generate(
                shell,
                &mut Cli::command(),
                "cloud-sync",
                &mut std::io::stdout(),
            );
        }
        Commands::ConfigPath => println!("{}", paths.config_file.display()),
        Commands::Daemon => daemon::run(config, paths)?,
        Commands::InstallService => install_service()?,
    }
    Ok(())
}

fn sync_all(config: &Config, paths: &AppPaths) -> Result<()> {
    let mut state = AppState::load(paths)?;
    let mut failures = Vec::new();
    for repo in &config.repositories {
        match session::snapshot(config, &mut state, paths, repo) {
            Ok(Some(commit)) => println!("{}: snapshot {commit}", repo.id),
            Ok(None) => println!("{}: no changes", repo.id),
            Err(error) => failures.push(format!("{}: {error:#}", repo.id)),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        bail!(failures.join("\n"))
    }
}

fn finalize_all(config: &Config, paths: &AppPaths) -> Result<()> {
    let mut state = AppState::load(paths)?;
    let mut failures = Vec::new();
    for repo in &config.repositories {
        match session::finalize(config, &mut state, paths, repo) {
            Ok(Some(commit)) => println!("{}: finalized {commit}", repo.id),
            Ok(None) => println!("{}: no session changes", repo.id),
            Err(error) => failures.push(format!("{}: {error:#}", repo.id)),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        bail!(failures.join("\n"))
    }
}

fn status(config: &Config, paths: &AppPaths, selector: Option<&str>) -> Result<()> {
    let state = AppState::load(paths)?;
    for repo in &config.repositories {
        if selector.is_some_and(|value| value != repo.id && Path::new(value) != repo.path) {
            continue;
        }
        println!("{}", repo.id);
        println!("  path: {}", repo.path.display());
        println!("  base: {}", repo.base_branch);
        println!(
            "  presets: {}",
            if repo.presets.is_empty() {
                "none".into()
            } else {
                repo.presets.join(", ")
            }
        );
        if let Some(repo_state) = state.repositories.get(&repo.id) {
            println!(
                "  session: {}",
                repo_state
                    .active_session
                    .as_ref()
                    .map_or("none", |session| session.branch.as_str())
            );
            println!(
                "  last snapshot: {}",
                repo_state
                    .last_snapshot_at
                    .map_or_else(|| "never".into(), |time| time.to_rfc3339())
            );
            if let Some(error) = &repo_state.last_error {
                println!("  error: {error}");
            }
        } else {
            println!("  session: none");
        }
    }
    Ok(())
}

fn install_service() -> Result<()> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    let home = PathBuf::from(home);
    let binary_dir = home.join(".local/bin");
    fs::create_dir_all(&binary_dir)?;
    let current_binary =
        std::env::current_exe().context("could not locate running cloud-sync binary")?;
    fs::copy(&current_binary, binary_dir.join("cloud-sync"))
        .context("could not install cloud-sync binary")?;
    let unit_dir = home.join(".config/systemd/user");
    fs::create_dir_all(&unit_dir)?;
    let units = [
        (
            "cloud-sync.service",
            include_str!("../systemd/cloud-sync.service"),
        ),
        (
            "cloud-sync.timer",
            include_str!("../systemd/cloud-sync.timer"),
        ),
        (
            "cloud-sync-sync.service",
            include_str!("../systemd/cloud-sync-sync.service"),
        ),
    ];
    for (name, content) in units {
        fs::write(unit_dir.join(name), content)?;
    }
    install_completion(&home)?;
    systemctl(["daemon-reload"])?;
    systemctl(["enable", "--now", "cloud-sync.service", "cloud-sync.timer"])?;
    println!("installed and started systemd user units");
    Ok(())
}

fn install_completion(home: &Path) -> Result<()> {
    let shell = std::env::var("SHELL").unwrap_or_default();
    let (generator, destination) = if shell.ends_with("/zsh") {
        (
            Shell::Zsh,
            home.join(".local/share/zsh/site-functions/_cloud-sync"),
        )
    } else if shell.ends_with("/fish") {
        (
            Shell::Fish,
            home.join(".config/fish/completions/cloud-sync.fish"),
        )
    } else {
        (
            Shell::Bash,
            home.join(".local/share/bash-completion/completions/cloud-sync"),
        )
    };
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = Vec::new();
    generate(generator, &mut Cli::command(), "cloud-sync", &mut output);
    fs::write(destination, output)?;
    Ok(())
}

fn systemctl<const N: usize>(args: [&str; N]) -> Result<()> {
    let status = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .status()
        .context("could not execute systemctl")?;
    if !status.success() {
        bail!("systemctl --user failed");
    }
    Ok(())
}
