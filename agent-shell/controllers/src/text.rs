//! Pango markup text escaping (S1-07, "Escaping" in the sprint 1 spec).
//!
//! User and network text is shown in GTK only through `set_label` with `set_use_markup(false)`
//! or through [`escape_markup`]; calling `set_markup` directly with such text is forbidden.

/// Replaces `& < > " '` with the entities `&amp; &lt; &gt; &quot; &#39;`.
pub fn escape_markup(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}
