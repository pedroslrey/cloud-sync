use cloud_sync::config::Config;
use cloud_sync::paths::AppPaths;
use cloud_sync::state::AppState;
use gtk4::prelude::*;
use gtk4::{self, Align, Box as GtkBox, Button, Label, Orientation, ScrolledWindow, TextView};

pub fn build(config: &Config, paths: &AppPaths) -> ScrolledWindow {
    let container = GtkBox::new(Orientation::Vertical, 8);
    container.set_margin_start(16);
    container.set_margin_end(16);
    container.set_margin_top(8);
    container.set_margin_bottom(16);

    let header = Label::new(Some("Conflicts"));
    header.add_css_class("panel-header");
    header.set_halign(Align::Start);
    container.append(&header);

    let state = match AppState::load(paths) {
        Ok(s) => s,
        Err(e) => {
            let err = Label::new(Some(&format!("Could not load state: {e}")));
            err.add_css_class("repo-status-error");
            container.append(&err);
            let scrolled = ScrolledWindow::new();
            scrolled.set_child(Some(&container));
            scrolled.set_vexpand(true);
            scrolled.set_hexpand(true);
            return scrolled;
        }
    };

    let unresolved: Vec<_> = state
        .conflicts
        .values()
        .filter(|c| c.resolved_at.is_none())
        .collect();

    if unresolved.is_empty() {
        let empty = Label::new(Some("No unresolved conflicts."));
        empty.add_css_class("empty-state");
        empty.set_halign(Align::Start);
        container.append(&empty);
    } else {
        let summary = Label::new(Some(&format!(
            "{} unresolved conflict(s)",
            unresolved.len()
        )));
        summary.add_css_class("muted");
        summary.set_halign(Align::Start);
        summary.set_margin_bottom(8);
        container.append(&summary);

        for c in &unresolved {
            let card = build_conflict_card(config, c);
            container.append(&card);
        }
    }

    let scrolled = ScrolledWindow::new();
    scrolled.set_child(Some(&container));
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);
    scrolled
}

fn build_conflict_card(config: &Config, record: &cloud_sync::state::ConflictRecord) -> GtkBox {
    let card = GtkBox::new(Orientation::Vertical, 6);
    card.add_css_class("conflict-card");

    let header_row = GtkBox::new(Orientation::Horizontal, 8);
    let repo_label = Label::new(Some(&record.repository_id));
    repo_label.add_css_class("repo-name");
    header_row.append(&repo_label);

    let date_label = Label::new(Some(
        &record.detected_at.format("%Y-%m-%d %H:%M").to_string(),
    ));
    date_label.add_css_class("muted");
    header_row.append(&date_label);

    // Resolution buttons sit at the right of the header row.
    let button_row = GtkBox::new(Orientation::Horizontal, 6);
    button_row.set_hexpand(true);
    button_row.set_halign(Align::End);
    for (label, strategy) in [
        ("Use Local", "local"),
        ("Use Remote", "remote"),
        ("Use Newest", "newest"),
        ("Keep Both", "keep-both"),
    ] {
        let button = Button::with_label(label);
        button.add_css_class("small-button");
        let conflict_id = record.id.clone();
        let strategy_owned = strategy.to_owned();
        button.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            let _ = std::process::Command::new("cloud-sync")
                .args(["conflict", "resolve", &conflict_id, &strategy_owned])
                .status();
        });
        button_row.append(&button);
    }
    header_row.append(&button_row);
    card.append(&header_row);

    for path in &record.paths {
        let path_label = Label::new(Some(path));
        path_label.add_css_class("conflict-path");
        path_label.set_halign(Align::Start);
        card.append(&path_label);
    }

    // Show the raw diff between remote and local for the conflicting paths.
    if let Ok(repository) = config.repository(&record.repository_id) {
        let mut args = vec![
            "diff",
            "--no-color",
            "--no-ext-diff",
            &record.remote_commit,
            &record.local_commit,
        ];
        if !record.paths.is_empty() {
            args.push("--");
            args.extend(record.paths.iter().map(String::as_str));
        }
        if let Ok(diff_text) = cloud_sync::git::run(&repository.path, args)
            && !diff_text.is_empty()
        {
            let diff_view = TextView::new();
            diff_view.set_editable(false);
            diff_view.set_monospace(true);
            diff_view.add_css_class("diff-view");
            diff_view.buffer().set_text(&diff_text);
            diff_view.set_vexpand(false);

            let diff_scroll = ScrolledWindow::new();
            diff_scroll.set_child(Some(&diff_view));
            diff_scroll.set_min_content_height(150);
            diff_scroll.set_max_content_height(300);
            diff_scroll.set_margin_top(8);
            card.append(&diff_scroll);
        }
    }

    card
}
