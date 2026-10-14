use anyhow::Result;
use clap::{Parser, Subcommand};
use cloud_sync::paths::AppPaths;
use cloud_sync::theme::ThemeConfig;
use cloud_sync::updater;
use gtk4::prelude::*;

mod app;
mod panels;
mod theme_provider;

#[derive(Parser)]
#[command(version, about = "Cloud Sync desktop interface")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage the color theme
    Theme {
        #[command(subcommand)]
        command: ThemeCommands,
    },
    /// Check for and install GUI updates
    Update {
        #[command(subcommand)]
        command: UpdateCommands,
    },
}

#[derive(Subcommand)]
enum UpdateCommands {
    /// Check whether a newer release is available
    Check,
    /// Download and install the latest GUI release
    Install,
}

#[derive(Subcommand)]
enum ThemeCommands {
    /// List available themes
    List,
    /// Show the active theme
    Get,
    /// Set the active theme
    Set { name: String },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Theme { command }) => {
            if let Err(e) = handle_theme(command) {
                eprintln!("error: {e:#}");
                std::process::exit(1);
            }
        }
        Some(Commands::Update { command }) => {
            if let Err(e) = handle_update(command) {
                eprintln!("error: {e:#}");
                std::process::exit(1);
            }
        }
        None => {
            let app = gtk4::Application::builder()
                .application_id("dev.cloud-sync.gui")
                .build();

            app.connect_activate(|app| {
                app::build(app);
            });

            app.run();
        }
    }
}

fn handle_update(command: UpdateCommands) -> Result<()> {
    let paths = AppPaths::discover()?;
    paths.ensure()?;
    match command {
        UpdateCommands::Check => match updater::check()? {
            Some(update) => println!(
                "update available: {} -> {} ({})",
                update.current, update.available, update.tag
            ),
            None => println!("cloud-sync-gui is up to date"),
        },
        UpdateCommands::Install => match updater::check()? {
            Some(update) => match updater::install_gui(&update, &paths)? {
                Some(installed) => println!(
                    "updated cloud-sync-gui from {} to {} at {}",
                    update.current,
                    update.available,
                    installed.display()
                ),
                None => {
                    println!("cloud-sync-gui is not installed; install it to ~/.local/bin first")
                }
            },
            None => println!("cloud-sync-gui is already up to date"),
        },
    }
    Ok(())
}

fn handle_theme(command: ThemeCommands) -> Result<()> {
    let paths = AppPaths::discover()?;
    paths.ensure()?;

    match command {
        ThemeCommands::List => {
            let tc = ThemeConfig::load(&paths)?;
            for name in tc.list_names() {
                let marker = if name == tc.active { " (active)" } else { "" };
                println!("{name}{marker}");
            }
        }
        ThemeCommands::Get => {
            let tc = ThemeConfig::load(&paths)?;
            println!("{}", tc.active);
        }
        ThemeCommands::Set { name } => {
            let mut tc = ThemeConfig::load(&paths)?;
            tc.set_active(&name)?;
            tc.save(&paths)?;
            println!("theme set to {name}");
        }
    }
    Ok(())
}
