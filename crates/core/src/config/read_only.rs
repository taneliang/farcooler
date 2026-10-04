//! The `[files.read_only]` table: extra folders the file viewer may read
//! (ov-232).
//!
//! ```toml
//! [files.read_only]
//! logs = "/var/log"
//! ```
//!
//! A name, which is what a client asks for, and an absolute path, which only
//! this file says. This is the only place a folder is added: no request can
//! name a path, and none of the writers below this file's settings editor
//! uses (`write_branch_prefix`, `write_theme`, `write_adapter`) touches this
//! table. The daemon reads it once, when it starts
//! (`farcoolerd`'s `read_only_folders::load`), so even a write to this file
//! from elsewhere changes nothing until a restart.
//!
//! What the paths are checked against, and how a link is treated, is the
//! daemon's business, not this parser's: here a path is only text.

use std::path::Path;

/// The read side only: everything else in the file is ignored here, and this
/// table is ignored by `ConfigFile`, so a mistake in one costs nothing in the
/// other.
#[derive(Debug, Default, serde::Deserialize)]
struct FilesOnly {
    #[serde(default)]
    files: Files,
}

#[derive(Debug, Default, serde::Deserialize)]
struct Files {
    /// Values as TOML values rather than strings, so one entry that isn't a
    /// string is skipped and reported rather than costing the whole table.
    #[serde(default)]
    read_only: std::collections::BTreeMap<String, toml::Value>,
}

/// `[files.read_only]`, as written: each name and its path, in name order.
/// An entry whose value isn't a string is reported and left out.
pub fn read_only_folders_from(path: &Path) -> Vec<(String, String)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let parsed: FilesOnly = match toml::from_str(&text) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "ignoring a malformed config file");
            return Vec::new();
        }
    };
    parsed
        .files
        .read_only
        .into_iter()
        .filter_map(|(name, value)| match value {
            toml::Value::String(folder) => Some((name, folder)),
            _ => {
                tracing::warn!(folder = %name, "ignoring a read-only folder whose path isn't a string");
                None
            }
        })
        .collect()
}

/// The runner's `[files.read_only]`, found the way the registry is.
pub fn load_read_only_folders() -> Vec<(String, String)> {
    match super::config_path() {
        Some(path) => read_only_folders_from(&path),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config file holding `text`, in a directory of its own per test.
    fn config(tag: &str, text: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("farcooler-read-only-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn the_table_is_read_by_name() {
        let path = config("by-name", "[files.read_only]\nlogs = \"/var/log\"\napp = \"/srv/app/logs\"\n");
        assert_eq!(
            read_only_folders_from(&path),
            vec![("app".to_string(), "/srv/app/logs".to_string()), ("logs".to_string(), "/var/log".to_string())]
        );
    }

    #[test]
    fn absent_file_or_table_is_none() {
        let path = config("absent", "[branches]\nprefix = \"elt/\"\n");
        assert!(read_only_folders_from(&path).is_empty());
        assert!(read_only_folders_from(&path.with_file_name("nothing.toml")).is_empty());
    }

    #[test]
    fn one_bad_entry_costs_only_itself_and_the_rest_of_the_file_still_reads() {
        let path =
            config("bad-entry", "[files.read_only]\nlogs = \"/var/log\"\nbad = [\"/etc\"]\n\n[branches]\nprefix = \"elt/\"\n");
        assert_eq!(read_only_folders_from(&path), vec![("logs".to_string(), "/var/log".to_string())]);
        // And the table, even malformed, doesn't cost the settings beside it.
        let path = config("bad-table", "[files]\nread_only = [\"/etc\"]\n\n[branches]\nprefix = \"elt/\"\n");
        assert!(read_only_folders_from(&path).is_empty());
        assert_eq!(super::super::branch_prefix_from(&path), "elt/");
    }

    /// The writers a settings editor reaches over the wire each edit their own
    /// table, and none of them can add a folder, whatever name it is handed.
    #[test]
    fn no_settings_writer_adds_or_changes_a_folder() {
        let path = config("writers", "[files.read_only]\nlogs = \"/var/log\"\n");
        let before = read_only_folders_from(&path);
        super::super::write_branch_prefix(&path, "files.read_only").unwrap();
        let mut theme = crate::theme::default_theme();
        for name in ["files", "files.read_only", "read_only", "x\"]\n[files.read_only]\nhome = \"/"] {
            theme.name = name.to_string();
            super::super::write_theme(&path, &theme).unwrap();
            super::super::write_adapter(&path, name, &super::super::AdapterTable::default()).unwrap();
        }
        assert_eq!(read_only_folders_from(&path), before);
    }
}
