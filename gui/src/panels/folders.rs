use cloud_sync::config::Config;
use cloud_sync::paths::AppPaths;
use cloud_sync::repository;
use cloud_sync::session;
use cloud_sync::state::AppState;
use gtk4::prelude::*;
use gtk4::{self, Align, Box as GtkBox, Button, Dialog, Entry, Label, Orientation, ScrolledWindow};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;

pub fn build(config: &Config, paths: &AppPaths) -> ScrolledWindow {
    let container = GtkBox::new(Orientation::Vertical, 8);
    container.set_margin_start(16);
    container.set_margin_end(16);
    container.set_margin_top(8);
    container.set_margin_bottom(16);

    let header_row = GtkBox::new(Orientation::Horizontal, 8);
    let header = Label::new(Some("Monitored Folders"));
    header.add_css_class("panel-header");
    header.set_halign(Align::Start);
    header.set_hexpand(true);
    header_row.append(&header);

    let paths_rc = Rc::new(paths.clone());
    let container_rc = container.clone();

    let add_button = Button::with_label("Add folder");
    add_button.add_css_class("small-button");
    let paths_for_add = paths_rc.clone();
    let container_for_add = container_rc.clone();
    add_button.connect_clicked(move |button| {
        show_add_dialog(button, paths_for_add.clone(), container_for_add.clone());
    });
    header_row.append(&add_button);

    container.append(&header_row);
    populate(&container, config, paths);

    let scrolled = ScrolledWindow::new();
    scrolled.set_child(Some(&container));
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);
    scrolled
}

fn refresh(container: &GtkBox, paths: &AppPaths) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    let config = Config::load(paths).unwrap_or_default();
    let header_row = GtkBox::new(Orientation::Horizontal, 8);
    let header = Label::new(Some("Monitored Folders"));
    header.add_css_class("panel-header");
    header.set_halign(Align::Start);
    header.set_hexpand(true);
    header_row.append(&header);
    let add_button = Button::with_label("Add folder");
    add_button.add_css_class("small-button");
    let paths_rc = Rc::new(paths.clone());
    let container_rc = container.clone();
    add_button.connect_clicked(move |button| {
        show_add_dialog(button, paths_rc.clone(), container_rc.clone());
    });
    header_row.append(&add_button);
    container.append(&header_row);
    populate(container, &config, paths);
}

fn populate(container: &GtkBox, config: &Config, paths: &AppPaths) {
    let state = AppState::load(paths).ok();
    if config.repositories.is_empty() {
        let empty = Label::new(Some(
            "No repositories registered.\nUse the Add folder button or run: cloud-sync add <folder> <remote>",
        ));
        empty.add_css_class("empty-state");
        empty.set_halign(Align::Start);
        container.append(&empty);
        return;
    }
    for repo in &config.repositories {
        let card = build_repo_card(repo, state.as_ref(), paths, container);
        container.append(&card);
    }
}

