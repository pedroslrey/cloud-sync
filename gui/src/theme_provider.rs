use cloud_sync::theme::Base16Scheme;
use gtk4::gdk;

/// Generate a GTK CSS stylesheet from a base16 color scheme.
pub fn css_for_scheme(scheme: &Base16Scheme) -> String {
    format!(
        r#"
/* base16 theme: {} */
window, .background {{
    background-color: {base00};
    color: {base05};
}}

.sidebar {{
    background-color: {base01};
    border-right: 1px solid {base02};
}}

.sidebar button {{
    background: transparent;
    border: none;
    border-radius: 4px;
    padding: 8px 12px;
    color: {base04};
    font-weight: normal;
}}

.sidebar button:hover {{
    background-color: {base02};
    color: {base05};
}}

.sidebar button:checked,
.sidebar button.active {{
    background-color: {base02};
    color: {base0d};
    font-weight: bold;
}}

.panel-header {{
    font-size: 18px;
    font-weight: bold;
    color: {base05};
    padding: 12px 16px 8px;
}}

.repo-card {{
    background-color: {base01};
    border-radius: 6px;
    padding: 12px;
    margin: 4px 0;
}}

.repo-name {{
    font-weight: bold;
    color: {base05};
}}

.repo-path {{
    font-size: 12px;
    color: {base03};
}}

.repo-status-ok {{
    color: {base0b};
}}

.repo-status-syncing {{
    color: {base0d};
}}

.repo-status-conflict {{
    color: {base08};
}}

.repo-status-error {{
    color: {base09};
}}

.conflict-card {{
    background-color: {base01};
    border-radius: 6px;
    border-left: 3px solid {base08};
    padding: 12px;
    margin: 4px 0;
}}

.conflict-path {{
    font-family: monospace;
    color: {base05};
}}

.diff-view {{
    font-family: monospace;
    font-size: 13px;
    background-color: {base00};
    color: {base05};
    padding: 8px;
}}

.diff-add {{
    color: {base0b};
}}

.diff-remove {{
    color: {base08};
}}

.diff-header {{
    color: {base0d};
    font-weight: bold;
}}

.commit-graph {{
    font-family: monospace;
    color: {base0d};
}}

.commit-node {{
    color: {base0d};
}}

.commit-message {{
    color: {base05};
}}

.commit-meta {{
    font-size: 12px;
    color: {base03};
}}

.branch-label {{
    background-color: {base0d};
    color: {base00};
    border-radius: 3px;
    padding: 1px 6px;
    font-size: 11px;
    font-weight: bold;
}}

.tag-label {{
    background-color: {base0a};
    color: {base00};
    border-radius: 3px;
    padding: 1px 6px;
    font-size: 11px;
    font-weight: bold;
}}

.setting-label {{
    color: {base04};
    font-size: 13px;
}}

.setting-value {{
    color: {base05};
    font-weight: bold;
}}

.action-button {{
    background-color: {base0d};
    color: {base00};
    border: none;
    border-radius: 4px;
    padding: 6px 16px;
    font-weight: bold;
}}

.action-button:hover {{
    background-color: {base0c};
}}

.small-button {{
    background-color: {base02};
    color: {base05};
    border: none;
    border-radius: 4px;
    padding: 3px 10px;
    font-size: 12px;
    font-weight: normal;
}}

.small-button:hover {{
    background-color: {base03};
    color: {base07};
}}

.danger-button {{
    background-color: {base08};
    color: {base07};
    border: none;
    border-radius: 4px;
    padding: 6px 16px;
    font-weight: bold;
}}

.danger-button:hover {{
    background-color: {base09};
}}

.muted {{
    color: {base03};
}}

.empty-state {{
    color: {base03};
    font-size: 14px;
    padding: 32px;
}}

scrolledwindow {{
    background-color: transparent;
}}

textview, textview text {{
    background-color: {base00};
    color: {base05};
    font-family: monospace;
}}
"#,
        scheme.name,
        base00 = scheme.base00,
        base01 = scheme.base01,
        base02 = scheme.base02,
        base03 = scheme.base03,
        base04 = scheme.base04,
        base05 = scheme.base05,
        base07 = scheme.base07,
        base08 = scheme.base08,
        base09 = scheme.base09,
        base0a = scheme.base0a,
        base0b = scheme.base0b,
        base0c = scheme.base0c,
        base0d = scheme.base0d,
    )
}

/// Load a CSS provider with the given scheme and apply it to the default display.
pub fn apply_scheme(scheme: &Base16Scheme) {
    let css = css_for_scheme(scheme);
    let provider = gtk4::CssProvider::new();
    #[allow(deprecated)]
    provider.load_from_data(&css);
    if let Some(display) = gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
