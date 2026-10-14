use cloud_sync::config::Config;
use cloud_sync::paths::AppPaths;
use cloud_sync::theme::ThemeConfig;
use cloud_sync::updater;
use gtk4::prelude::*;
use gtk4::{
    self, Align, Box as GtkBox, Button, DropDown, Label, Orientation, ScrolledWindow, StringList,
};

use crate::theme_provider;

type NumericSetting = (
    &'static str,
    f64,
    f64,
    f64,
    fn(&mut cloud_sync::config::SyncSettings, u64),
    u64,
);

pub fn build(config: &Config, paths: &AppPaths) -> ScrolledWindow {
    let container = GtkBox::new(Orientation::Vertical, 12);
    container.set_margin_start(16);
    container.set_margin_end(16);
    container.set_margin_top(8);
    container.set_margin_bottom(16);

    let header = Label::new(Some("Settings"));
    header.add_css_class("panel-header");
    header.set_halign(Align::Start);
    container.append(&header);

    // Theme section
    let theme_section = GtkBox::new(Orientation::Vertical, 8);
    let theme_header = Label::new(Some("Theme"));
    theme_header.add_css_class("repo-name");
    theme_header.set_halign(Align::Start);
    theme_section.append(&theme_header);

    let theme_config = ThemeConfig::load(paths).unwrap_or_default();
    let names: Vec<&str> = theme_config.list_names();
    let active_index = names
        .iter()
        .position(|n| *n == theme_config.active)
        .unwrap_or(0);

    let string_list = StringList::new(&names);
    let theme_dropdown = DropDown::new(Some(string_list), gtk4::Expression::NONE);
    theme_dropdown.set_selected(active_index as u32);
    theme_dropdown.set_halign(Align::Start);

    let paths_clone = paths.clone();
    theme_dropdown.connect_selected_notify(move |dropdown| {
        let index = dropdown.selected() as usize;
        if let Ok(mut tc) = ThemeConfig::load(&paths_clone) {
            let names = tc
                .list_names()
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>();
            if let Some(name) = names.get(index)
                && tc.set_active(name).is_ok()
            {
                let _ = tc.save(&paths_clone);
                let scheme = tc.active_scheme().clone();
                theme_provider::apply_scheme(&scheme);
            }
        }
    });
    theme_section.append(&theme_dropdown);
    container.append(&theme_section);

    // Sync defaults section — editable values saved back to config.toml.
    let sync_section = GtkBox::new(Orientation::Vertical, 6);
    let sync_header = Label::new(Some("Sync Defaults"));
    sync_header.add_css_class("repo-name");
    sync_header.set_halign(Align::Start);
    sync_section.append(&sync_header);

    let saved_notice = Label::new(None);
    saved_notice.add_css_class("muted");
    saved_notice.set_halign(Align::Start);

    let numeric_settings: Vec<NumericSetting> = vec![
        (
            "Inactivity delay (seconds)",
            30.0,
            3600.0,
            config.defaults.inactivity_seconds as f64,
            |s, v| s.inactivity_seconds = v,
            30,
        ),
        (
            "Remote check interval (seconds)",
            60.0,
            86_400.0,
            config.defaults.remote_check_seconds as f64,
            |s, v| s.remote_check_seconds = v,
            60,
        ),
        (
            "Session retention (days)",
            1.0,
            365.0,
            config.defaults.completed_session_retention_days as f64,
            |s, v| s.completed_session_retention_days = v,
            1,
        ),
    ];

    for (label, min, max, current, setter, step) in &numeric_settings {
        let setter = *setter;
        let row = GtkBox::new(Orientation::Horizontal, 12);
        let key = Label::new(Some(*label));
        key.add_css_class("setting-label");
        key.set_halign(Align::Start);
        key.set_hexpand(true);
        row.append(&key);

        let spin = gtk4::SpinButton::with_range(*min, *max, *step as f64);
        spin.set_value(*current);
        spin.set_hexpand(false);
        spin.set_width_chars(8);
        let paths_rc = paths.clone();
        let notice = saved_notice.clone();
        spin.connect_value_changed(move |s| {
            let value = s.value() as u64;
            if let Ok(mut cfg) = Config::load(&paths_rc) {
                setter(&mut cfg.defaults, value);
                if cfg.save(&paths_rc).is_ok() {
                    notice.set_text("Saved");
                }
            }
        });
        row.append(&spin);
        sync_section.append(&row);
    }

    // Skip-binaries toggle
    let binary_row = GtkBox::new(Orientation::Horizontal, 12);
    let binary_key = Label::new(Some("Skip untracked binaries"));
    binary_key.add_css_class("setting-label");
    binary_key.set_halign(Align::Start);
    binary_key.set_hexpand(true);
    binary_row.append(&binary_key);
    let binary_toggle = gtk4::CheckButton::new();
    binary_toggle.set_active(config.defaults.skip_untracked_binaries);
    let paths_rc = paths.clone();
    let notice = saved_notice.clone();
    binary_toggle.connect_toggled(move |toggle| {
        if let Ok(mut cfg) = Config::load(&paths_rc) {
            cfg.defaults.skip_untracked_binaries = toggle.is_active();
            if cfg.save(&paths_rc).is_ok() {
                notice.set_text("Saved");
            }
        }
    });
    binary_row.append(&binary_toggle);
    sync_section.append(&binary_row);
    sync_section.append(&saved_notice);

    container.append(&sync_section);

    // Device section
    let device_section = GtkBox::new(Orientation::Vertical, 6);
    let device_header = Label::new(Some("Device"));
    device_header.add_css_class("repo-name");
    device_header.set_halign(Align::Start);
    device_section.append(&device_header);

    let device_row = GtkBox::new(Orientation::Horizontal, 12);
    let device_key = Label::new(Some("Device ID"));
    device_key.add_css_class("setting-label");
    device_key.set_halign(Align::Start);
    device_key.set_width_chars(24);
    device_row.append(&device_key);
    let device_val = Label::new(Some(&config.device_id));
    device_val.add_css_class("setting-value");
    device_row.append(&device_val);
    device_section.append(&device_row);
    container.append(&device_section);

    // Config file location — header row with the button at the right.
    let path_section = GtkBox::new(Orientation::Vertical, 6);
    let path_header_row = GtkBox::new(Orientation::Horizontal, 8);
    let path_header = Label::new(Some("Configuration"));
    path_header.add_css_class("repo-name");
    path_header.set_halign(Align::Start);
    path_header.set_hexpand(true);
    path_header_row.append(&path_header);

    let open_button = Button::with_label("Open folder");
    open_button.add_css_class("small-button");
    open_button.set_halign(Align::End);
    let config_dir = paths
        .config_file
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();
    open_button.connect_clicked(move |_| {
        let _ = std::process::Command::new("xdg-open")
            .arg(&config_dir)
            .spawn();
    });
    path_header_row.append(&open_button);
    path_section.append(&path_header_row);

    let config_path = Label::new(Some(&paths.config_file.display().to_string()));
    config_path.add_css_class("muted");
    config_path.set_halign(Align::Start);
    config_path.set_selectable(true);
    path_section.append(&config_path);

    let theme_path = Label::new(Some(&ThemeConfig::path(paths).display().to_string()));
    theme_path.add_css_class("muted");
    theme_path.set_halign(Align::Start);
    theme_path.set_selectable(true);
    path_section.append(&theme_path);
    container.append(&path_section);

    // Updates section — header row with the button at the right.
    let update_section = GtkBox::new(Orientation::Vertical, 6);
    update_section.set_margin_top(12);
    let update_header_row = GtkBox::new(Orientation::Horizontal, 8);
    let update_header = Label::new(Some("Updates"));
    update_header.add_css_class("repo-name");
    update_header.set_halign(Align::Start);
    update_header_row.append(&update_header);

    let version = Label::new(Some(&format!("Version: {}", env!("CARGO_PKG_VERSION"))));
    version.add_css_class("muted");
    version.set_halign(Align::Start);
    update_header_row.append(&version);

    let update_status = Label::new(Some(""));
    update_status.add_css_class("muted");
    update_status.set_halign(Align::End);
    update_status.set_hexpand(true);
    update_status.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    update_header_row.append(&update_status);

    let check_button = Button::with_label("Check");
    check_button.add_css_class("small-button");
    check_button.set_halign(Align::End);
    update_header_row.append(&check_button);
    update_section.append(&update_header_row);

    let status_label = update_status.clone();
    check_button.connect_clicked(move |button| {
        button.set_sensitive(false);
        status_label.set_text("Checking…");
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(updater::check());
        });
        let button = button.clone();
        let status_label = status_label.clone();
        gtk4::glib::spawn_future_local(async move {
            let result = loop {
                match receiver.try_recv() {
                    Ok(result) => break Some(result),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break None,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        gtk4::glib::timeout_future(std::time::Duration::from_millis(100)).await;
                    }
                }
            };
            button.set_sensitive(true);
            match result {
                Some(Ok(Some(update))) => status_label.set_text(&format!(
                    "Update available: {} — run: cloud-sync update install",
                    update.available
                )),
                Some(Ok(None)) => status_label.set_text("Up to date"),
                Some(Err(e)) => status_label.set_text(&format!("Check failed: {e}")),
                None => status_label.set_text("Check failed"),
            }
        });
    });
    container.append(&update_section);

    let scrolled = ScrolledWindow::new();
    scrolled.set_child(Some(&container));
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);
    scrolled
}
