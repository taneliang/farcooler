//! An extra read-only folder is read with every check a worktree gets, is
//! named only by the runner's config, and can't be widened to a secret.

use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use farcooler_core::DomainError;
use farcooler_protocol::v1 as pb;

use super::*;
use crate::worktree_files::{list_in, read_in, Base, Content, Refusal};

/// A home with an `.ssh`, a runtime directory, a `logs` folder beside them,
/// and a secret outside the folder.
struct Scene {
    _dir: tempfile::TempDir,
    top: PathBuf,
    guarded: Guarded,
}

impl Scene {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let top = dir.path().to_path_buf();
        for d in ["home/.ssh", "runtime", "logs/app", "outside"] {
            std::fs::create_dir_all(top.join(d)).unwrap();
        }
        std::fs::write(top.join("home/.ssh/id_ed25519"), "PRIVATE KEY\n").unwrap();
        std::fs::write(top.join("logs/app/today.log"), "started\n").unwrap();
        std::fs::write(top.join("outside/secret"), "SECRET\n").unwrap();
        let guarded = Guarded { home: Some(top.join("home")), runtime: top.join("runtime") };
        Scene { _dir: dir, top, guarded }
    }

    fn path(&self, rel: &str) -> String {
        self.top.join(rel).to_string_lossy().into_owned()
    }

    fn logs(&self) -> Folder {
        admit("logs", &self.path("logs"), &self.guarded).expect("logs is a folder")
    }
}

