//! UI icons (line, 24x24, `currentColor` stroke). The content is static paths, user text never gets here.

use leptos::prelude::*;

/// Names of all icons the view uses.
pub const ALL: &[&str] = &[
    "bell",
    "chat",
    "bot",
    "task",
    "msg",
    "plus",
    "folder-plus",
    "folder",
    "term",
    "settings",
    "search",
    "clock",
    "journal",
    "clip",
    "send",
    "stop",
    "check",
    "x",
    "pencil",
    "trash",
    "thread",
    "move",
    "eraser",
    "chev-down",
    "chev-right",
];

/// Inside of `<svg>` by name; an unknown name gives an empty string.
pub fn path(name: &str) -> &'static str {
    match name {
        "bell" => {
            r##"<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/>"##
        }
        "chat" => r##"<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>"##,
        "bot" => {
            r##"<path d="M12 8V4H8"/><rect width="16" height="12" x="4" y="8" rx="2"/><path d="M2 14h2M20 14h2M15 13v2M9 13v2"/>"##
        }
        "task" => {
            r##"<path d="M9 11l3 3L22 4"/><path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11"/>"##
        }
        "msg" => r##"<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/>"##,
        "plus" => r##"<path d="M5 12h14"/><path d="M12 5v14"/>"##,
        "folder-plus" => {
            r##"<path d="M12 10v6"/><path d="M9 13h6"/><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"##
        }
        "folder" => {
            r##"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"##
        }
        "term" => r##"<polyline points="4 17 10 11 4 5"/><line x1="12" x2="20" y1="19" y2="19"/>"##,
        "settings" => {
            r##"<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>"##
        }
        "search" => r##"<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>"##,
        "clock" => r##"<circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/>"##,
        "journal" => r##"<path d="M4 4h16v16H4z"/><path d="M8 9h8M8 13h8M8 17h5"/>"##,
        "clip" => {
            r##"<path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48"/>"##
        }
        "send" => r##"<path d="M5 12h14"/><path d="m12 5 7 7-7 7"/>"##,
        "stop" => r##"<rect width="12" height="12" x="6" y="6" rx="2"/>"##,
        "check" => r##"<path d="M20 6 9 17l-5-5"/>"##,
        "x" => r##"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"##,
        "pencil" => {
            r##"<path d="M12 20h9"/><path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z"/>"##
        }
        "trash" => {
            r##"<path d="M3 6h18"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>"##
        }
        "thread" => {
            r##"<path d="M3 11h18"/><path d="M7 11v8a2 2 0 0 0 2 2h8"/><path d="M3 5h18"/>"##
        }
        "move" => {
            r##"<path d="M2 9V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2v-1"/><path d="M2 13h10"/><path d="m9 16 3-3-3-3"/>"##
        }
        "eraser" => {
            r##"<path d="m7 21-4.3-4.3c-1-1-1-2.5 0-3.4l9.6-9.6c1-1 2.5-1 3.4 0l5.6 5.6c1 1 1 2.5 0 3.4L13 21"/><path d="M22 21H7"/><path d="m5 11 9 9"/>"##
        }
        "chev-down" => r##"<path d="m6 9 6 6 6-6"/>"##,
        "chev-right" => r##"<path d="m9 6 6 6-6 6"/>"##,
        _ => "",
    }
}

#[component]
pub fn Icon(name: &'static str, #[prop(default = 18)] size: u32) -> impl IntoView {
    debug_assert!(ALL.contains(&name), "no icon {name}");
    view! {
        <svg class="li" width=size height=size viewBox="0 0 24 24" fill="none" stroke="currentColor"
            stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" inner_html=path(name)></svg>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_has_path_data() {
        assert!(ALL.len() >= 14, "too few icons: {}", ALL.len());
        for n in ALL {
            let p = path(n);
            assert!(
                p.contains("d=\"")
                    || p.contains("points=")
                    || p.contains("cx=")
                    || p.contains("width="),
                "no data for {n}"
            );
        }
    }

    #[test]
    fn unknown_icon_is_empty() {
        assert_eq!(path("no-such"), "");
    }

    #[test]
    fn icon_markup_is_inert() {
        for n in ALL {
            let p = path(n);
            assert!(
                !p.contains("script") && !p.contains(" on"),
                "active content in {n}"
            );
            for tag in p.split('<').skip(1) {
                let name = tag.split([' ', '/', '>']).next().unwrap_or("");
                assert!(
                    ["path", "circle", "rect", "line", "polyline"].contains(&name),
                    "tag {name} in {n}"
                );
            }
        }
    }
}
