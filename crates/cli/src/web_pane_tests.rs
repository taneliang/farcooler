use clap::Parser;
use farcooler_protocol::v1 as pb;

use super::*;

fn parsed(args: &[&str]) -> OpenUrl {
    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        open: OpenUrl,
    }
    Cli::try_parse_from(std::iter::once("open-url").chain(args.iter().copied())).expect("parses").open
}

fn no_panes(given: &str) -> Result<bytes::Bytes, String> {
    Err(format!("no terminal matching {given:?}"))
}

/// The split the runner is asked for: the `web` preset, the page, and the
/// capability that makes an older runner refuse rather than run `web`.
#[test]
fn open_url_asks_for_a_web_split_that_an_older_runner_refuses() {
    let open = parsed(&["billing", "https://linear.app/acme/issue/ENG-12", "--side", "bottom"]);
    let mut update = LayoutUpdate::default();
    fill(&open, &mut update, no_panes).expect("an https page is opened");
    assert_eq!(update.command_preset, "web");
    assert_eq!(update.url.as_deref(), Some("https://linear.app/acme/issue/ENG-12"));
    assert_eq!(update.side, pb::SplitSide::Bottom as i32);
    assert_eq!(update.target, None, "beside the focused pane");

    let request = required(true, Request::default());
    assert_eq!(request.required_capabilities, vec!["web_pane".to_string()]);
    assert!(required(false, Request::default()).required_capabilities.is_empty(), "only open-url asks");
}

/// Anything but http and https is refused here, before the runner is asked.
#[test]
fn open_url_refuses_other_schemes_before_asking() {
    for url in ["file:///etc/passwd", "javascript:alert(1)", "data:text/html,hi", "linear://issue/1", "github.com", ""] {
        let mut update = LayoutUpdate::default();
        let said = fill(&parsed(&["billing", url]), &mut update, no_panes).expect_err(url);
        assert!(said.contains("http or https"), "{url}: {said}");
        assert_eq!(update, LayoutUpdate::default(), "{url}: nothing was filled in");
    }
}

/// A named pane is the split's target, resolved as `layout split` resolves it.
#[test]
fn open_url_beside_a_named_pane() {
    let open = parsed(&["billing", "https://github.com/", "3fa1"]);
    let mut update = LayoutUpdate::default();
    fill(&open, &mut update, |given| Ok(bytes::Bytes::copy_from_slice(given.as_bytes()))).unwrap();
    assert_eq!(update.target.as_deref(), Some(b"3fa1".as_slice()));
}

/// **`worktree list --json`'s terminal is the shape the Mac decodes**: a web
/// pane's mode and page. `test/fixtures/web-pane-terminal.json` is this output
/// byte for byte, and the Mac's `WebPaneTests` decode the same file. Rewrite
/// it with `FARCOOLER_WRITE_FIXTURES=1` after a deliberate change.
#[test]
fn a_web_panes_json_is_the_shape_the_mac_reads() {
    let t = pb::Terminal {
        id: bytes::Bytes::copy_from_slice(uuid::Uuid::from_u128(0x0435).as_bytes()),
        title: "Web".into(),
        command_preset: "web".into(),
        current_command: "web".into(),
        state: pb::TerminalState::Running as i32,
        epoch: 1,
        pane_mode: pb::PaneMode::Web as i32,
        web_url: Some("https://github.com/".into()),
        ..Default::default()
    };
    let out = serde_json::to_string_pretty(&crate::worktree_list_terminal_json(&t)).unwrap() + "\n";
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/web-pane-terminal.json");
    if std::env::var_os("FARCOOLER_WRITE_FIXTURES").is_some() {
        std::fs::write(&path, &out).unwrap();
    }
    let fixture = std::fs::read_to_string(&path).expect("test/fixtures/web-pane-terminal.json is missing");
    assert_eq!(out, fixture, "a web pane's JSON no longer matches the fixture the Mac decodes");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["paneMode"], "web");
    assert_eq!(v["webUrl"], "https://github.com/");
}
