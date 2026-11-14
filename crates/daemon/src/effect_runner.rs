//! A supervised process group for workflow fixtures and configured commands.
//! A leader exiting is not proof that its background writers have stopped.
use std::path::Path;
use std::os::fd::OwnedFd;
use std::os::unix::fs::OpenOptionsExt;
use tokio::io::AsyncReadExt;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

pub(crate) struct Group(Option<u32>);
impl Group {
    pub(crate) fn new(pid: u32) -> Self { Self(Some(pid)) }

    /// Caller retains its unreaped leader until this signal, pinning the PGID.
    /// Disarm after one attempt, even on error, before the caller may reap.
    /// An error requires independent disappearance proof, never a late retry.
    pub(crate) fn kill(&mut self) -> std::io::Result<()> {
        let Some(pid) = self.0.take() else { return Ok(()); };
        if unsafe { libc::killpg(pid as i32, libc::SIGKILL) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) { return Err(error); }
        }
        Ok(())
    }

    pub(crate) fn gone(pid: u32) -> std::io::Result<bool> {
        if unsafe { libc::killpg(pid as i32, 0) } == 0 { return Ok(false); }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) { Ok(true) } else { Err(error) }
    }
}
impl Drop for Group {
    fn drop(&mut self) {
        // SAFETY: this is the group created for our child, never a discovered PID.
        if let Some(pid) = self.0 { unsafe { libc::killpg(pid as i32, libc::SIGKILL); } }
    }
}

