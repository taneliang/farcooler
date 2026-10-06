//! Claude's own registry of running sessions: which conversation each claude
//! process is, and what it is doing (ov-365).
//!
//! Claude Code writes `<config>/sessions/<pid>.json` for every interactive
//! process and rewrites it as the process moves. Read on 2.1.285-2.1.290:
//!
//! ```json
//! {"pid":14975,"sessionId":"76e8…","cwd":"/Users/…","procStart":"Wed Sep 30 00:50:34 2026",
//!  "status":"busy","tmux":"farcooler:@0.%0","messagingSocketPath":"/tmp/cc-socks/14975.sock",
//!  "kind":"interactive","entrypoint":"cli","pidDomain":"darwin",…}
//! ```
//!
//! That answers outright what `session_discovery` and `log_join` guess at from
//! a worktree's files and a pane's title. Three things about it were found by
//! reading, not from any document, and each is load-bearing:
//!
//! - `procStart` is UTC. `ps -o lstart=` prints the same instant in local time
//!   (`Sun Oct  4 11:06:13` here for `Sun Oct  4 18:06:13` in the file, at
//!   UTC-7).
//! - `status` is `busy`, `idle`, or `shell` (running a `!` command), not only
//!   the two the design names.
//! - `tmux` names a session, a window and a pane (`farcooler:@0.%0`) but not
//!   the server. Every Far Cooler daemon calls its session `farcooler`, and the
//!   owner runs two (Far Cooler and Canary), so `%0` alone is two panes. A
//!   pane is matched by its tty's device as well, which is per machine.
//!
//! **Stale files exist.** A claude that crashed leaves its file behind, and
//! its pid can be reused. An entry counts only while a process with its pid is
//! alive AND started within a second of `procStart`.
//!
//! Watched with `notify` (FSEvents on macOS) rather than polled: the cache is
//! read again only after the directory changed. Liveness is checked on every
//! lookup regardless, because a process can die without touching its file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use farcooler_core::session_log::claude_slug;
use farcooler_core::session_log::projector::Activity;

/// One running claude, as its registry file describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub pid: i32,
    pub session_id: String,
    pub cwd: Option<PathBuf>,
    pub status: Option<Activity>,
    pub tmux: Option<TmuxPlace>,
    pub messaging_socket: Option<PathBuf>,
    /// `procStart`, as seconds since the epoch.
    pub started: Option<i64>,
}

/// Where in tmux claude says it is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxPlace {
    pub session: String,
    pub window: String,
    pub pane: String,
}

/// `farcooler:@0.%0`.
fn parse_tmux(text: &str) -> Option<TmuxPlace> {
    let (session, rest) = text.rsplit_once(':')?;
    let (window, pane) = rest.split_once('.')?;
    (window.starts_with('@') && pane.starts_with('%')).then(|| TmuxPlace {
        session: session.to_string(),
        window: window.to_string(),
        pane: pane.to_string(),
    })
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Days from 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `Sun Oct  4 18:06:13 2026`, in UTC, as seconds since the epoch. Strict
/// about every field, as `foreground::parse_lstart` is: a field out of range is
/// a format this does not know, not a date to normalize.
pub fn parse_proc_start(text: &str) -> Option<i64> {
    let mut fields = text.split_whitespace();
    let _weekday = fields.next()?;
    let month_name = fields.next()?;
    let month = MONTHS.iter().position(|m| *m == month_name)? as i64 + 1;
    let day: i64 = fields.next()?.parse().ok().filter(|d| (1..=31).contains(d))?;
    let clock = fields.next()?;
    let year: i64 = fields.next()?.parse().ok().filter(|y| (1970..=9999).contains(y))?;
    if fields.next().is_some() {
        return None;
    }
    let mut hms = clock.split(':').map(|f| f.parse::<i64>().ok());
    let (h, m, s) = (hms.next()??, hms.next()??, hms.next()??);
    if hms.next().is_some() || h > 23 || m > 59 || s > 60 || h < 0 || m < 0 || s < 0 {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + h * 3600 + m * 60 + s)
}

/// One registry file, or `None` when it names no pid or no session.
pub fn parse(bytes: &[u8]) -> Option<Entry> {
    let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let text = |k: &str| v.get(k).and_then(|x| x.as_str());
    Some(Entry {
        pid: i32::try_from(v.get("pid")?.as_i64()?).ok()?,
        session_id: text("sessionId").filter(|s| !s.is_empty())?.to_string(),
        cwd: text("cwd").map(PathBuf::from),
        status: match text("status") {
            Some("busy") => Some(Activity::Busy),
            Some("idle") => Some(Activity::Idle),
            Some("shell") => Some(Activity::Shell),
            _ => None,
        },
        tmux: text("tmux").and_then(parse_tmux),
        messaging_socket: text("messagingSocketPath").map(PathBuf::from),
        started: text("procStart").and_then(parse_proc_start),
    })
}

/// What the kernel says about a process. A trait so the stale-file rules can
/// be tested against processes that do not exist.
pub trait Processes: Send + Sync {
    /// When `pid` started, in seconds since the epoch, or `None` if there is
    /// no such process.
    fn started(&self, pid: i32) -> Option<i64>;
    /// The device number of `pid`'s controlling terminal.
    fn tty(&self, pid: i32) -> Option<u64>;
}

/// The running system.
pub struct Kernel;

#[cfg(target_os = "macos")]
fn bsd_info(pid: i32) -> Option<libc::proc_bsdinfo> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: the buffer is a zeroed `proc_bsdinfo` of exactly the size passed,
    // and the kernel writes at most that many bytes into it.
    let written = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size) };
    // SAFETY: fully written when the call returns the whole size.
    (written == size).then(|| unsafe { info.assume_init() })
}