fn text(content: Content) -> String {
    match content {
        Content::Text { text, .. } => text,
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn a_listed_folder_is_readable() {
    let scene = Scene::new();
    let logs = scene.logs();
    let names: Vec<String> = list_in(Base::Folder(logs.real()), "").unwrap().entries.into_iter().map(|e| e.name).collect();
    assert_eq!(names, vec!["app"]);
    assert_eq!(text(read_in(Base::Folder(logs.real()), "app/today.log").unwrap()), "started\n");
}

#[test]
fn dot_dot_and_absolute_paths_are_refused_in_a_folder() {
    let scene = Scene::new();
    let logs = scene.logs();
    let base = Base::Folder(logs.real());
    for bad in ["../outside/secret", "app/../../outside/secret", "/etc/hosts", "./app/today.log"] {
        assert_eq!(read_in(base, bad), Err(Refusal::NotRelative), "{bad}");
    }
    assert_eq!(list_in(base, ".."), Err(Refusal::NotRelative));
    assert_eq!(list_in(base, "../home/.ssh"), Err(Refusal::NotRelative));
}

#[test]
fn a_link_inside_a_folder_pointing_out_is_refused() {
    let scene = Scene::new();
    let logs = scene.logs();
    let base = Base::Folder(logs.real());
    symlink(scene.top.join("home/.ssh"), scene.top.join("logs/keys")).unwrap();
    symlink("../../outside", scene.top.join("logs/app/up")).unwrap();
    assert_eq!(read_in(base, "keys/id_ed25519"), Err(Refusal::NotFound));
    assert_eq!(list_in(base, "keys").map(|l| l.entries), Err(Refusal::NotFound));
    assert_eq!(read_in(base, "app/up/secret"), Err(Refusal::NotFound));
    // A link at the end is answered as a link, its target never read.
    let key = scene.top.join("home/.ssh/id_ed25519");
    symlink(&key, scene.top.join("logs/innocent.log")).unwrap();
    assert_eq!(read_in(base, "innocent.log"), Ok(Content::Link { target: key.to_string_lossy().into_owned() }));
}

#[test]
fn an_unlisted_name_or_any_path_is_no_folder() {
    let scene = Scene::new();
    let folders = vec![scene.logs()];
    assert!(find(&folders, "logs").is_some());
    for asked in ["", "Logs", "logs/", "logs/..", "..", "/", "/etc", scene.path("logs").as_str(), scene.path("outside").as_str()] {
        assert!(find(&folders, asked).is_none(), "{asked}");
    }
}

/// The folder is the directory the link named at startup. A link changed
/// later moves nothing, and the folder itself swapped for a link afterwards
/// is refused, not followed: every request walks the real path from `/`
/// without following a link.
#[test]
fn a_configured_link_is_resolved_once_and_never_followed_again() {
    let scene = Scene::new();
    let link = scene.top.join("logs-link");
    symlink(scene.top.join("logs"), &link).unwrap();
    let folder = admit("logs", &link.to_string_lossy(), &scene.guarded).unwrap();
    assert_eq!(folder.real(), std::fs::canonicalize(scene.top.join("logs")).unwrap());
    assert_eq!(folder.configured, link.to_string_lossy());
    let base = Base::Folder(folder.real());
    assert_eq!(text(read_in(base, "app/today.log").unwrap()), "started\n");

    // The configured link repointed at a secret: the folder doesn't move.
    std::fs::remove_file(&link).unwrap();
    symlink(scene.top.join("home/.ssh"), &link).unwrap();
    assert_eq!(read_in(base, "id_ed25519"), Err(Refusal::NotFound));
    assert_eq!(text(read_in(base, "app/today.log").unwrap()), "started\n");

    // The real folder itself replaced by a link to the secret: refused.
    std::fs::rename(scene.top.join("logs"), scene.top.join("logs-was")).unwrap();
    symlink(scene.top.join("home/.ssh"), scene.top.join("logs")).unwrap();
    assert_eq!(read_in(base, "id_ed25519"), Err(Refusal::NotFound));
    assert_eq!(list_in(base, "").map(|l| l.entries), Err(Refusal::NotFound));
}

#[test]
fn a_configured_link_into_a_secret_or_too_broad_a_folder_is_refused() {
    let scene = Scene::new();
    let g = &scene.guarded;
    let link = scene.top.join("keys-link");
    symlink(scene.top.join("home/.ssh"), &link).unwrap();
    assert_eq!(admit("keys", &link.to_string_lossy(), g), Err(Refused::Secret));
    assert_eq!(admit("keys", &scene.path("home/.ssh"), g), Err(Refused::Secret));
    assert_eq!(admit("state", &scene.path("runtime"), g), Err(Refused::TooBroad));
    std::fs::create_dir_all(scene.top.join("runtime/worktrees")).unwrap();
    assert_eq!(admit("state", &scene.path("runtime/worktrees"), g), Err(Refused::Secret));
    assert_eq!(admit("home", &scene.path("home"), g), Err(Refused::TooBroad));
    assert_eq!(admit("top", &scene.path(""), g), Err(Refused::TooBroad));
    assert_eq!(admit("root", "/", g), Err(Refused::TooBroad));
    let up = scene.top.join("up-link");
    symlink(&scene.top, &up).unwrap();
    assert_eq!(admit("up", &up.to_string_lossy(), g), Err(Refused::TooBroad));
    // A home given through a link is guarded as the real directory.
    let home_link = scene.top.join("home-link");
    symlink(scene.top.join("home"), &home_link).unwrap();
    let via = Guarded { home: Some(home_link), runtime: g.runtime.clone() };
    assert_eq!(admit("keys", &scene.path("home/.ssh"), &via), Err(Refused::Secret));
}

#[test]
fn a_bad_name_or_path_is_left_out() {
    let scene = Scene::new();
    let g = &scene.guarded;
    let logs = scene.path("logs");
    for name in ["", ".", "..", "a/b", "tab\there", "x".repeat(65).as_str()] {
        assert_eq!(admit(name, &logs, g), Err(Refused::BadName), "{name:?}");
    }
    assert_eq!(admit("logs", "logs", g), Err(Refused::NotAbsolute));
    assert_eq!(admit("logs", &scene.path("nothing"), g), Err(Refused::NotADirectory));
    assert_eq!(admit("logs", &scene.path("outside/secret"), g), Err(Refused::NotADirectory));
    let kept = resolve(
        vec![("logs".into(), logs), ("keys".into(), scene.path("home/.ssh")), ("rel".into(), "logs".into())],
        g,
    );
    assert_eq!(kept.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec!["logs"]);
}

/// Over the two methods: a folder by name is read; a path, or a name the
/// config doesn't list, is not found; a folder and a worktree at once is
/// refused; and the runner says which folders it has.
#[tokio::test]
async fn the_methods_read_a_folder_by_name_and_nothing_else() {
    let scene = Scene::new();
    let svc = crate::service::Service::open_in(scene.top.join("runtime")).await.unwrap().with_read_only_folders(vec![scene.logs()]);
    let dir = |folder: &str, path: &str| pb::WorktreeDirRequest { folder: folder.into(), path: path.into(), ..Default::default() };
    let file = |folder: &str, path: &str| pb::WorktreeFileRequest { folder: folder.into(), path: path.into(), ..Default::default() };

    let listed = crate::worktree_files::list_dir(&svc, &dir("logs", "app")).await.unwrap();
    assert_eq!(listed.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["today.log"]);
    assert_eq!(crate::worktree_files::read_file(&svc, &file("logs", "app/today.log")).await.unwrap().text, "started\n");

    for folder in ["outside", scene.path("outside").as_str(), scene.path("home/.ssh").as_str(), "..", "logs/.."] {
        assert!(matches!(crate::worktree_files::read_file(&svc, &file(folder, "secret")).await, Err(DomainError::NotFound)), "{folder}");
        assert!(matches!(crate::worktree_files::list_dir(&svc, &dir(folder, "")).await, Err(DomainError::NotFound)), "{folder}");
    }
    assert!(matches!(
        crate::worktree_files::read_file(&svc, &file("logs", "../outside/secret")).await,
        Err(DomainError::InvalidArgument { .. })
    ));
    let both = pb::WorktreeDirRequest { worktree_id: vec![1u8; 16].into(), folder: "logs".into(), ..Default::default() };
    assert!(matches!(crate::worktree_files::list_dir(&svc, &both).await, Err(DomainError::InvalidArgument { what: "folder" })));

    let host = crate::wire::host("t", svc.host_id, &svc.inventory_snapshot(), 0, None, false, None, Vec::new(), svc.read_only_folders());
    assert_eq!(host.read_only_folders, vec![pb::ReadOnlyFolder { name: "logs".into(), path: scene.path("logs") }]);
}

/// A client can't add or change a folder: the list is set by `Service::open`
/// from the config file and by a test's builder, and nothing in the daemon
/// writes it after. A setter, a `push`, or a `&mut` to it anywhere else fails
/// here. (And no settings write over the wire reaches `[files.read_only]`:
/// `farcooler_core::config`'s `no_settings_writer_adds_or_changes_a_folder`.)
#[test]
fn nothing_but_startup_sets_the_folders() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut writers = Vec::new();
    let mut stack = vec![src];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") || path.ends_with("read_only_folders/tests.rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            for line in source.lines() {
                let l = line.trim();
                let writes = ["read_only_folders =", "read_only_folders.push", "read_only_folders.extend", "read_only_folders.insert", "read_only_folders.retain", "read_only_folders.clear", "mut self.read_only_folders", "read_only_folders_mut", "read_only_folders.iter_mut", "read_only_folders.get_mut", "read_only_folders: Vec::new()", "RwLock<Vec<crate::read_only_folders", "Mutex<Vec<crate::read_only_folders"];
                if writes.iter().any(|w| l.contains(w)) {
                    writers.push(l.to_string());
                }
            }
        }
    }
    writers.sort();
    assert_eq!(
        writers,
        vec![
            "read_only_folders: Vec::new(),",
            "self.read_only_folders = folders;",
            "service.read_only_folders = crate::read_only_folders::load(&service.root);",
        ],
        "the folders are written only at startup"
    );
    // And no method is about folders but the two reads that take one by name.
    for method in farcooler_protocol::method::Method::ALL {
        assert!(!method.name().contains("folder"), "{} would be a way to change the folders", method.name());
    }
}
