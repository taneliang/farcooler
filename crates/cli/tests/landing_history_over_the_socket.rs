//! The CLI reads old landing history through the real daemon dispatch.
#[path = "../../daemon/tests/support/in_process.rs"]
mod in_process;

use farcooler_protocol::v1::Scope;
use farcooler_store::{models::Actor, testing::old_landing};
use in_process::*;
use serde_json::Value;

#[tokio::test]
async fn list_and_show_read_stored_history() {
    let h = start_legacy(Scope::Read).await;
    let repo = a_repository(&h);
    let task = h.service.store.create_task(repo.workspace, "Old landing", Actor::Manager).unwrap();
    old_landing(&h.service.store, repo.workspace, 373, "pushed", Some("abcdef1"), &[task.id], now_millis());
    let home = h.socket.parent().unwrap();
    std::os::unix::fs::symlink(&h.socket, home.join("farcoolerd.sock")).unwrap();
    for args in [&["list"][..], &["show", "373"][..]] {
        let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_farcooler"))
            .env("FARCOOLER_HOME", home)
            .env_remove("FARCOOLER_WORKSPACE")
            .env_remove("FARCOOLER_ACTOR")
            .args(["landing", "--json", "--repo", &repo.id.to_string(), "--workspace", &repo.workspace.to_string()])
            .args(args)
            .output().await.unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        let landing = if args[0] == "list" { &value["landings"][0] } else { &value };
        assert_eq!(landing["number"], 373);
        assert_eq!(landing["pushed_sha"], "abcdef1");
        if args[0] == "show" { assert!(landing["events"].is_array()); }
    }
}