#[cfg(target_os = "linux")]
fn linux_stat(pid: i32) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name is parenthesized and may hold spaces; fields resume
    // after its closing parenthesis, at field 3 (state).
    let after = &text[text.rfind(')')? + 1..];
    Some(after.split_whitespace().map(str::to_string).collect())
}

impl Processes for Kernel {
    #[cfg(target_os = "macos")]
    fn started(&self, pid: i32) -> Option<i64> {
        bsd_info(pid).map(|i| i.pbi_start_tvsec as i64)
    }

    #[cfg(target_os = "macos")]
    fn tty(&self, pid: i32) -> Option<u64> {
        bsd_info(pid).map(|i| i.e_tdev).filter(|&d| d != u32::MAX && d != 0).map(u64::from)
    }

    #[cfg(target_os = "linux")]
    fn started(&self, pid: i32) -> Option<i64> {
        // Field 22, start time in clock ticks since boot, plus boot time.
        let ticks: i64 = linux_stat(pid)?.get(19)?.parse().ok()?;
        let boot: i64 = std::fs::read_to_string("/proc/stat")
            .ok()?
            .lines()
            .find_map(|l| l.strip_prefix("btime ")?.trim().parse().ok())?;
        // SAFETY: sysconf has no preconditions.
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        (hz > 0).then(|| boot + ticks / hz)
    }

    #[cfg(target_os = "linux")]
    fn tty(&self, pid: i32) -> Option<u64> {
        linux_stat(pid)?.get(4)?.parse().ok().filter(|&d: &u64| d != 0)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn started(&self, _: i32) -> Option<i64> {
        None
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn tty(&self, _: i32) -> Option<u64> {
        None
    }
}

/// The device number of a terminal device path (`/dev/ttys012`).
pub fn device_of(tty: &str) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(tty).ok().map(|m| m.rdev())
}

/// Whether `entry` is the process running now under its pid, and not a file a
/// crashed claude left for a pid since reused. An entry with no readable
/// `procStart` is refused: the check cannot be made, and a wrong conversation
/// is worse than none.
pub fn is_live(entry: &Entry, procs: &dyn Processes) -> bool {
    match (entry.started, procs.started(entry.pid)) {
        (Some(said), Some(actual)) => (said - actual).abs() <= 1,
        _ => false,
    }
}

/// Claude's configuration directory: `CLAUDE_CONFIG_DIR`, else `~/.claude`.
pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude")))
}

#[derive(Default)]
struct Cache {
    /// Every entry in the directory, by pid, as last read.
    entries: HashMap<i32, Entry>,
}

/// The registry directory, read when it changes.
pub struct Registry {
    config: PathBuf,
    procs: Box<dyn Processes>,
    cache: Mutex<Cache>,
    /// Set by the watcher; also set while no watch could be made, so every
    /// lookup reads the directory (a few small files) rather than going stale.
    dirty: Arc<AtomicBool>,
    watched: AtomicBool,
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
    /// When a watch was last tried and could not be made. On Linux a failed
    /// try still costs a thread (`INotifyWatcher::new` starts its loop before
    /// `watch` fails), so it is not retried on every lookup.
    watch_tried: Mutex<Option<std::time::Instant>>,
    /// Watches made or tried, for the test that the retry is rate-limited.
    watch_attempts: std::sync::atomic::AtomicU32,
    /// `false` for a test registry whose cache must move only when the test
    /// says so: no watch is made, and the cache is treated as watched.
    may_watch: bool,
    /// Session id to the transcript found for it.
    transcripts: Mutex<HashMap<String, PathBuf>>,
    /// Session id to when every project directory was last searched for it.
    misses: Mutex<HashMap<String, std::time::Instant>>,
}

