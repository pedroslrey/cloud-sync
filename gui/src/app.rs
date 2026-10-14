use cloud_sync::config::Config;
use cloud_sync::paths::AppPaths;
use cloud_sync::theme::ThemeConfig;
use gtk4::prelude::*;
use gtk4::{
    self, Align, Application, ApplicationWindow, Box as GtkBox, Label, Orientation, Stack,
    ToggleButton,
};

use crate::panels;
use crate::theme_provider;

pub fn build(app: &Application) {
    let paths = match AppPaths::discover() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: could not discover paths: {e}");
            return;
        }
    };
    let _ = paths.ensure();

    let config = Config::load(&paths).unwrap_or_default();
    let theme_config = ThemeConfig::load(&paths).unwrap_or_default();

    // Apply the active theme
    theme_provider::apply_scheme(theme_config.active_scheme());

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Cloud Sync")
        .default_width(900)
        .default_height(600)
        .build();

    let root = GtkBox::new(Orientation::Horizontal, 0);

    // Left sidebar
    let sidebar = GtkBox::new(Orientation::Vertical, 2);
    sidebar.add_css_class("sidebar");
    sidebar.set_width_request(180);
    sidebar.set_margin_top(8);
    sidebar.set_margin_bottom(8);
    sidebar.set_margin_start(8);
    sidebar.set_margin_end(0);

    let title = Label::new(Some("Cloud Sync"));
    title.add_css_class("panel-header");
    title.set_halign(Align::Start);
    title.set_margin_bottom(12);
    sidebar.append(&title);

    // Content stack
    let stack = Stack::new();
    stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    stack.set_transition_duration(150);

    // Build panels
    let folders_panel = panels::folders::build(&config, &paths);
    stack.add_named(&folders_panel, Some("folders"));

    let conflicts_panel = panels::conflicts::build(&config, &paths);
    stack.add_named(&conflicts_panel, Some("conflicts"));

    let tree_panel = panels::tree::build(&config, &paths);
    stack.add_named(&tree_panel, Some("tree"));

    let settings_panel = panels::settings::build(&config, &paths);
    stack.add_named(&settings_panel, Some("settings"));

    // Sidebar buttons — use GTK's native toggle group so exactly one is active.
    let buttons = [
        ("Folders", "folders"),
        ("Conflicts", "conflicts"),
        ("History", "tree"),
        ("Settings", "settings"),
    ];

    let mut group_leader: Option<ToggleButton> = None;
    for (label, page) in &buttons {
        let button = ToggleButton::with_label(label);
        button.set_halign(Align::Fill);
        if let Some(leader) = &group_leader {
            button.set_group(Some(leader));
        } else {
            group_leader = Some(button.clone());
        }
        let stack_ref = stack.clone();
        let page_name = page.to_string();
        button.connect_toggled(move |btn| {
            if btn.is_active() {
                stack_ref.set_visible_child_name(&page_name);
            }
        });
        sidebar.append(&button);
    }

    if let Some(first) = group_leader {
        first.set_active(true);
    }

    root.append(&sidebar);
    root.append(&stack);

    window.set_child(Some(&root));
    window.present();
}