fn build_repo_card(
    repo: &cloud_sync::config::RepositoryConfig,
    state: Option<&AppState>,
    paths: &AppPaths,
    container: &GtkBox,
) -> GtkBox {
    let card = GtkBox::new(Orientation::Horizontal, 12);
    card.add_css_class("repo-card");

    let info = GtkBox::new(Orientation::Vertical, 4);
    info.set_hexpand(true);
    info.set_valign(Align::Start);

    let name = Label::new(Some(&repo.id));
    name.add_css_class("repo-name");
    name.set_halign(Align::Start);
    info.append(&name);

    let path = Label::new(Some(&repo.path.display().to_string()));
    path.add_css_class("repo-path");
    path.set_halign(Align::Start);
    info.append(&path);

    let mut details = vec![format!("Branch: {}", repo.base_branch)];
    if !repo.presets.is_empty() {
        details.push(format!("Presets: {}", repo.presets.join(", ")));
    }
    if let Some(last) = state
        .and_then(|s| s.repositories.get(&repo.id))
        .and_then(|rs| rs.last_snapshot_at)
    {
        details.push(format!("Last sync: {}", last.format("%Y-%m-%d %H:%M")));
    }
    let details_label = Label::new(Some(&details.join("  ·  ")));
    details_label.add_css_class("muted");
    details_label.set_halign(Align::Start);
    info.append(&details_label);

    if let Some(state) = state {
        let repo_state = state.repositories.get(&repo.id);
        // A repository is only actually syncing while the daemon holds the
        // repo lock. An active session just means it is watching for changes.
        let syncing = is_locked(paths, &repo.id);
        let (status_text, status_class) = match repo_state {
            Some(rs) if rs.last_error.is_some() => ("Error", "repo-status-error"),
            _ if syncing => ("Syncing…", "repo-status-syncing"),
            Some(rs) if rs.active_session.is_some() => ("Watching", "repo-status-ok"),
            Some(_) => ("Up to date", "repo-status-ok"),
            None => ("Idle", "repo-status-ok"),
        };
        let conflicts = state
            .conflicts
            .values()
            .filter(|c| c.repository_id == repo.id && c.resolved_at.is_none())
            .count();
        let status_row = GtkBox::new(Orientation::Horizontal, 8);
        let status_label = Label::new(Some(status_text));
        status_label.add_css_class(status_class);
        status_label.set_halign(Align::Start);
        status_row.append(&status_label);
        if conflicts > 0 {
            let conflict_label = Label::new(Some(&format!("{conflicts} Conflict(s)")));
            conflict_label.add_css_class("repo-status-conflict");
            status_row.append(&conflict_label);
        }
        info.append(&status_row);
        if let Some(error) = repo_state.and_then(|rs| rs.last_error.as_ref()) {
            let err_label = Label::new(Some(error));
            err_label.add_css_class("repo-status-error");
            err_label.set_halign(Align::Start);
            err_label.set_wrap(true);
            info.append(&err_label);
        }
    }

    card.append(&info);

    let actions = GtkBox::new(Orientation::Horizontal, 6);
    actions.set_valign(Align::Start);
    actions.set_halign(Align::End);

    let repo_id = repo.id.clone();
    let paths_rc = Rc::new(paths.clone());
    let container_rc = container.clone();
    let sync_button = Button::with_label("Sync");
    sync_button.add_css_class("small-button");
    {
        let paths_rc = paths_rc.clone();
        let container_rc = container_rc.clone();
        sync_button.connect_clicked(move |button| {
            button.set_sensitive(false);
            let repo_id = repo_id.clone();
            let paths = (*paths_rc).clone();
            let thread_paths = paths.clone();
            let button = button.clone();
            let container = container_rc.clone();
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let paths = thread_paths;
                let result = (|| -> anyhow::Result<()> {
                    let config = Config::load(&paths)?;
                    let mut state = AppState::load(&paths)?;
                    let repo = config.repository(&repo_id)?.clone();
                    session::snapshot(&config, &mut state, &paths, &repo)?;
                    Ok(())
                })();
                let _ = sender.send(result);
            });
            gtk4::glib::spawn_future_local(async move {
                let result = loop {
                    match receiver.try_recv() {
                        Ok(r) => break Some(r),
                        Err(mpsc::TryRecvError::Disconnected) => break None,
                        Err(mpsc::TryRecvError::Empty) => {
                            gtk4::glib::timeout_future(std::time::Duration::from_millis(100)).await;
                        }
                    }
                };
                let _ = &result;
                button.set_sensitive(true);
                refresh(&container, &paths);
            });
        });
    }
    actions.append(&sync_button);

    let remove_button = Button::with_label("Remove");
    remove_button.add_css_class("small-button");
    let repo_id = repo.id.clone();
    let paths_for_remove = paths_rc.clone();
    let container_for_remove = container_rc.clone();
    remove_button.connect_clicked(move |button| {
        show_remove_dialog(
            button,
            repo_id.clone(),
            paths_for_remove.clone(),
            container_for_remove.clone(),
        );
    });
    actions.append(&remove_button);

    card.append(&actions);
    card
}

/// True when the daemon (or a manual sync) currently holds the repo lock,
/// i.e. a snapshot is actually in progress right now.
fn is_locked(paths: &AppPaths, repo_id: &str) -> bool {
    let lock_path = paths.locks_dir.join(format!("{repo_id}.lock"));
    let Ok(file) = std::fs::File::open(&lock_path) else {
        return false;
    };
    file.try_lock_shared().is_err()
}