/// How often a missing watch is tried again.
const WATCH_RETRY: std::time::Duration = std::time::Duration::from_secs(5);

/// How often a session with no transcript yet is searched for across every
/// project directory.
const SEARCH_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

impl Registry {
    pub fn new(config: PathBuf, procs: Box<dyn Processes>) -> Registry {
        let registry = Registry {
            config,
            procs,
            cache: Mutex::new(Cache::default()),
            dirty: Arc::new(AtomicBool::new(true)),
            watched: AtomicBool::new(false),
            watcher: Mutex::new(None),
            watch_tried: Mutex::new(None),
            watch_attempts: Default::default(),
            may_watch: true,
            transcripts: Mutex::new(HashMap::new()),
            misses: Mutex::new(HashMap::new()),
        };
        registry.watch();
        registry
    }

    /// A registry that never watches: its cache is read once and then only
    /// when something marks it dirty. For tests of what a stale cache does.
    #[cfg(test)]
    pub(crate) fn unwatched(config: PathBuf, procs: Box<dyn Processes>) -> Registry {
        let mut registry = Registry::new(PathBuf::from("/nonexistent-fc-registry"), procs);
        registry.config = config;
        registry.may_watch = false;
        registry.watched.store(false, Ordering::Relaxed);
        *registry.watcher.lock().unwrap_or_else(|e| e.into_inner()) = None;
        registry.dirty.store(true, Ordering::Relaxed);
        registry
    }

    #[cfg(test)]
    pub(crate) fn watch_attempts(&self) -> u32 {
        self.watch_attempts.load(Ordering::Relaxed)
    }

    fn sessions_dir(&self) -> PathBuf {
        self.config.join("sessions")
    }

    pub fn config(&self) -> &Path {
        &self.config
    }

    /// Start the watch, if it is not running and the directory now exists.
    fn watch(&self) {
        use notify::Watcher as _;
        if self.watched.load(Ordering::Relaxed) {
            return;
        }
        if !self.may_watch {
            self.watched.store(true, Ordering::Relaxed);
            return;
        }
        {
            let mut tried = self.watch_tried.lock().unwrap_or_else(|e| e.into_inner());
            let now = std::time::Instant::now();
            if tried.is_some_and(|at| now.duration_since(at) < WATCH_RETRY) {
                return;
            }
            *tried = Some(now);
        }
        self.watch_attempts.fetch_add(1, Ordering::Relaxed);
        let dirty = self.dirty.clone();
        let Ok(mut watcher) = notify::recommended_watcher(move |_: notify::Result<notify::Event>| {
            dirty.store(true, Ordering::Relaxed);
        }) else {
            return;
        };
        if watcher.watch(&self.sessions_dir(), notify::RecursiveMode::NonRecursive).is_ok() {
            *self.watcher.lock().unwrap_or_else(|e| e.into_inner()) = Some(watcher);
            self.watched.store(true, Ordering::Relaxed);
        }
    }

