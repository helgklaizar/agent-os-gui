//! Look checks: the stylesheet matches the page CSP policy and holds both themes and all columns.

#[cfg(test)]
mod tests {
    const CSS: &str = include_str!("../style.css");

    #[test]
    fn css_obeys_csp() {
        // style-src 'self': no external addresses, no @import, no data URIs in styles.
        for bad in ["@import", "url(http", "url(//", "url(data:", "expression("] {
            assert!(!CSS.contains(bad), "forbidden in style.css: {bad}");
        }
    }

    #[test]
    fn css_has_both_themes_and_gradient_wall() {
        assert!(CSS.contains("prefers-color-scheme: dark"));
        // The light theme is the default values in :root, the dark one is the media block; both have their own gradient background.
        assert_eq!(
            CSS.matches("--wall:").count(),
            2,
            "both the light and the dark theme need a background"
        );
        assert!(CSS.contains("radial-gradient"), "no gradient background");
    }

    #[test]
    fn css_styles_four_columns_and_icons() {
        for sel in [
            ".notes",
            ".tree",
            ".chat",
            ".tools",
            ".stripe-task",
            ".stripe-bad",
            ".send",
            ".li",
            ".ico-btn",
            ".bar",
        ] {
            assert!(CSS.contains(sel), "no rule {sel}");
        }
    }
}
