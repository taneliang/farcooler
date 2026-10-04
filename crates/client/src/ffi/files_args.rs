//! The Files calls, as an app passes them (ov-259): `worktree.file_search`,
//! `worktree.list_dir` and `worktree.read_file`.

use serde_json::{Value, json};
use uuid::Uuid;

use crate::session::{FilesPlace, Session, SessionError};

/// `{worktree?, folder?, path?}`: exactly one of `worktree` and `folder`.
///
/// Refused here, before the wire, for both and for neither: a call naming
/// both would read one place while the screen showed the other, and one
/// naming neither is a bug in the app that a silent default would hide.
pub(super) fn place_of(method: &str, args: &Value) -> Result<(FilesPlace, String), SessionError> {
    let worktree = args.get("worktree").and_then(Value::as_str).filter(|s| !s.is_empty());
    let folder = args.get("folder").and_then(Value::as_str).filter(|s| !s.is_empty());
    let place = match (worktree, folder) {
        (Some(id), None) => FilesPlace::Worktree(
            id.parse::<Uuid>().map_err(|_| SessionError::Protocol(format!("{method} needs a worktree")))?,
        ),
        (None, Some(name)) => FilesPlace::Folder(name.to_string()),
        _ => return Err(SessionError::Protocol(format!("{method} needs a worktree or a folder, and not both"))),
    };
    let path = args.get("path").and_then(Value::as_str).unwrap_or_default().to_string();
    Ok((place, path))
}

/// One of the three Files methods. `method` is one `dispatch` matched.
pub(super) async fn call(session: &Session, method: &str, args: &Value) -> Result<Value, SessionError> {
    match method {
        "worktree.list_dir" => {
            let (place, path) = place_of(method, args)?;
            session.list_dir(&place, &path).await
        }
        "worktree.read_file" => {
            let (place, path) = place_of(method, args)?;
            session.read_file(&place, &path).await
        }
        _ => {
            let worktree = args
                .get("worktree")
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<Uuid>().ok())
                .ok_or_else(|| SessionError::Protocol(format!("{method} needs a worktree")))?;
            let query = args.get("query").and_then(Value::as_str).unwrap_or_default();
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20) as u32;
            let paths = session.search_worktree_files(worktree, query, limit).await?;
            Ok(json!({ "paths": paths }))
        }
    }
}

/// `Host.read_only_folders` by name, or None from a runner without the
/// `read_only_folders` capability: it sent none because it cannot, not because
/// it has none. The path stays on the runner: a phone shows names.
pub(super) fn read_only_folders(host: &farcooler_protocol::v1::Host, reports: bool) -> Option<Vec<&str>> {
    reports.then(|| host.read_only_folders.iter().map(|f| f.name.as_str()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_one_place_is_named() {
        let wt = Uuid::now_v7();
        let ok = place_of("m", &json!({ "worktree": wt.to_string(), "path": "src" })).unwrap();
        assert_eq!(ok, (FilesPlace::Worktree(wt), "src".to_string()));
        let ok = place_of("m", &json!({ "folder": "logs" })).unwrap();
        assert_eq!(ok, (FilesPlace::Folder("logs".into()), String::new()));
        assert!(place_of("m", &json!({ "worktree": wt.to_string(), "folder": "logs" })).is_err());
        assert!(place_of("m", &json!({ "path": "src" })).is_err());
        assert!(place_of("m", &json!({ "worktree": "not-a-uuid" })).is_err());
    }

    #[test]
    fn host_names_the_folders_only_when_the_runner_reports_them() {
        let host = farcooler_protocol::v1::Host {
            read_only_folders: vec![farcooler_protocol::v1::ReadOnlyFolder { name: "logs".into(), path: "/var/log".into() }],
            ..Default::default()
        };
        assert_eq!(read_only_folders(&host, true), Some(vec!["logs"]));
        assert_eq!(read_only_folders(&host, false), None);
    }
}