    fn refresh(&self) -> std::sync::MutexGuard<'_, Cache> {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let unwatched = !self.watched.load(Ordering::Relaxed);
        if self.dirty.swap(false, Ordering::Relaxed) || unwatched {
            // A watched directory that is gone (deleted, or its volume
            // unmounted) sends one last event and then nothing: drop the watch
            // so a directory made again is watched again, and read until then.
            if !unwatched && self.may_watch && !self.sessions_dir().is_dir() {
                *self.watcher.lock().unwrap_or_else(|e| e.into_inner()) = None;
                self.watched.store(false, Ordering::Relaxed);
            }
            self.watch();
            cache.entries = std::fs::read_dir(self.sessions_dir())
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .filter_map(|e| parse(&std::fs::read(e.path()).ok()?))
                .map(|e| (e.pid, e))
                .collect();
        }
        cache
    }

    /// The live entry for `pid`: the process in a pane, asked by its pid.
    pub fn by_pid(&self, pid: i32) -> Option<Entry> {
        let entry = self.refresh().entries.get(&pid).cloned()?;
        is_live(&entry, self.procs.as_ref()).then_some(entry)
    }

    /// The live entry for a session id, when exactly one live process has it.
    ///
    /// A miss reads the directory once more before answering: `/clear`
    /// rewrites the pid's file just before the new session's `SessionStart`
    /// hook fires, and the watch's event can trail the hook.
    pub fn by_session(&self, session: &str) -> Option<Entry> {
        if let Some(entry) = self.find_session(session) {
            return Some(entry);
        }
        self.dirty.store(true, Ordering::Relaxed);
        self.find_session(session)
    }

    fn find_session(&self, session: &str) -> Option<Entry> {
        let cache = self.refresh();
        let mut live = cache.entries.values().filter(|e| e.session_id == session).filter(|e| is_live(e, self.procs.as_ref()));
        let first = live.next()?.clone();
        live.next().is_none().then_some(first)
    }

    /// Whether `entry`'s process is attached to the terminal device at `tty`.
    pub fn runs_on(&self, entry: &Entry, tty: &str) -> bool {
        matches!((self.procs.tty(entry.pid), device_of(tty)), (Some(a), Some(b)) if a == b)
    }

    /// The transcript `entry` is writing, under the project directory claude
    /// keeps for `worktree`, or `None`. What adoption needs: a chat started in
    /// the worktree resumes only a session filed there, and one started by
    /// hand in a subdirectory is filed under the subdirectory.
    pub fn transcript_in(&self, entry: &Entry, worktree: &str) -> Option<PathBuf> {
        let resolved = std::fs::canonicalize(worktree).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| worktree.to_string());
        let path = self.config.join("projects").join(claude_slug(&resolved)).join(format!("{}.jsonl", entry.session_id));
        path.exists().then_some(path)
    }

    /// The transcript `entry` is writing, if it exists yet: under the slug of
    /// the registry's `cwd`, else of `fallback_cwd` (the pane's), else
    /// whichever project directory holds the session's file. Claude names its
    /// project directory for the directory it started in, and `cwd` in the
    /// registry is not promised to stay that. Remembered once found.
    pub fn transcript(&self, entry: &Entry, fallback_cwd: &str) -> Option<PathBuf> {
        let file = format!("{}.jsonl", entry.session_id);
        if let Some(found) = self.transcripts.lock().unwrap_or_else(|e| e.into_inner()).get(&entry.session_id) {
            return Some(found.clone()).filter(|p| p.exists());
        }
        let projects = self.config.join("projects");
        let fallback = std::fs::canonicalize(fallback_cwd).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| fallback_cwd.to_string());
        let by_cwd = entry
            .cwd
            .iter()
            .map(|c| c.to_string_lossy().into_owned())
            .chain([fallback])
            .map(|cwd| projects.join(claude_slug(&cwd)).join(&file))
            .find(|p| p.exists());
        let found = by_cwd.or_else(|| {
            // The wide search, at most every few seconds per session: before
            // its first turn a session has no file anywhere, and the watcher
            // asks every tick.
            let mut misses = self.misses.lock().unwrap_or_else(|e| e.into_inner());
            let now = std::time::Instant::now();
            if misses.get(&entry.session_id).is_some_and(|at| now.duration_since(*at) < SEARCH_EVERY) {
                return None;
            }
            misses.insert(entry.session_id.clone(), now);
            std::fs::read_dir(&projects).ok()?.flatten().map(|d| d.path().join(&file)).find(|p| p.exists())
        })?;
        self.transcripts.lock().unwrap_or_else(|e| e.into_inner()).insert(entry.session_id.clone(), found.clone());
        Some(found)
    }
}

/// The daemon's registry, over `config_dir()`. Under test it reads an empty
/// directory, so no test can bind to a session running on this machine.
pub fn global() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let config = if cfg!(test) {
            std::env::temp_dir().join(format!("fc-no-claude-registry-{}", std::process::id()))
        } else {
            config_dir().unwrap_or_else(|| PathBuf::from("/nonexistent"))
        };
        Registry::new(config, Box::new(Kernel))
    })
}

/// A registry over made-up processes, for tests anywhere in the crate.
#[cfg(test)]
pub(crate) mod fake {
    use super::*;

    /// `Sun Oct  4 18:06:13 2026` UTC, every fake process's start.
    pub const STARTED: i64 = 1_791_137_173;

    /// Pids that are alive, and the terminal device each runs on.
    pub struct Alive(pub Vec<(i32, Option<&'static str>)>);

    impl Processes for Alive {
        fn started(&self, pid: i32) -> Option<i64> {
            self.0.iter().any(|(p, _)| *p == pid).then_some(STARTED)
        }
        fn tty(&self, pid: i32) -> Option<u64> {
            self.0.iter().find(|(p, _)| *p == pid).and_then(|(_, t)| device_of((*t)?))
        }
    }

    /// Write `<config>/sessions/<pid>.json` naming `session`, started at
    /// `STARTED`, in pane `%7`.
    pub fn write(config: &Path, pid: i32, session: &str, cwd: &str) {
        std::fs::create_dir_all(config.join("sessions")).unwrap();
        let text = format!(
            r#"{{"pid":{pid},"sessionId":"{session}","cwd":"{cwd}","procStart":"Sun Oct  4 18:06:13 2026","status":"busy","tmux":"farcooler:@1.%7"}}"#
        );
        std::fs::write(config.join("sessions").join(format!("{pid}.json")), text).unwrap();
    }
}

#[cfg(test)]
#[path = "claude_registry_tests.rs"]
mod tests;
