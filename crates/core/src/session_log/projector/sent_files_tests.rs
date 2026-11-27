//! The files an agent sends (`SendUserFile`, ov-1509): on their tool row, in
//! the Markdown that names them, and read back only from what was sent.

use serde_json::{Value, json};

use super::fixtures::{Scratch, fold, tool};
use super::sent_files::{AUTO_SHOW_BYTES, MAX_SENT_FILE_BYTES, ReadRefused, read};
use super::{Projection, RowKind};

fn prompt() -> String {
    json!({"type":"user","uuid":"u1","promptId":"p1","promptSource":"typed","timestamp":"2026-11-27T10:00:00.000Z","message":{"role":"user","content":"show me"}}).to_string()
}

fn says(uuid: &str, text: &str) -> String {
    json!({"type":"assistant","uuid":uuid,"timestamp":"2026-11-27T10:00:02.000Z","message":{"id":uuid,"role":"assistant","content":[{"type":"text","text":text}]}}).to_string()
}

fn call(id: &str, name: &str, input: Value) -> String {
    json!({"type":"assistant","uuid":format!("a-{id}"),"timestamp":"2026-11-27T10:00:03.000Z","message":{"id":format!("m-{id}"),"role":"assistant","content":[{"type":"tool_use","id":id,"name":name,"input":input}]}}).to_string()
}

fn answer(id: &str, failed: bool, result: Value) -> String {
    json!({"type":"user","uuid":format!("r-{id}"),"timestamp":"2026-11-27T10:00:04.000Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":id,"is_error":failed,"content":"1 file delivered to user."}]},"toolUseResult":result}).to_string()
}

/// A send of a capture and a report, as claude 2.1.332 records one: the
/// resolved paths in `attachments`, the paths as asked in the input.
fn send(id: &str) -> [String; 2] {
    [
        call(id, "SendUserFile", json!({"files":["shots/board 1.png","/tmp/out/report.pdf"],"caption":"The board","status":"normal"})),
        answer(id, false, json!({"caption":"The board","attachments":[
            {"path":"/work/shots/board 1.png","size":48213,"isImage":true,"media_type":"image/png"},
            {"path":"/tmp/out/report.pdf","size":2_400_000,"isImage":false},
        ],"sentAt":"2026-11-27T10:00:04.000Z"})),
    ]
}

fn folded(lines: &[String]) -> Projection {
    fold(&lines.join("\n"))
}

fn sent_json(p: &Projection, id: &str) -> Value {
    serde_json::to_value(&p.row(id).expect("the row").kind).expect("json")["Tool"]["sent"].clone()
}

#[test]
fn a_send_puts_its_files_on_the_row_and_keeps_their_paths_on_the_runner() {
    let [asked, done] = send("t1");
    let p = folded(&[prompt(), asked, done]);
    assert_eq!(
        sent_json(&p, "tool:t1"),
        json!({"caption":"The board","files":[
            {"name":"board 1.png","mime":"image/png","size":48213,"size_words":"48 KB","image":true,"auto":true,"fetchable":true},
            {"name":"report.pdf","mime":"application/pdf","size":2400000,"size_words":"2.4 MB","image":false,"auto":false,"fetchable":true},
        ]})
    );
    assert!(!serde_json::to_string(&p.row("tool:t1").unwrap().kind).unwrap().contains("/work/"), "no path leaves the runner");
    let file = p.sent_file("tool:t1", 0).expect("the capture");
    assert_eq!(file.path(), std::path::Path::new("/work/shots/board 1.png"));
    assert!(p.sent_file("tool:t1", 2).is_none());
    assert!(p.sent_file("turn:p1", 0).is_none());
}

#[test]
fn only_a_send_claude_reports_delivered_puts_files_on_a_row() {
    let attachments = json!({"attachments":[{"path":"/etc/passwd","size":10,"isImage":false}]});
    let p = folded(&[
        prompt(),
        // Asked and not yet answered: the input's paths are not sends.
        call("t1", "SendUserFile", json!({"files":["/etc/passwd"]})),
        call("t2", "SendUserFile", json!({"files":["/etc/passwd"]})),
        answer("t2", true, attachments.clone()),
        // Another tool whose result happens to carry the same shape.
        call("t3", "mcp__evil__send", json!({"files":["/etc/passwd"]})),
        answer("t3", false, attachments),
    ]);
    for id in ["tool:t1", "tool:t2", "tool:t3"] {
        assert!(tool(&p, id).sent.is_none(), "{id}");
        assert!(p.sent_file(id, 0).is_none(), "{id}");
    }
}

#[test]
fn a_relative_or_unnamed_attachment_is_not_a_file() {
    let p = folded(&[
        prompt(),
        call("t1", "SendUserFile", json!({"files":["a"]})),
        answer("t1", false, json!({"attachments":[{"path":"../secrets","size":1},{"path":"","size":1},{"size":1},{"path":"/ok/a.txt","size":1}]})),
    ]);
    let sent = tool(&p, "tool:t1").sent.as_ref().expect("sent");
    assert_eq!(sent.files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["a.txt"]);
}

