use cloud_sync::config::{Config, RepositoryConfig};
use cloud_sync::git;
use cloud_sync::paths::AppPaths;
use gtk4::prelude::*;
use gtk4::{
    self, Align, Box as GtkBox, Button, DropDown, Label, Orientation, ScrolledWindow, StringList,
};

pub fn build(config: &Config, _paths: &AppPaths) -> ScrolledWindow {
    let container = GtkBox::new(Orientation::Vertical, 8);
    container.set_margin_start(16);
    container.set_margin_end(16);
    container.set_margin_top(8);
    container.set_margin_bottom(16);

    let header_row = GtkBox::new(Orientation::Horizontal, 8);
    let header = Label::new(Some("History"));
    header.add_css_class("panel-header");
    header.set_halign(Align::Start);
    header.set_hexpand(true);
    header_row.append(&header);
    container.append(&header_row);

    let content_box = GtkBox::new(Orientation::Vertical, 0);
    content_box.set_vexpand(true);

    if config.repositories.is_empty() {
        let empty = Label::new(Some("No repositories registered."));
        empty.add_css_class("empty-state");
        content_box.append(&empty);
    } else {
        let names: Vec<&str> = config.repositories.iter().map(|r| r.id.as_str()).collect();
        let string_list = StringList::new(&names);
        let selector = DropDown::new(Some(string_list), gtk4::Expression::NONE);
        selector.set_halign(Align::Start);
        selector.set_margin_bottom(8);
        container.append(&selector);

        let repos: Vec<RepositoryConfig> = config.repositories.clone();
        let repos_for_selector = repos.clone();
        let content_for_cb = content_box.clone();
        selector.connect_selected_notify(move |dropdown| {
            let index = dropdown.selected() as usize;
            if let Some(repo) = repos_for_selector.get(index) {
                while let Some(child) = content_for_cb.first_child() {
                    content_for_cb.remove(&child);
                }
                populate_tree(&content_for_cb, repo);
            }
        });

        // Refresh button at the top right of the header.
        let refresh = Button::with_label("Refresh");
        refresh.add_css_class("small-button");
        let repos_for_refresh = repos.clone();
        let selector_for_refresh = selector.clone();
        let content_for_refresh = content_box.clone();
        refresh.connect_clicked(move |_| {
            let index = selector_for_refresh.selected() as usize;
            if let Some(repo) = repos_for_refresh.get(index) {
                while let Some(child) = content_for_refresh.first_child() {
                    content_for_refresh.remove(&child);
                }
                populate_tree(&content_for_refresh, repo);
            }
        });
        header_row.append(&refresh);

        if let Some(repo) = config.repositories.first() {
            populate_tree(&content_box, repo);
        }
    }

    container.append(&content_box);
    let scrolled = ScrolledWindow::new();
    scrolled.set_child(Some(&container));
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);
    scrolled
}

fn populate_tree(container: &GtkBox, repo: &RepositoryConfig) {
    // Fetch first so remote-tracking branches (other devices' sessions)
    // appear in the graph.
    let _ = git::run(&repo.path, ["fetch", "--all", "--prune"]);
    let log = git::run(
        &repo.path,
        [
            "log",
            "--all",
            "--graph",
            "--date=relative",
            "--pretty=format:%s%x1f%cr%x1f%D",
            "-n",
            "200",
        ],
    );

    match log {
        Ok(text) => {
            if text.trim().is_empty() {
                let empty = Label::new(Some("No commits yet."));
                empty.add_css_class("empty-state");
                empty.set_halign(Align::Start);
                container.append(&empty);
                return;
            }
            for line in text.lines() {
                container.append(&build_row(line));
            }
        }
        Err(e) => {
            let err = Label::new(Some(&format!("Could not load history: {e}")));
            err.add_css_class("repo-status-error");
            container.append(&err);
        }
    }
}

fn build_row(line: &str) -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 8);
    row.set_margin_start(2);
    row.set_margin_top(2);
    row.set_margin_bottom(2);

    // Split the graph gutter (leading *, |, /, \, space) from the fields.
    let graph_end = line
        .char_indices()
        .take_while(|(_, ch)| matches!(ch, '*' | '|' | '/' | '\\' | ' ' | '_'))
        .map(|(i, ch)| i + ch.len_utf8())
        .last()
        .unwrap_or(0);
    let graph_part = &line[..graph_end];
    let fields = &line[graph_end..];

    let gutter = Label::new(Some(graph_part.trim_end()));
    gutter.add_css_class("commit-graph");
    gutter.set_width_chars(graph_part.trim_end().chars().count().max(2) as i32);
    gutter.set_halign(Align::Start);
    gutter.set_valign(Align::Center);
    row.append(&gutter);

    // Fields are separated by \x1f: subject, relative date, decorations.
    let mut parts = fields.split('\u{1f}');
    let subject = parts.next().unwrap_or("").trim();
    let date = parts.next().unwrap_or("").trim();
    let decorations = parts.next().unwrap_or("").trim();

    if !subject.is_empty() {
        let message = Label::new(Some(subject));
        message.add_css_class("commit-message");
        message.set_halign(Align::Start);
        message.set_hexpand(true);
        message.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        message.set_max_width_chars(60);
        row.append(&message);
    }

    let mut seen_chips = Vec::new();
    for decoration in decorations.split(',') {
        let name = decoration.trim().trim_start_matches("HEAD ->").trim();
        if name.is_empty() || name == "HEAD" {
            continue;
        }
        // Only show chips for device refs and tags; regular branch names
        // (main, origin/…) are noise in this view.
        if !name.starts_with("cloud-sync/") && !name.starts_with("tag:") {
            continue;
        }
        if let Some(short) = shorten_ref_name(name) {
            if seen_chips.contains(&short) {
                continue;
            }
            seen_chips.push(short.clone());
            let chip = Label::new(Some(&short));
            if name.starts_with("tag:") {
                chip.add_css_class("tag-label");
            } else {
                chip.add_css_class("branch-label");
            }
            chip.set_valign(Align::Center);
            row.append(&chip);
        }
    }

    if !date.is_empty() {
        let when = Label::new(Some(date));
        when.add_css_class("commit-meta");
        when.set_halign(Align::End);
        row.append(&when);
    }

    row
}

/// Turn long internal ref names into short, meaningful labels.
///
/// - `cloud-sync/active/nitro/abc123`    → `nitro`
/// - `cloud-sync/completed/x/def456`     → `x snapshot`
/// - `cloud-sync/recovery/nitro/…`       → `nitro backup`
/// - `origin/main`                       → `main`
/// - `tag: v0.3.0`                        → `v0.3.0`
fn shorten_ref_name(name: &str) -> Option<String> {
    if let Some(rest) = name.strip_prefix("cloud-sync/active/") {
        return rest.split('/').next().map(str::to_owned);
    }
    if let Some(rest) = name.strip_prefix("cloud-sync/completed/") {
        return rest
            .split('/')
            .next()
            .map(|device| format!("{device} snapshot"));
    }
    if let Some(rest) = name.strip_prefix("cloud-sync/recovery/") {
        return rest
            .split('/')
            .next()
            .map(|device| format!("{device} backup"));
    }
    let name = name.strip_prefix("tag:").map(str::trim).unwrap_or(name);
    let name = name
        .strip_prefix("origin/")
        .or_else(|| name.strip_prefix("upstream/"))
        .unwrap_or(name);
    Some(name.to_owned())
}
