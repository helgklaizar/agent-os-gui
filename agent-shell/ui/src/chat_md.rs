//! Markdown model (`controllers::markdown`) -> GTK widgets. All text goes through `escape_markup`; links are already
//! filtered by scheme (`safe_url`), `url = None` is shown as plain text.

use controllers::markdown::{Block, Inline};
use controllers::text::escape_markup;
use gtk4 as gtk;
use gtk4::prelude::*;

fn inline_markup(parts: &[Inline]) -> String {
    let mut out = String::new();
    for p in parts {
        match p {
            Inline::Text(t) => out.push_str(&escape_markup(t)),
            Inline::Code(t) => out.push_str(&format!("<tt>{}</tt>", escape_markup(t))),
            Inline::Strong(t) => out.push_str(&format!("<b>{}</b>", escape_markup(t))),
            Inline::Emph(t) => out.push_str(&format!("<i>{}</i>", escape_markup(t))),
            Inline::Link {
                text,
                url: Some(url),
            } => out.push_str(&format!(
                "<a href=\"{}\">{}</a>",
                escape_markup(url),
                escape_markup(text)
            )),
            Inline::Link { text, url: None } => out.push_str(&escape_markup(text)),
        }
    }
    out
}

fn text_label(css: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_selectable(true);
    label.set_max_width_chars(70);
    label.add_css_class(css);
    label
}

/// A paragraph with markup built only from escaped parts.
fn rich_label(parts: &[Inline], css: &str) -> gtk::Label {
    let label = text_label(css);
    label.set_use_markup(true);
    label.set_markup(&inline_markup(parts));
    label
}

pub(crate) fn block_widget(block: &Block) -> gtk::Widget {
    match block {
        Block::Paragraph(parts) => rich_label(parts, "chat-p").upcast(),
        Block::Heading { inlines, .. } => rich_label(inlines, "chat-h").upcast(),
        Block::Code { text, .. } => {
            let label = text_label("chat-code");
            label.set_use_markup(false);
            label.set_label(text);
            label.upcast()
        }
        Block::List { ordered, items } => {
            let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
            for (i, item) in items.iter().enumerate() {
                let mark = if *ordered {
                    format!("{}. ", i + 1)
                } else {
                    "• ".to_string()
                };
                let mut parts = vec![Inline::Text(mark)];
                parts.extend(item.iter().cloned());
                list.append(&rich_label(&parts, "chat-li"));
            }
            list.upcast()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(text: &str, url: Option<&str>) -> Inline {
        Inline::Link {
            text: text.into(),
            url: url.map(str::to_string),
        }
    }

    #[test]
    fn markup_escapes_every_text_part() {
        let hostile = "<b>x</b> & \"q\" 'a'";
        let parts = [
            Inline::Text(hostile.into()),
            Inline::Code(hostile.into()),
            Inline::Strong(hostile.into()),
            Inline::Emph(hostile.into()),
        ];
        let out = inline_markup(&parts);
        assert!(!out.contains("<b>x"), "raw tag from text: {out}");
        assert_eq!(out.matches("&lt;b&gt;x&lt;/b&gt;").count(), 4, "{out}");
        assert_eq!(out.matches("<tt>").count(), 1);
        assert_eq!(out.matches("<i>").count(), 1);
    }

    #[test]
    fn markup_link_href_and_text_are_escaped() {
        let out = inline_markup(&[link("<i>t</i>", Some("https://a.b/?x=1&y=\"2\""))]);
        assert_eq!(
            out,
            "<a href=\"https://a.b/?x=1&amp;y=&quot;2&quot;\">&lt;i&gt;t&lt;/i&gt;</a>"
        );
    }

    #[test]
    fn markup_link_without_url_is_plain_text() {
        let out = inline_markup(&[link("<a href=\"x\">", None)]);
        assert!(
            !out.contains("<a "),
            "link without url became a link: {out}"
        );
        assert_eq!(out, "&lt;a href=&quot;x&quot;&gt;");
    }
}