#[test]
fn a_large_image_waits_for_a_click_and_one_past_the_cap_is_not_offered() {
    let p = folded(&[
        prompt(),
        call("t1", "SendUserFile", json!({"files":["/a/big.png","/a/huge.zip","/a/logo.svg"]})),
        answer("t1", false, json!({"attachments":[
            {"path":"/a/big.png","size":AUTO_SHOW_BYTES + 1,"isImage":true},
            {"path":"/a/huge.zip","size":MAX_SENT_FILE_BYTES + 1,"isImage":false},
            // SVG can carry script: it is a file to download, never drawn.
            {"path":"/a/logo.svg","size":100,"isImage":true},
        ]})),
    ]);
    let files = &tool(&p, "tool:t1").sent.as_ref().expect("sent").files;
    assert_eq!(files.iter().map(|f| (f.image, f.auto, f.fetchable)).collect::<Vec<_>>(), [(true, false, true), (false, false, false), (false, false, true)]);
}

fn images_of(p: &Projection, text: &str) -> Value {
    let row = p.rows().iter().find(|r| matches!(&r.kind, RowKind::Prose(t) if t.text.contains(text))).expect("the prose");
    serde_json::to_value(&row.kind).expect("json")["Prose"]["images"].clone()
}

#[test]
fn markdown_that_names_a_sent_image_carries_it_whichever_came_first() {
    let [asked, done] = send("t1");
    let p = folded(&[
        prompt(),
        // Before the send: by the resolved path, in angle brackets for its space.
        says("m1", "before ![board](</work/shots/board 1.png>)"),
        asked,
        done,
        // After it: as the agent asked for it, and as a file URL with a title.
        says("m2", "after ![board](<shots/board 1.png>) and ![report](/tmp/out/report.pdf)"),
        says("m3", "never sent ![key](/work/shots/other.png) ![web](https://example.com/x.png)"),
    ]);
    let board = |src: &str| json!([{"src":src,"row":"tool:t1","index":0,"name":"board 1.png","mime":"image/png","size":48213,"size_words":"48 KB","image":true,"auto":true,"fetchable":true}]);
    assert_eq!(images_of(&p, "before"), board("/work/shots/board 1.png"));
    // The report is not an image, so its Markdown stays alt text.
    assert_eq!(images_of(&p, "after"), board("shots/board 1.png"));
    assert_eq!(images_of(&p, "never sent"), Value::Null);
}

#[test]
fn a_follower_hears_of_a_row_whose_image_was_sent_after_it() {
    let [asked, done] = send("t1");
    let mut p = folded(&[prompt(), says("m1", "![b](/work/shots/board%201.png)")]);
    let rev = p.revision();
    for line in [asked, done] {
        p.fold_line(line.as_bytes());
    }
    let prose = p.rows().iter().find(|r| matches!(r.kind, RowKind::Prose(_))).unwrap();
    assert!(prose.rev > rev, "the row moved, so a follow sends it again");
    // A percent-encoded destination names the same file.
    assert_eq!(images_of(&p, "![b]")[0]["src"], "/work/shots/board%201.png");
}

#[test]
fn a_sent_file_is_read_in_pieces_and_only_while_it_is_the_file_that_was_sent() {
    let dir = Scratch::new("sent");
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let path = root.join("a.txt");
    std::fs::write(&path, b"0123456789").unwrap();
    let projected = |path: &std::path::Path, size: u64| {
        folded(&[prompt(), call("t1", "SendUserFile", json!({"files":["a.txt"]})), answer("t1", false, json!({"attachments":[{"path":path,"size":size}]}))])
    };
    let p = projected(&path, 10);
    let file = p.sent_file("tool:t1", 0).unwrap();
    assert_eq!(read(file, 0, 4), Ok(b"0123".to_vec()));
    assert_eq!(read(file, 8, 4), Ok(b"89".to_vec()));
    assert_eq!(read(file, 20, 4), Ok(Vec::new()));

    // Rewritten to another size: not what was sent.
    std::fs::write(&path, b"changed").unwrap();
    assert_eq!(read(file, 0, 4), Err(ReadRefused::Changed));
    // Replaced by a link to something else of the same size.
    let secret = root.join("secret");
    std::fs::write(&secret, b"abcdefghij").unwrap();
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&secret, &path).unwrap();
    assert_eq!(read(file, 0, 4), Err(ReadRefused::Gone));
    // Or by a hard link to it.
    std::fs::remove_file(&path).unwrap();
    std::fs::hard_link(&secret, &path).unwrap();
    assert_eq!(read(file, 0, 4), Err(ReadRefused::Gone));
    std::fs::remove_file(&path).unwrap();
    assert_eq!(read(file, 0, 4), Err(ReadRefused::Gone));
    std::fs::create_dir(&path).unwrap();
    assert_eq!(read(file, 0, 4), Err(ReadRefused::Gone));

    // A link on the way is fine (macOS's /tmp is one) until it is pointed
    // somewhere else after the send.
    for (folder, words) in [("first", b"0123456789"), ("second", b"abcdefghij")] {
        std::fs::create_dir_all(root.join(folder)).unwrap();
        std::fs::write(root.join(folder).join("b.txt"), words).unwrap();
    }
    let inner = root.join("inner");
    std::os::unix::fs::symlink(root.join("first"), &inner).unwrap();
    let p = projected(&inner.join("b.txt"), 10);
    let file = p.sent_file("tool:t1", 0).unwrap();
    assert_eq!(read(file, 0, 4), Ok(b"0123".to_vec()));
    std::fs::remove_file(&inner).unwrap();
    std::os::unix::fs::symlink(root.join("second"), &inner).unwrap();
    assert_eq!(read(file, 0, 4), Err(ReadRefused::Gone));

    let p = projected(&secret, MAX_SENT_FILE_BYTES + 1);
    assert_eq!(read(p.sent_file("tool:t1", 0).unwrap(), 0, 4), Err(ReadRefused::TooLarge));
}
