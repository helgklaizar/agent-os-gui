//! What the "Files" editor opens: only `AGENT.md` and `memory/**/*.md`. `keys.env`, `permissions.json`,
//! personas, modules and other `~/.agentos/` files are unavailable; the check is made both before the request and against the core's list.

/// The path is allowed for the editor.
pub fn is_editable_path(path: &str) -> bool {
    if path == "AGENT.md" {
        return true;
    }
    let Some(rest) = path.strip_prefix("memory/") else {
        return false;
    };
    path.len() <= 200
        && rest.ends_with(".md")
        && !path.contains(['\\', '\0'])
        && rest
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != ".." && !p.starts_with('~'))
}

#[cfg(test)]
mod tests {
    use super::is_editable_path;

    #[test]
    fn allows_agent_md_and_memory_notes() {
        assert!(is_editable_path("AGENT.md"));
        assert!(is_editable_path("memory/notes.md"));
        assert!(is_editable_path("memory/a/b.md"));
    }

    #[test]
    fn refuses_everything_else() {
        for bad in [
            "keys.env",
            "permissions.json",
            "personas.json",
            "modules.json",
            "agent.md",
            "AGENT.md/",
            "/AGENT.md",
            "memory/",
            "memory/keys.env",
            "memory/../keys.env",
            "memory/./a.md",
            "memory//a.md",
            "memory\\a.md",
            "../memory/a.md",
            "memory/a.md\0",
            "~/.agentos/keys.env",
            "memorys/a.md",
        ] {
            assert!(!is_editable_path(bad), "{bad:?}");
        }
    }
}
