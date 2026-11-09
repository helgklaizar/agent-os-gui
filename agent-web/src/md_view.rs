//! Markdown model to DOM. Text enters the page only as text nodes; HTML from messages is not expressible by the
//! model (`controllers::markdown`), links are clickable only with `http`, `https`, `mailto` schemes, a jump to a foreign address needs confirmation.

use controllers::markdown::{Block, Inline};
use leptos::prelude::*;

fn inline_view(i: Inline) -> AnyView {
    match i {
        Inline::Text(t) => t.into_any(),
        Inline::Code(t) => view! { <code>{t}</code> }.into_any(),
        Inline::Strong(t) => view! { <strong>{t}</strong> }.into_any(),
        Inline::Emph(t) => view! { <em>{t}</em> }.into_any(),
        Inline::Link {
            text,
            url: Some(url),
        } => {
            let target = url.clone();
            view! {
                <a href=url target="_blank" rel="noopener noreferrer"
                    on:click=move |ev| {
                        if !confirm_leave(&target) {
                            ev.prevent_default();
                        }
                    }>{text}</a>
            }
            .into_any()
        }
        Inline::Link { text, url: None } => view! { <span>{text}</span> }.into_any(),
    }
}

fn inlines(v: Vec<Inline>) -> Vec<AnyView> {
    v.into_iter().map(inline_view).collect()
}

fn block_view(b: Block) -> AnyView {
    match b {
        Block::Paragraph(i) => view! { <p>{inlines(i)}</p> }.into_any(),
        Block::Heading { level, inlines: i } => {
            let c = inlines(i);
            match level {
                1 => view! { <h3>{c}</h3> }.into_any(),
                2 => view! { <h4>{c}</h4> }.into_any(),
                _ => view! { <h5>{c}</h5> }.into_any(),
            }
        }
        Block::Code { lang, text } => {
            let lang = lang.unwrap_or_default();
            view! { <pre data-lang=lang><code>{text}</code></pre> }.into_any()
        }
        Block::List { ordered, items } => {
            let li: Vec<AnyView> = items
                .into_iter()
                .map(|i| view! { <li>{inlines(i)}</li> }.into_any())
                .collect();
            if ordered {
                view! { <ol>{li}</ol> }.into_any()
            } else {
                view! { <ul>{li}</ul> }.into_any()
            }
        }
    }
}

pub fn blocks_view(blocks: Vec<Block>) -> Vec<AnyView> {
    blocks.into_iter().map(block_view).collect()
}

/// The link leads away from the client page (foreign origin or `mailto:`) and needs a warning.
pub fn warn_needed(url: &str, own_origin: &str) -> bool {
    let url = url.to_ascii_lowercase();
    let Some((scheme, rest)) = url.split_once("://") else {
        return true;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    format!("{scheme}://{authority}") != own_origin.to_ascii_lowercase()
}

/// Confirmation of a jump to a foreign address: `false` means stay on the page.
fn confirm_leave(url: &str) -> bool {
    let Some(w) = web_sys::window() else {
        return false;
    };
    let own = w.location().origin().unwrap_or_default();
    if !warn_needed(url, &own) {
        return true;
    }
    w.confirm_with_message(&format!("Go to an external address?\n{url}"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::warn_needed;

    const OWN: &str = "http://198.51.100.2:4096";

    #[test]
    fn foreign_origin_and_mailto_warn() {
        assert!(warn_needed("https://example.com/x", OWN));
        assert!(warn_needed("http://198.51.100.2:4097/x", OWN), "other port");
        assert!(
            warn_needed("https://198.51.100.2:4096/x", OWN),
            "other scheme"
        );
        assert!(
            warn_needed("http://198.51.100.2:4096@evil.example/", OWN),
            "userinfo is not origin"
        );
        assert!(warn_needed("mailto:a@example.com", OWN));
        assert!(warn_needed("HTTPS://Example.COM", OWN));
    }

    #[test]
    fn own_origin_does_not_warn() {
        assert!(!warn_needed("http://198.51.100.2:4096/app", OWN));
        assert!(!warn_needed("http://198.51.100.2:4096", OWN));
        assert!(!warn_needed("HTTP://198.51.100.2:4096/?q=1#a", OWN));
    }
}
