//! The owner's shell startup files exporting a Claude key (ov-651), against a
//! stand-in login shell: a script that exports what the test says an rc file
//! would, then runs the `-ilc` payload it was given in a real `sh`, so the
//! pipeline under test is the one that runs.

use std::os::unix::fs::PermissionsExt;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use farcooler_protocol::v1::{self as pb, Request, Scope, result};
use farcooler_transport::Peer;
use prost::Message as _;

use super::*;
use crate::test_support::ScratchDir;

/// The value the stand-in exports. Nothing may carry it out of the shell.
const SECRET: &str = "sk-ant-api03-SECRETVALUE0123456789";

/// One test at a time: they share log callsites with the log test.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A login shell that exports `exports` (`NAME=value` lines) before it runs
/// its `-ilc` payload, as an rc file would, and prints a greeting first.
fn stand_in_shell(dir: &std::path::Path, exports: &[&str]) -> String {
    let path = dir.join("stand-in-shell");
    let lines: String = exports.iter().map(|e| format!("export {e}\n")).collect();
    std::fs::write(&path, format!("#!/bin/sh\necho 'Welcome back'\n{lines}exec /bin/sh -c \"$2\"\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_string_lossy().into_owned()
}

fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[tokio::test]
async fn a_shell_that_exports_a_key_is_named_and_the_value_is_not() {
    let dir = scratch();
    let shell = stand_in_shell(dir.path(), &[&format!("ANTHROPIC_API_KEY={SECRET}"), "CLAUDE_CODE_OAUTH_TOKEN=oauth-VALUE", "UNRELATED=1"]);
    let found = ask(&shell).await;
    assert_eq!(found, ["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN"]);
    assert!(!format!("{found:?}").contains("VALUE"), "{found:?}");
}

#[tokio::test]
async fn a_shell_that_exports_nothing_names_nothing() {
    let dir = scratch();
    // An empty variable sets nothing a claude would use; a longer name that
    // only starts like one is not one.
    let shell = stand_in_shell(dir.path(), &["ANTHROPIC_AUTH_TOKEN=", "ANTHROPIC_API_KEY_FILE=/x", "MY_ANTHROPIC_API_KEY=y"]);
    assert_eq!(ask(&shell).await, Vec::<String>::new());
    assert_eq!(ask("/nonexistent/shell").await, Vec::<String>::new(), "a shell that can't run warns of nothing");
}

/// An interactive fish prints a terminal escape before the payload's first
/// line (measured: `ESC[6 q`); that line is still a name, and a value that
/// merely ends in one is not.
#[tokio::test]
async fn a_terminal_escape_before_the_first_name_is_ignored() {
    let dir = scratch();
    let path = dir.path().join("fishy");
    std::fs::write(&path, "#!/bin/sh\nprintf '\\033[6 qANTHROPIC_API_KEY\\nsk-ant-xCLAUDE_CODE_OAUTH_TOKEN\\nANTHROPIC_AUTH_TOKEN\\n'\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(ask(path.to_str().unwrap()).await, ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"]);
}

/// A rc file's `echo` that looks like a name is not a variable: only what the
/// pipeline prints counts, and it prints a name only for a set variable. A
/// stand-in that ignores the payload and prints names proves the daemon
/// takes exactly those lines, in the order of `NAMES`.
#[tokio::test]
async fn only_exact_names_are_taken_from_what_the_shell_prints() {
    let dir = scratch();
    let path = dir.path().join("noisy");
    std::fs::write(&path, "#!/bin/sh\necho 'ANTHROPIC_AUTH_TOKEN'\necho 'ANTHROPIC_API_KEY=oops'\necho ' ANTHROPIC_API_KEY'\necho 'CLAUDE_CODE_OAUTH_TOKEN'\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(ask(path.to_str().unwrap()).await, ["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_AUTH_TOKEN"]);
}

async fn runner(members: &[&str]) -> (ScratchDir, Arc<Service>, tokio::sync::MutexGuard<'static, ()>) {
    let serial = SERIAL.lock().await;
    let (dir, svc, _repo) = crate::test_support::fixture().await;
    for handle in members {
        svc.store.set_member(handle, handle, &[format!("phone-{handle}")]).unwrap();
    }
    (dir, svc, serial)
}

async fn asked(svc: &Service, shell: &str, members: bool) {
    svc.claude_logins.key_exports.refresh(members, shell.to_string());
    svc.claude_logins.key_exports.settled().await;
}

async fn host_get(svc: &Arc<Service>, client_id: Option<&str>) -> pb::Host {
    let factory = crate::rpc::RpcFactory::new(
        svc.clone(),
        crate::watch::Watcher::new(svc.clone()),
        Arc::new(tokio::sync::Notify::new()),
        Peer { client_id: client_id.map(str::to_string), scope: Scope::Read },
    );
    use farcooler_transport::Handler as _;
    let reply = factory.handle(Request { method: "host.get".into(), ..Default::default() }).await;
    match reply.outcome {
        Some(pb::response::Outcome::Result(r)) => match r.value {
            Some(result::Value::Host(host)) => host,
            other => panic!("not a host: {other:?}"),
        },
        other => panic!("no host: {other:?}"),
    }
}

/// The warning is the owner's, and only on a runner with members.
#[tokio::test(flavor = "multi_thread")]
async fn the_owner_of_a_runner_with_members_is_warned_and_no_one_else() {
    let (dir, svc, _turn) = runner(&["bob"]).await;
    let shell = stand_in_shell(dir.path(), &[&format!("ANTHROPIC_API_KEY={SECRET}")]);
    asked(&svc, &shell, true).await;
    assert_eq!(host_get(&svc, None).await.rc_claude_keys, ["ANTHROPIC_API_KEY"], "the owner's local socket");
    assert_eq!(host_get(&svc, Some("phone-bob")).await.rc_claude_keys, Vec::<String>::new(), "a member's phone is not told");
}

#[tokio::test(flavor = "multi_thread")]
async fn no_members_is_no_warning() {
    let (dir, svc, _turn) = runner(&[]).await;
    let shell = stand_in_shell(dir.path(), &[&format!("ANTHROPIC_API_KEY={SECRET}")]);
    // Asked anyway, say before the last member left: nothing is shown.
    asked(&svc, &shell, true).await;
    assert_eq!(host_get(&svc, None).await.rc_claude_keys, Vec::<String>::new());
    // A member arrives and the answer applies.
    svc.store.set_member("bob", "Bob", &[]).unwrap();
    assert_eq!(host_get(&svc, None).await.rc_claude_keys, ["ANTHROPIC_API_KEY"]);
    // The last one leaves, a refresh runs no shell and forgets, and the next
    // member starts with a fresh ask rather than the old answer.
    svc.store.remove_member("bob").unwrap();
    asked(&svc, &shell, false).await;
    svc.store.set_member("carol", "Carol", &[]).unwrap();
    assert_eq!(host_get(&svc, None).await.rc_claude_keys, Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_set_is_no_warning() {
    let (dir, svc, _turn) = runner(&["bob"]).await;
    asked(&svc, &stand_in_shell(dir.path(), &[]), true).await;
    assert_eq!(host_get(&svc, None).await.rc_claude_keys, Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn no_members_while_an_ask_runs_leaves_nothing() {
    let (dir, svc, _turn) = runner(&["bob"]).await;
    let shell = stand_in_shell(dir.path(), &["CLAUDE_CODE_OAUTH_TOKEN=x"]);
    svc.claude_logins.key_exports.refresh(true, shell.clone());
    svc.claude_logins.key_exports.refresh(false, shell);
    svc.claude_logins.key_exports.settled().await;
    assert_eq!(svc.claude_key_exports(), Vec::<String>::new(), "the later trigger said no members");
}

/// A stand-in that runs the payload in its own shell (`eval`, so the rc's
/// functions apply to it, as an rc's aliases do in a login shell), prints what
/// the rc files say on stderr, and copies exactly what the payload printed to
/// stdout into `raw`.
fn hostile_shell(dir: &std::path::Path, rc: &str) -> (String, std::path::PathBuf) {
    let path = dir.join("hostile-shell");
    let raw = dir.join("raw");
    std::fs::write(&path, format!("#!/bin/sh\n{rc}\neval \"$2\" | tee '{}'\n", raw.display())).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    (path.to_string_lossy().into_owned(), raw)
}

/// An rc that prints the value, shadows `env`, `printenv`, `sed` and `grep` with functions that print it, and
/// exports a variable
/// holding a newline and a forged name: the result is still just the name,
/// and the value is not among what the payload printed.
#[tokio::test]
async fn a_hostile_rc_cannot_put_the_value_in_stdout_or_forge_a_name() {
    let dir = scratch();
    let rc = format!(
        "export ANTHROPIC_API_KEY={SECRET}\n\
         echo \"$ANTHROPIC_API_KEY\" >&2\n\
         env() {{ echo \"ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY\"; }}\n\
         printenv() {{ echo \"$ANTHROPIC_API_KEY\"; }}\n\
         sed() {{ cat; }}\n\
         grep() {{ cat; }}\n\
         EVIL=\"x\nCLAUDE_CODE_OAUTH_TOKEN=forged\"; export EVIL"
    );
    let (shell, raw) = hostile_shell(dir.path(), &rc);
    assert_eq!(ask(&shell).await, ["ANTHROPIC_API_KEY"]);
    let printed = std::fs::read_to_string(raw).unwrap();
    assert!(!printed.contains("SECRETVALUE") && !printed.contains("forged"), "the payload printed {printed:?}");
    assert_eq!(printed, "ANTHROPIC_API_KEY\n");
}

/// A newline in another variable can't forge a name: nothing is set, so
/// nothing is reported.
#[tokio::test]
async fn a_newline_in_another_variable_forges_nothing() {
    let dir = scratch();
    let (shell, _raw) = hostile_shell(dir.path(), "EVIL=\"x\nANTHROPIC_API_KEY=forged\"; export EVIL");
    assert_eq!(ask(&shell).await, Vec::<String>::new());
}

/// The daemon's own environment may hold the variables (it was started from a
/// shell that set them); the shell a member's pane starts doesn't inherit
/// them, so neither does this one, and only an rc file can make one set.
#[tokio::test]
async fn variables_in_the_daemons_environment_are_not_reported() {
    let dir = scratch();
    let shell = stand_in_shell(dir.path(), &[]);
    let inherited = [("ANTHROPIC_API_KEY", SECRET), ("CLAUDE_CODE_OAUTH_TOKEN", "x"), ("ANTHROPIC_AUTH_TOKEN", "y")];
    assert_eq!(ask_inheriting(&shell, TIMEOUT, &inherited).await, Vec::<String>::new());
}

/// **The value is never in any program's argv** (the review's catch: zsh and
/// fish run `/bin/test` for `command test`). Every program the payload runs
/// that could take it is replaced by a wrapper that logs its arguments, put
/// first on PATH by the shell's own rc file (a login shell rebuilds PATH), under
/// the real zsh and fish, whose rc file also exports the key. Skipped, and said
/// so, for a shell that isn't installed.
async fn argv_never_holds_the_value(shell: &str, rc_dir: &std::path::Path, env_name: &str, rc_file: &str, rc: String) {
    let dir = rc_dir;
    let bin = dir.join("bin");
    let log = dir.join("argv.log");
    std::fs::create_dir_all(&bin).unwrap();
    for program in ["test", "grep", "printenv", "env", "sed", "echo"] {
        let real = ["/usr/bin", "/bin"].iter().map(|d| format!("{d}/{program}")).find(|p| std::path::Path::new(p).exists());
        let Some(real) = real else { continue };
        let wrapper = bin.join(program);
        std::fs::write(&wrapper, format!("#!/bin/sh\nprintf '%s %s\\n' {program} \"$*\" >> '{}'\nexec {real} \"$@\"\n", log.display())).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let config = dir.join("config");
    let rc_path = if env_name == "XDG_CONFIG_HOME" { config.join("fish").join(rc_file) } else { config.join(rc_file) };
    std::fs::create_dir_all(rc_path.parent().unwrap()).unwrap();
    std::fs::write(&rc_path, rc.replace("BIN", &bin.to_string_lossy())).unwrap();
    let found = ask_inheriting(shell, TIMEOUT, &[(env_name, &config.to_string_lossy())]).await;
    assert_eq!(found, ["ANTHROPIC_API_KEY"], "{shell}");
    let logged = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(logged.contains("grep -q ."), "{shell}: the wrappers weren't the programs that ran: {logged:?}");
    assert!(!logged.contains("SECRETVALUE"), "{shell}: the value reached a program's argv: {logged:?}");
}

#[tokio::test]
async fn the_value_is_never_in_an_argv_under_zsh() {
    if !std::path::Path::new("/bin/zsh").exists() {
        eprintln!("SKIPPED: /bin/zsh isn't installed");
        return;
    }
    let dir = scratch();
    let rc = format!("export PATH=BIN:$PATH\nexport ANTHROPIC_API_KEY={SECRET}\n");
    argv_never_holds_the_value("/bin/zsh", dir.path(), "ZDOTDIR", ".zshrc", rc).await;
}

#[tokio::test]
async fn the_value_is_never_in_an_argv_under_fish() {
    let Some(fish) = ["/opt/homebrew/bin/fish", "/usr/local/bin/fish", "/usr/bin/fish"].into_iter().find(|p| std::path::Path::new(p).exists()) else {
        eprintln!("SKIPPED: fish isn't installed");
        return;
    };
    let dir = scratch();
    let rc = format!("set -gx PATH BIN $PATH\nset -gx ANTHROPIC_API_KEY {SECRET}\n");
    argv_never_holds_the_value(fish, dir.path(), "XDG_CONFIG_HOME", "config.fish", rc).await;
}

/// A shell that doesn't finish is killed with everything it started: an rc
/// file's background job must not outlive it.
#[tokio::test]
async fn a_shell_that_hangs_is_killed_with_its_group() {
    let dir = scratch();
    let pidfile = dir.path().join("pid");
    let path = dir.path().join("hangs");
    std::fs::write(&path, format!("#!/bin/sh\nsleep 60 &\necho $! > '{}'\nwait\n", pidfile.display())).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(ask_within(path.to_str().unwrap(), Duration::from_millis(500)).await, Vec::<String>::new());
    let pid = std::fs::read_to_string(&pidfile).unwrap().trim().to_string();
    let alive = || std::process::Command::new("kill").args(["-0", &pid]).stderr(Stdio::null()).status().unwrap().success();
    for _ in 0..100 {
        if !alive() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    std::process::Command::new("kill").args(["-9", &pid]).status().unwrap();
    panic!("the shell's background job outlived the timeout");
}

/// Triggers during an ask start no second shell; one more runs after it, once.
#[tokio::test(flavor = "multi_thread")]
async fn triggers_during_an_ask_run_one_more_ask_not_one_each() {
    let (dir, svc, _turn) = runner(&["bob"]).await;
    let log = dir.path().join("asks");
    let path = dir.path().join("counting");
    std::fs::write(&path, format!("#!/bin/sh\necho ask >> '{}'\nsleep 0.4\nexport ANTHROPIC_API_KEY=v\nexec /bin/sh -c \"$2\"\n", log.display())).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let shell = path.to_string_lossy().into_owned();
    for _ in 0..5 {
        svc.claude_logins.key_exports.refresh(true, shell.clone());
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    svc.claude_logins.key_exports.settled().await;
    assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 2, "one running, one after");
    assert_eq!(svc.claude_key_exports(), ["ANTHROPIC_API_KEY"]);
}

fn rpc_factory(svc: &Arc<Service>) -> crate::rpc::RpcFactory {
    crate::rpc::RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), Peer { client_id: None, scope: Scope::HostAdmin })
}

/// Adding a member asks the login shell again, and removing the last one
/// forgets the answer, over the RPC.
#[tokio::test(flavor = "multi_thread")]
async fn member_set_asks_and_member_remove_forgets() {
    use farcooler_transport::Handler as _;
    let (dir, svc, _turn) = runner(&[]).await;
    *svc.claude_logins.key_exports.shell.lock().unwrap() = Some(stand_in_shell(dir.path(), &[&format!("ANTHROPIC_API_KEY={SECRET}")]));
    let factory = rpc_factory(&svc);
    let set = Request {
        method: "member.set".into(),
        payload: Some(pb::request::Payload::MemberSet(pb::MemberSet { handle: "bob".into(), name: "Bob".into(), ..Default::default() })),
        ..Default::default()
    };
    assert!(matches!(factory.handle(set).await.outcome, Some(pb::response::Outcome::Result(_))));
    svc.claude_logins.key_exports.settled().await;
    assert_eq!(svc.claude_key_exports(), ["ANTHROPIC_API_KEY"], "member.set asked");
    let remove = Request {
        method: "member.remove".into(),
        payload: Some(pb::request::Payload::MemberRemove(pb::MemberRemove { handle: "bob".into() })),
        ..Default::default()
    };
    assert!(matches!(factory.handle(remove).await.outcome, Some(pb::response::Outcome::Result(_))));
    assert_eq!(svc.claude_logins.key_exports.get(), Vec::<String>::new(), "member.remove forgot");
}

/// The value is on no wire and in no log: the host the owner is sent, as
/// bytes, and everything the daemon logged while it asked. One thread, because
/// the subscriber is the thread's own.
#[tokio::test]
async fn the_value_never_reaches_the_wire_or_the_log() {
    let sink = Sink::default();
    let subscriber = tracing_subscriber::fmt().with_writer(sink.clone()).with_ansi(false).with_max_level(tracing::Level::TRACE).finish();
    let (dir, svc, _turn) = runner(&["bob"]).await;
    let _guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();
    // One ask that works, and one that can't run, which logs.
    asked(&svc, &stand_in_shell(dir.path(), &[&format!("ANTHROPIC_API_KEY={SECRET}")]), true).await;
    let host = host_get(&svc, None).await;
    assert_eq!(host.rc_claude_keys, ["ANTHROPIC_API_KEY"]);
    asked(&svc, "/nonexistent/shell", true).await;
    let wire = host.encode_to_vec();
    assert!(!String::from_utf8_lossy(&wire).contains("SECRETVALUE"), "the value reached the wire");
    let logged = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    assert!(logged.contains("couldn't ask the login shell"), "it logged the failure: {logged}");
    assert!(!logged.contains("SECRETVALUE") && !logged.contains("sk-ant"), "the value reached the log: {logged}");
}

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Sink {
    type Writer = Sink;
    fn make_writer(&'a self) -> Sink {
        self.clone()
    }
}