/// Peek only: the retained Child remains the sole reaper and pins the owned
/// group identity until its one signal, even when the leader already exited.
pub(crate) async fn exited(pid: u32) -> std::io::Result<()> {
    loop {
        if has_exited(pid)? { return Ok(()); }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
pub(crate) fn has_exited(pid: u32) -> std::io::Result<bool> {
    exit_info(pid).map(|info| unsafe { info.si_pid() } == pid as i32
        && matches!(info.si_code, libc::CLD_EXITED | libc::CLD_KILLED | libc::CLD_DUMPED))
}
/// A normal exit is still the supervisor's retained result, not a lost process.
pub(crate) fn present(pid: u32) -> bool {
    exit_info(pid).is_ok_and(|info| unsafe { info.si_pid() } == 0 || info.si_code == libc::CLD_EXITED)
}
fn exit_info(pid: u32) -> std::io::Result<libc::siginfo_t> {
    let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
    if unsafe { libc::waitid(libc::P_PID, pid, &mut info,
        libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted { return Err(error); }
    }
    Ok(info)
}

pub(crate) async fn fixture(tree: &Path, script: &str, log: &Path, limit: Duration,
    environment: &[(&str, &str)]) -> std::io::Result<bool>
{
    supervised(tree, script, log, limit, Some(environment)).await
}
async fn supervised(tree: &Path, script: &str, log: &Path, limit: Duration,
    environment: Option<&[(&str, &str)]>) -> std::io::Result<bool>
{
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(script);
    if let Some(environment) = environment {
        command.env_clear().env("PATH", "/usr/bin:/bin").envs(environment.iter().copied());
    }
    supervise(command, tree, log, limit, None, None, None).await.map(|status| status == Some(0)).map_err(|failure| failure.error)
}

/// Only a failure before spawn, or after proven group disappearance, permits a retry.
/// Ordinary I/O conversion is deliberately conservative: errors after launch stay unsettled.
#[derive(Debug)]
pub(crate) struct ProcessFailure {
    pub error: std::io::Error,
    pub retry_safe: bool,
}
impl ProcessFailure {
    fn retry_safe(error: std::io::Error) -> Self { Self { error, retry_safe: true } }
}
impl From<std::io::Error> for ProcessFailure {
    fn from(error: std::io::Error) -> Self { Self { error, retry_safe: false } }
}

/// Configured argv uses the same group ownership/quiescence boundary as gates.
/// No inherited credentials/environment; callers explicitly pin every variable.
pub(crate) async fn configured(tree: &Path, argv: &[String], environment: &std::collections::BTreeMap<String, String>,
    log: &Path, limit: Duration, cancel: tokio::sync::watch::Receiver<bool>) -> std::io::Result<Option<i32>>
{
    owned(tree, argv, environment, log, limit, cancel, None).await.map_err(|failure| failure.error)
}

/// Like `configured`, but records who the process is in `owner` as soon as it exists, so that a
/// later run that finds the process still going (the daemon died and left it) can end it. The
/// record is a pid and the process's start time; see `Owner::reap`.
pub(crate) async fn owned(tree: &Path, argv: &[String], environment: &std::collections::BTreeMap<String, String>,
    log: &Path, limit: Duration, cancel: tokio::sync::watch::Receiver<bool>, owner: Option<&Path>) -> Result<Option<i32>, ProcessFailure>
{
    owned_yielding(tree, argv, environment, log, limit, cancel, owner, None).await
}

/// Like `owned`, and a builder's group pauses while a landing gate runs (ov-1214).
pub(crate) async fn owned_yielding(tree: &Path, argv: &[String], environment: &std::collections::BTreeMap<String, String>,
    log: &Path, limit: Duration, cancel: tokio::sync::watch::Receiver<bool>, owner: Option<&Path>,
    builder: Option<&crate::yielding::Yielding>) -> Result<Option<i32>, ProcessFailure>
{
let program = argv.first().ok_or_else(|| ProcessFailure::retry_safe(std::io::Error::other("empty argv")))?;
    let mut command = Command::new(program);
    command.args(&argv[1..]).env_clear().envs(environment);
    supervise(command, tree, log, limit, Some(cancel), owner, builder).await
}

/// A process this runtime started and may have outlived: its pid, which is also its process group,
/// and when it started, so a recycled pid is never taken for it.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Owner { pub pid: u32, pub started: String }
impl Owner {
    pub(crate) fn started_at(pid: u32) -> std::io::Result<String> {
        #[cfg(test)]
        if TEST_START_UNAVAILABLE.try_with(|_| ()).is_ok() {
            return Err(std::io::Error::other("injected unavailable process start identity"));
        }
        let output = std::process::Command::new("/bin/ps")
            .args(["-o", "lstart=", "-p", &pid.to_string()]).env_clear().output()?;
        let started = String::from_utf8(output.stdout).map_err(std::io::Error::other)?.trim().to_string();
        if !output.status.success() || started.is_empty() {
            return Err(std::io::Error::other("process start identity unavailable"));
        }
        Ok(started)
    }
    pub(crate) fn write(path: &Path, pid: u32) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(&Owner { pid, started: Self::started_at(pid)? }).map_err(std::io::Error::other)?;
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, bytes)?;
        std::fs::File::open(&temporary)?.sync_all()?;
        std::fs::rename(&temporary, path)
    }
    /// End whatever the record at `path` still owns. True when nothing of it is left. A group that is
    /// alive with its leader gone is ours (a pid is never reused while its group lives); a live leader
    /// that provably started at another time is someone else's, and left alone. Missing identity
    /// stays uncertain unless the whole group is gone; a failed lookup is never a different identity.
    pub(crate) async fn reap(path: &Path) -> std::io::Result<bool> {
        let raw = match std::fs::read(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
            Err(error) => return Err(error),
        };
        let owner: Owner = serde_json::from_slice(&raw).map_err(std::io::Error::other)?;
        // macOS answers EPERM for a group holding only a zombie; that is a group on its way out, not an error.
        if matches!(Group::gone(owner.pid), Ok(true)) { return Ok(true); }
        // An old/partial receipt or failed lookup cannot distinguish our live writer
        // from a recycled PID. Neither signal it nor authorize another gate.
        if owner.started.trim().is_empty() { return Ok(false); }
        if unsafe { libc::kill(owner.pid as i32, 0) } == 0 {
            if Self::started_at(owner.pid)? != owner.started { return Ok(true); }
        } else {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) { return Err(error); }
        }
        if let Err(error) = Group::new(owner.pid).kill() { tracing::warn!(pid = owner.pid, %error, "signalling an old group failed; waiting to see it go"); }
        for _ in 0..100 {
            if matches!(Group::gone(owner.pid), Ok(true)) { return Ok(true); }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok(false)
    }
}

/// Keep only the last 2 MiB, shared by command and native terminal capture (ov-1127).
pub(crate) const LOG_LIMIT: usize = 2 * 1024 * 1024;
pub(crate) fn save_tail(log: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().write(true).create(true).mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(log)?;
    use std::os::unix::fs::MetadataExt;
    if !file.metadata()?.is_file() || file.metadata()?.nlink() != 1 {
        return Err(std::io::Error::other("nonregular or aliased process log"));
    }
    file.set_len(0)?;
    file.write_all(&bytes[bytes.len().saturating_sub(LOG_LIMIT)..])?;
    file.sync_data()
}

/// Drain a merged stdout/stderr stream continuously, with bounded memory and disk. Publish once a
/// second as well as at EOF, so an interrupted daemon leaves useful output beside its receipt.
pub(crate) async fn capture(mut reader: tokio::net::UnixStream, log: std::path::PathBuf) -> std::io::Result<()> {
    let mut tail = std::collections::VecDeque::with_capacity(LOG_LIMIT);
    let mut chunk = [0u8; 16384];
    let mut flush = tokio::time::interval(Duration::from_secs(1));
    let mut dirty = false;
    loop {
        tokio::select! {
            read = reader.read(&mut chunk) => {
                let count = read?;
                if count == 0 { return save_tail(&log, tail.make_contiguous()); }
                let discard = (tail.len() + count).saturating_sub(LOG_LIMIT);
                tail.drain(..discard);
                tail.extend(&chunk[..count]);
                dirty = true;
            }
            _ = flush.tick() => {
                if dirty { save_tail(&log, tail.make_contiguous())?; dirty = false; }
            }
        }
    }
}

async fn supervise(mut command: Command, tree: &Path, log: &Path, limit: Duration,
    mut cancel: Option<tokio::sync::watch::Receiver<bool>>, owner: Option<&Path>,
    builder: Option<&crate::yielding::Yielding>) -> Result<Option<i32>, ProcessFailure>
{
    save_tail(log, b"").map_err(ProcessFailure::retry_safe)?;
    let (reader, writer) = std::os::unix::net::UnixStream::pair().map_err(ProcessFailure::retry_safe)?;
    reader.set_nonblocking(true).map_err(ProcessFailure::retry_safe)?;
    let reader = tokio::net::UnixStream::from_std(reader).map_err(ProcessFailure::retry_safe)?;
    let stderr = writer.try_clone().map_err(ProcessFailure::retry_safe)?;
    command.current_dir(tree)
        .stdin(Stdio::null()).stdout(Stdio::from(OwnedFd::from(writer)))
        .stderr(Stdio::from(OwnedFd::from(stderr))).kill_on_drop(true);
    command.process_group(0);
    let mut child = command.spawn().map_err(ProcessFailure::retry_safe)?;
    // Command retains its configured stdio handles; release them so EOF follows the child.
    drop(command);
    let mut output = tokio::spawn(capture(reader, log.to_path_buf()));
    let pid = child.id().expect("newly spawned child has a pid");
    let presence = crate::workflow_runtime::execution::process(pid);
    let mut group = Group(Some(pid));
    if let Some(owner) = owner { Owner::write(owner, pid)?; }
    #[cfg(test)]
    if owner.is_some() && TEST_UNSETTLED.try_with(|_| ()).is_ok() {
        group.0.take();
        TEST_UNSETTLED.with(|held| held.replace(Some(child)));
        return Err(std::io::Error::other("injected unproved process settlement").into());
    }
    // Forgotten (and continued) before the group is signalled below, so a recycled pgid is never stopped.
    let paused = builder.map(|y| y.builder(crate::yielding::Target::Group { pid, started: Owner::started_at(pid)? }));
    let result = tokio::select! {
        result = tokio::time::timeout(limit, exited(pid)) => Some(result),
        _ = async {
            if let Some(cancel) = cancel.as_mut() {
                while !*cancel.borrow_and_update() {
                    if cancel.changed().await.is_err() { break; }
                }
            } else { std::future::pending::<()>().await; }
        } => None,
    };
    // The supervisor has an answer; the adapter remains present while it settles that answer.
    if !matches!(result, Some(Ok(Err(_)))) { presence.settle(); }
    // Signal before releasing the leader identity. Disarm even on signal error:
    // no retry may target a recycled PGID after the child has been reaped.
    #[cfg(test)]
    let _ = TEST_SIGNAL_PINNED.try_with(|pinned| {
        let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
        pinned.set(unsafe { libc::waitid(libc::P_PID, pid, &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) } == 0);
    });
    drop(paused);
    let signal_error = group.kill().err();
    let timed_out = matches!(result, Some(Err(_)));
    let observed = matches!(result, Some(Ok(Ok(()))));
    let status = tokio::time::timeout(Duration::from_secs(1), async {
        // A child the kernel was still creating when the group was signalled can miss the signal and
        // outlive its dying parent (ov-1027). The leader forks nothing once it has exited, and until it
        // is reaped its zombie still holds the PGID, so signalling the group again cannot hit a recycled
        // one. A repeat narrows the race, it does not close it: a few short rounds, then the quiescence
        // loop below decides whether anything survived.
        exited(pid).await?;
        if signal_error.is_none() {
            for _ in 0..5 {
                unsafe { libc::killpg(pid as i32, libc::SIGKILL); }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
        child.wait().await
    }).await.map_err(|_| std::io::Error::other("owned process leader reap unproved"))??;
    match tokio::time::timeout(Duration::from_secs(1), &mut output).await {
        Ok(result) => result.map_err(std::io::Error::other)??,
        Err(_) => { output.abort(); return Err(std::io::Error::other("process output did not close").into()); }
    }
    // A surviving group (including unreaped grandchildren) is not quiescence.
    for _ in 0..100 {
        if matches!(Group::gone(pid), Ok(true)) {
            if let Some(Ok(Err(error))) = result { return Err(ProcessFailure::retry_safe(error)); }
            return if timed_out { Err(ProcessFailure::retry_safe(std::io::Error::new(std::io::ErrorKind::TimedOut, "process deadline exceeded"))) }
                else { Ok(if observed { status.code() } else { None }) };
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if let Some(error) = signal_error {
        tracing::warn!(pid, %error, "owned process signal failed and disappearance remains unproved");
    }
    Err(std::io::Error::other("process group did not become quiescent").into())
}

#[cfg(test)]
tokio::task_local! { pub(crate) static TEST_START_UNAVAILABLE: (); }
#[cfg(test)]
tokio::task_local! { static TEST_SIGNAL_PINNED: std::cell::Cell<bool>; }
#[cfg(test)]
tokio::task_local! { pub(crate) static TEST_UNSETTLED: std::cell::RefCell<Option<tokio::process::Child>>; }

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn process_output_keeps_the_bounded_tail_of_both_streams() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("failure.output");
        let passed = supervised(dir.path(),
            "printf 'discarded prefix'; head -c 3145728 /dev/zero; printf 'stderr at the end\\n' >&2; exit 1",
            &log, Duration::from_secs(5), None).await.unwrap();
        assert!(!passed);
        let bytes = std::fs::read(&log).unwrap();
        assert_eq!(bytes.len(), LOG_LIMIT);
        assert!(bytes.starts_with(&[0; 32]));
        assert!(bytes.ends_with(b"stderr at the end\n"));
    }

    #[test]
    fn native_output_uses_the_same_cap_and_refuses_aliased_logs() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("native.output");
        let bytes = vec![b'x'; LOG_LIMIT + 100];
        save_tail(&log, &bytes).unwrap();
        assert_eq!(std::fs::read(&log).unwrap(), bytes[100..]);
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(&log, &alias).unwrap();
        assert!(save_tail(&alias, b"clobbered").is_err());
        std::fs::hard_link(&log, dir.path().join("hardlink")).unwrap();
        assert!(save_tail(&log, b"clobbered").is_err());
        assert_eq!(std::fs::metadata(&log).unwrap().len(), LOG_LIMIT as u64);
    }

    #[tokio::test]
    async fn workflow_runtime_exited_leader_keeps_group_owned_until_descendant_stops() {
        let dir = tempfile::tempdir().unwrap();
        let result = TEST_SIGNAL_PINNED.scope(std::cell::Cell::new(false), async {
            let result = supervised(dir.path(), "sleep 30 & echo $! > descendant.pid; exit 0",
                &dir.path().join("leader.log"), Duration::from_secs(2), None).await.unwrap();
            TEST_SIGNAL_PINNED.with(|pinned| assert!(pinned.get(), "leader was reaped before group signal"));
            result
        }).await;
        assert!(result);
        let descendant: i32 = std::fs::read_to_string(dir.path().join("descendant.pid"))
            .unwrap().trim().parse().unwrap();
        assert_eq!(unsafe { libc::kill(descendant, 0) }, -1);
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }
    #[tokio::test]
    async fn workflow_runtime_process_timeout_and_cancel_kill_owned_group() {
        let dir = tempfile::tempdir().unwrap();
        let script = "echo $$ > leader.pid; sleep 30 & wait";
        assert_eq!(supervised(dir.path(), script, &dir.path().join("timeout.log"), Duration::from_millis(50), None)
            .await.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        let pid: i32 = std::fs::read_to_string(dir.path().join("leader.pid")).unwrap().trim().parse().unwrap();
        assert_ne!(unsafe { libc::killpg(pid, 0) }, 0);
        let tree = dir.path().to_path_buf();
        let task = tokio::spawn(async move {
            supervised(&tree, script, &tree.join("cancel.log"), Duration::from_secs(30), None).await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let pid: i32 = std::fs::read_to_string(dir.path().join("leader.pid")).unwrap().trim().parse().unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        for _ in 0..100 {
            if unsafe { libc::killpg(pid, 0) } != 0 { return; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("owned group survived cancellation");
    }
    /// A cancel that lands while the command is forking must still end everything it started. A child
    /// the kernel is creating while the group is signalled can miss that signal and outlive its dying
    /// parent; the workflow shutdown test leaked its fixture's `sleep` this way under load (ov-1027).
    #[tokio::test]
    async fn a_cancel_during_a_fork_leaves_nothing_of_the_group_running() {
        let dir = tempfile::tempdir().unwrap();
        // Forks without pause, so each cancel very likely lands mid-fork.
        let argv = ["/bin/sh".to_string(), "-c".into(), "echo $$ > leader.pid; while :; do /bin/sleep 30 & done".into()];
        let environment: std::collections::BTreeMap<String, String> = [("PATH".to_string(), "/usr/bin:/bin".to_string())].into();
        for round in 0..20 {
            let _ = std::fs::remove_file(dir.path().join("leader.pid"));
            let (cancel, cancelled) = tokio::sync::watch::channel(false);
            let (tree, argv, environment) = (dir.path().to_path_buf(), argv.clone(), environment.clone());
            let task = tokio::spawn(async move {
                configured(&tree, &argv, &environment, &tree.join("fork.log"), Duration::from_secs(60), cancelled).await
            });
            let leader = loop {
                if let Some(pid) = std::fs::read_to_string(dir.path().join("leader.pid")).ok().and_then(|t| t.trim().parse::<i32>().ok()) { break pid; }
                tokio::time::sleep(Duration::from_millis(2)).await;
            };
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel.send_replace(true);
            let settled = task.await.unwrap();
            // Clean up a leak before failing, so one red round leaves no sleeps behind.
            let alive = unsafe { libc::killpg(leader, 0) } == 0;
            if alive { unsafe { libc::killpg(leader, libc::SIGKILL); } }
            assert!(!alive, "round {round}: a member of the cancelled group is still running");
            assert_eq!(settled.unwrap(), None, "round {round}: a cancel with the whole group gone settles as cancelled");
        }
    }
}
