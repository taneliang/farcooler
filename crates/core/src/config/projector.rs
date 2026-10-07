//! `[agents] projector`: whether this runner folds terminal-mode agent
//! sessions into rows and serves `agent.rows` (ov-372).
//!
//! ```toml
//! [agents]
//! projector = true
//! ```
//!
//! Off when absent. A client's settings write it through
//! `settings.set_projector`, which also turns the projector on or off in the
//! running daemon; the daemon reads it once more when it starts. The
//! `FARCOOLER_PROJECTOR=1` environment still turns it on regardless.

use std::path::Path;

#[derive(Debug, Default, serde::Deserialize)]
struct AgentsOnly {
    #[serde(default)]
    agents: Agents,
}

#[derive(Debug, Default, serde::Deserialize)]
struct Agents {
    #[serde(default)]
    projector: bool,
}

/// `[agents] projector`, false for an absent or malformed file.
pub fn projector_from(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else { return false };
    match toml::from_str::<AgentsOnly>(&text) {
        Ok(parsed) => parsed.agents.projector,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "ignoring a malformed config file");
            false
        }
    }
}

/// The runner's `[agents] projector`, found the way the registry is.
pub fn load_projector() -> bool {
    super::config_path().is_some_and(|path| projector_from(&path))
}

/// Set `[agents] projector`, creating the file if it does not exist. Off
/// removes the key, so the file says nothing it doesn't need to.
pub fn write_projector(path: &Path, on: bool) -> std::io::Result<()> {
    let Some(mut doc) = super::document_for_edit(path)? else { return Err(super::malformed(path)) };
    let agents = doc
        .entry("agents")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "`agents` exists but is not a table"))?;
    if on {
        agents.insert("projector", toml_edit::value(true));
    } else {
        agents.remove("projector");
        if agents.is_empty() {
            doc.remove("agents");
        }
    }
    super::save(path, &doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("farcooler-projector-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p.join("config.toml")
    }

    #[test]
    fn on_off_and_a_hand_edit_beside_it_survives() {
        let path = scratch("on-off");
        assert!(!projector_from(&path), "absent is off");
        std::fs::write(&path, "# mine\n[branches]\nprefix = \"elt/\"\n").unwrap();
        write_projector(&path, true).unwrap();
        assert!(projector_from(&path));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# mine") && text.contains("prefix = \"elt/\""), "{text}");
        write_projector(&path, false).unwrap();
        assert!(!projector_from(&path));
        assert!(!std::fs::read_to_string(&path).unwrap().contains("agents"), "off leaves no husk");
    }

    #[test]
    fn a_malformed_file_is_off_and_never_overwritten() {
        let path = scratch("malformed");
        std::fs::write(&path, "[agents\nprojector = true").unwrap();
        assert!(!projector_from(&path));
        assert!(write_projector(&path, true).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[agents\nprojector = true");
    }
}