fn show_add_dialog(button: &Button, paths: Rc<AppPaths>, container: GtkBox) {
    let dialog = Dialog::builder()
        .title("Add folder")
        .modal(true)
        .default_width(420)
        .build();
    if let Some(root) = button.root()
        && let Some(window) = root.downcast_ref::<gtk4::Window>()
    {
        dialog.set_transient_for(Some(window));
    }

    let content = GtkBox::new(Orientation::Vertical, 8);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.set_margin_top(16);
    content.set_margin_bottom(16);

    let folder_label = Label::new(Some("Local folder"));
    folder_label.set_halign(Align::Start);
    folder_label.add_css_class("setting-label");
    content.append(&folder_label);
    let folder_entry = Entry::new();
    folder_entry.set_placeholder_text(Some("/home/you/Documents/thesis"));
    content.append(&folder_entry);

    let remote_label = Label::new(Some("Remote URL"));
    remote_label.set_halign(Align::Start);
    remote_label.add_css_class("setting-label");
    remote_label.set_margin_top(8);
    content.append(&remote_label);
    let remote_entry = Entry::new();
    remote_entry.set_placeholder_text(Some("git@github.com:you/thesis.git"));
    content.append(&remote_entry);

    let error_label = Label::new(None);
    error_label.add_css_class("repo-status-error");
    error_label.set_halign(Align::Start);
    error_label.set_wrap(true);
    content.append(&error_label);

    let button_row = GtkBox::new(Orientation::Horizontal, 8);
    button_row.set_halign(Align::End);
    button_row.set_margin_top(12);
    let cancel = Button::with_label("Cancel");
    cancel.add_css_class("small-button");
    let add = Button::with_label("Add");
    add.add_css_class("action-button");
    button_row.append(&cancel);
    button_row.append(&add);
    content.append(&button_row);

    dialog.set_child(Some(&content));

    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| dialog.close());
    }

    {
        let dialog = dialog.clone();
        let paths = paths.clone();
        let container = container.clone();
        add.connect_clicked(move |_| {
            let folder_text = folder_entry.text().to_string();
            let remote_text = remote_entry.text().to_string();
            if folder_text.trim().is_empty() || remote_text.trim().is_empty() {
                error_label.set_text("Both fields are required");
                return;
            }
            let folder = PathBuf::from(folder_text.trim());
            let remote = remote_text.trim().to_string();
            let paths = (*paths).clone();
            let thread_paths = paths.clone();
            let container = container.clone();
            let dialog = dialog.clone();
            let error_label = error_label.clone();
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let paths = thread_paths;
                let result = (|| -> anyhow::Result<()> {
                    let mut config = Config::load(&paths)?;
                    repository::add(&mut config, &paths, &folder, &remote)?;
                    Ok(())
                })();
                let _ = sender.send(result);
            });
            gtk4::glib::spawn_future_local(async move {
                let result = loop {
                    match receiver.try_recv() {
                        Ok(r) => break Some(r),
                        Err(mpsc::TryRecvError::Disconnected) => break None,
                        Err(mpsc::TryRecvError::Empty) => {
                            gtk4::glib::timeout_future(std::time::Duration::from_millis(100)).await;
                        }
                    }
                };
                match result {
                    Some(Ok(())) => {
                        dialog.close();
                        refresh(&container, &paths);
                    }
                    Some(Err(e)) => error_label.set_text(&format!("{e:#}")),
                    None => error_label.set_text("Add failed unexpectedly"),
                }
            });
        });
    }

    dialog.present();
}

fn show_remove_dialog(button: &Button, repo_id: String, paths: Rc<AppPaths>, container: GtkBox) {
    let dialog = Dialog::builder()
        .title("Remove folder")
        .modal(true)
        .default_width(380)
        .build();
    if let Some(root) = button.root()
        && let Some(window) = root.downcast_ref::<gtk4::Window>()
    {
        dialog.set_transient_for(Some(window));
    }

    let content = GtkBox::new(Orientation::Vertical, 8);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.set_margin_top(16);
    content.set_margin_bottom(16);

    let message = Label::new(Some(&format!(
        "Stop syncing \"{repo_id}\"? The folder and its files stay on disk; it will no longer be synchronized."
    )));
    message.set_wrap(true);
    message.set_halign(Align::Start);
    content.append(&message);

    let button_row = GtkBox::new(Orientation::Horizontal, 8);
    button_row.set_halign(Align::End);
    button_row.set_margin_top(12);
    let cancel = Button::with_label("Cancel");
    cancel.add_css_class("small-button");
    let remove = Button::with_label("Remove");
    remove.add_css_class("danger-button");
    button_row.append(&cancel);
    button_row.append(&remove);
    content.append(&button_row);

    dialog.set_child(Some(&content));

    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| dialog.close());
    }
    {
        let dialog = dialog.clone();
        remove.connect_clicked(move |_| {
            if let Ok(mut config) = Config::load(&paths) {
                let before = config.repositories.len();
                config
                    .repositories
                    .retain(|r| r.id != repo_id && r.path != std::path::Path::new(&repo_id));
                if config.repositories.len() != before {
                    let _ = config.save(&paths);
                }
            }
            dialog.close();
            refresh(&container, &paths);
        });
    }

    dialog.present();
}
