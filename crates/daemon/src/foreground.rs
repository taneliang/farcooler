//! What is actually running in each terminal, arguments included.
//!
//! `pane_current_command` is a process NAME, which is the wrong grain for a label
//! people read: `pnpm dev` shows as `node`, `cargo build` as `cargo`, and every
//! idle shell as `zsh`. What distinguishes one pane from another is usually the
//! arguments — `pnpm dev` from `pnpm test`, `cargo build` from `cargo test`.
//!
//! So the foreground process group of each pane's tty is read from the process
//! table (in-process, `proc_table`; `ps` is the fallback), which has the argv. One call for the whole host per sample, not one per pane: this
//! sits on the watcher's loop, and a fleet of thirty panes must not mean thirty
//! processes a second.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

/// The process a pane is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Running {
    pub pid: i32,
    /// The process GROUP, which is the unit a pane's work actually occupies.
    ///
    /// The pid alone answers "what did the user type"; it does not answer "what
    /// is that thing doing", because a command started through a wrapper —
    /// `pnpm dev`, `npm run dev`, any shell script — does its work in a CHILD.
    /// The group is what both share, and what `ps` reports as `pgid`.
    pub pgid: i32,
    /// A command line short enough to be a label.
    pub command: String,
}

/// The ports a pane serves, as the wire carries them (`Terminal.ports`).
///
/// `by_group` is `Foreground::ports_by_group`'s answer and `running` the pane's
/// foreground process: the same join the pane's `web :PORT` purpose is read
/// through, so the structured field and the text can't disagree. A pane with
/// nothing in the foreground, or nothing listening, serves none.
pub fn pane_ports(running: Option<&Running>, by_group: &HashMap<i32, Vec<u16>>) -> Vec<u32> {
    running
        .and_then(|r| by_group.get(&r.pgid))
        .map(|open| farcooler_core::ports::field(open))
        .unwrap_or_default()
}

/// One `ps` walk, read for the two things a pane needs from it.
#[derive(Debug, Default)]
pub struct Foreground {
    /// The foreground process of each tty, keyed by tty name (`ttys162`).
    panes: HashMap<String, Running>,
    /// Every process's group, foreground or not.
    ///
    /// Kept for the whole host because the process holding a pane's socket
    /// is often not the one the pane is showing. The same walk already has
    /// both columns, so this costs a map and no extra process.
    groups: HashMap<i32, i32>,
    /// Every process's children, by parent pid.
    ///
    /// For claiming (`claims::scan`), which has to reach the commands codex
    /// and cursor run. Those have no controlling tty (`??`), so no per-tty
    /// reading finds them; only their parent does.
    children: HashMap<i32, Vec<i32>>,
    /// Every process that has a controlling tty, by tty name.
    on_tty: HashMap<String, Vec<i32>>,
    /// The command line of every foreground row, by tty name (ov-443).
    ///
    /// What `running_agent::of` reads: the pane's foreground processes, which is
    /// where a typed `claude` is, and never what the pane has drawn.
    foreground_args: HashMap<String, Vec<String>>,
}

impl Foreground {
    /// What a tty is showing, if anything.
    pub fn pane(&self, tty: &str) -> Option<&Running> {
        self.panes.get(tty)
    }

    /// The command lines of what is in a tty's foreground, wrappers included.
    pub fn foreground_args(&self, tty: &str) -> Vec<String> {
        self.foreground_args.get(tty).cloned().unwrap_or_default()
    }

    /// Listening ports by process GROUP rather than by process.
    ///
    /// `lsof` answers by pid, and the pid it names is usually not the one a pane
    /// is showing: `pnpm dev` is a wrapper whose child holds the socket, and
    /// looking the wrapper's pid up in `lsof`'s output finds nothing. Verified
    /// live — a `bash -c '… & wait'` at pid 60061 with the server at 60063
    /// resolves only through the group they share.
    ///
    /// The join is against the pid -> pgid table the same `ps` walk produced, so
    /// attributing a socket to a pane costs a hash lookup and not a second walk
    /// of the process table.
    pub fn ports_by_group(&self, ports: &HashMap<i32, Vec<u16>>) -> HashMap<i32, Vec<u16>> {
        let mut by_group: HashMap<i32, Vec<u16>> = HashMap::new();
        for (pid, open) in ports {
            // A process `lsof` saw and `ps` did not — one that started or ended
            // between the two reads. It belongs to no group we can name.
            let Some(pgid) = self.groups.get(pid) else { continue };
            let group = by_group.entry(*pgid).or_default();
            for port in open {
                if !group.contains(port) {
                    group.push(*port);
                }
            }
        }
        by_group
    }

    /// Every process on `tty`, and every descendant of those by parent pid,
    /// whether or not it has a tty of its own.
    ///
    /// Rooted at the tty rather than at a pid because a pane is known by its
    /// tty (`TaggedPane::tty`). Descended by parent rather than by tty
    /// because codex and cursor run each command as a child with no tty, and
    /// that child is the only process that goes where the command goes.
    pub fn under_tty(&self, tty: &str) -> Vec<i32> {
        let mut found: Vec<i32> = self.on_tty.get(tty).cloned().unwrap_or_default();
        let mut seen: std::collections::HashSet<i32> = found.iter().copied().collect();
        let mut next = 0;
        while next < found.len() {
            for &child in self.children.get(&found[next]).into_iter().flatten() {
                if seen.insert(child) {
                    found.push(child);
                }
            }
            next += 1;
        }
        found
    }
}

/// Read every tty's foreground command, and every process's group.
///
/// `stat` carries `+` for a process in its terminal's foreground group, which is
/// exactly "the thing you are looking at" — the shell itself is `Ss` and gets
/// skipped, so an idle pane reports nothing and keeps whatever tmux called it.
pub async fn read() -> Foreground {
    // In this process first (`proc_table`): no spawn, so a tick costs the
    // syscalls and not a `ps` over the whole host.
    if let Ok(Some(table)) = tokio::task::spawn_blocking(crate::proc_table::snapshot).await {
        return parse(&table);
    }
    // The system can't be read in-process, or the walk came back empty: ask `ps`.
    let out = tokio::process::Command::new("ps")
        .args(["-axo", "pid=,ppid=,pgid=,tty=,stat=,args="])
        .stdin(std::process::Stdio::null())
        .output()
        .await;
    let Ok(out) = out else { return Foreground::default() };
    parse(&String::from_utf8_lossy(&out.stdout))
}

/// When the process with this pid started, as the kernel recorded it.
///
/// The one fact adoption needs and the daemon does not otherwise hold: a
/// transcript written before the claude in a pane started cannot be that
/// claude's, and `session_discovery::discover_claude_session` takes exactly
/// that floor. Its sole production caller used to hand it
/// `SystemTime::UNIX_EPOCH`, which is older than every file on the disk, so
/// the guard filtered nothing and a pane routinely adopted last month's
/// conversation from a worktree it was reusing.
///
/// A targeted `ps` on one pid rather than a column added to the whole-host
/// walk above. Adoption runs only when a person switches a pane into agent
/// mode, so a process on that action is cheap, and the watcher's sampling loop
/// — which is where an extra column would have been paid for every tick — is
/// untouched. If anything else ever needs a start time, `etime=` on that walk
/// is where this should end up instead.
///
/// `None` for a pid that is gone, a `ps` that fails, and a line that does not
/// parse. Each of those is "we do not know when this started", and the caller
/// must refuse to adopt rather than substitute a floor of its own: any default
/// old enough to be safe is the epoch again, wearing a different name.
pub async fn started_at(pid: i32) -> Option<SystemTime> {
    let out = tokio::process::Command::new("ps")
        .arg("-o")
        .arg("lstart=")
        .arg("-p")
        .arg(pid.to_string())
        // `lstart` is rendered with `strftime`, so the month and weekday names
        // are whatever the caller's locale says. The daemon inherits a login
        // environment it did not choose; pinning the locale for this one call
        // is what makes the parser below a parser of a known format rather
        // than a guess at the user's.
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_lstart(&String::from_utf8_lossy(&out.stdout))
}

/// `Www Mmm dd hh:mm:ss yyyy`, in the LOCAL timezone, as `ps -o lstart=` writes
/// it — `Tue Sep  8 21:07:05 2026`, day-of-month space padded, trailing spaces
/// included.
///
/// Split from `started_at` so the format is testable without a live process,
/// and strict about every field: `mktime` happily NORMALIZES nonsense, so a
/// misread `32` for a day would come back as the first of the next month and
/// look like a perfectly good answer. A field out of range is a format this
/// code does not understand, and the honest report of that is `None`.
fn parse_lstart(stdout: &str) -> Option<SystemTime> {
    let line = stdout.lines().find(|l| !l.trim().is_empty())?;
    let mut fields = line.split_whitespace();
    // The weekday is redundant with the date and is not checked against it:
    // `mktime` computes the true one, and disagreeing with `ps` about it would
    // be this code's error, not a reason to refuse.
    let _weekday = fields.next()?;
    let month_name = fields.next()?;
    let month = MONTHS.iter().position(|m| *m == month_name)? as libc::c_int;
    let day = field(fields.next()?, 1, 31)?;
    let clock = fields.next()?;
    let year = field(fields.next()?, 1970, 9999)?;
    if fields.next().is_some() {
        return None;
    }

    let mut hms = clock.split(':');
    let hour = field(hms.next()?, 0, 23)?;
    let minute = field(hms.next()?, 0, 59)?;
    // A leap second is 60, and a process may genuinely be stamped with one.
    let second = field(hms.next()?, 0, 60)?;
    if hms.next().is_some() {
        return None;
    }

    // SAFETY: `mktime` reads and writes only the `tm` it is given, which is
    // fully initialized here and outlives the call. `tm_isdst` of -1 is the
    // documented "work out for yourself whether this local time was in
    // daylight saving", which is the only correct answer for a wall-clock
    // reading with no offset attached — the alternative, assuming one, is
    // wrong for an hour twice a year.
    let seconds = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tm.tm_sec = second;
        tm.tm_min = minute;
        tm.tm_hour = hour;
        tm.tm_mday = day;
        tm.tm_mon = month;
        tm.tm_year = year - 1900;
        tm.tm_isdst = -1;
        libc::mktime(&mut tm)
    };
    // -1 is `mktime`'s failure, and anything negative is a start time before
    // 1970 — neither is a process that is running right now.
    if seconds < 0 {
        return None;
    }
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds as u64))
}

/// One numeric field of an `lstart` line, refused rather than normalized when
/// it falls outside the range the format allows.
fn field(text: &str, low: libc::c_int, high: libc::c_int) -> Option<libc::c_int> {
    let value: libc::c_int = text.parse().ok()?;
    (low..=high).contains(&value).then_some(value)
}

/// `ps` writes these under `LC_ALL=C`, and `started_at` pins that locale so
/// this list is the whole set it can be asked about.
const MONTHS: [&str; 12] =
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Split out from `read` so the column layout is testable.
///
/// It has changed three times now, to carry the pid, then the pgid, then the
/// ppid, and a silent misparse would cost every label its arguments while
/// everything kept running.
pub(crate) fn parse(stdout: &str) -> Foreground {
    let mut found = Foreground::default();
    // Each tty's foreground rows, in the order `ps` listed them.
    let mut foreground: HashMap<&str, Vec<Row<'_>>> = HashMap::new();
    for line in stdout.lines() {
        let Some(row) = row(line) else { continue };
        // Every process, including the ones with no terminal: this is the table
        // the ports join reads, and the process holding a socket may be a
        // daemonized child that has left its tty behind.
        found.groups.insert(row.pid, row.pgid);
        found.children.entry(row.ppid).or_default().push(row.pid);
        if row.tty != "??" {
            found.on_tty.entry(row.tty.to_string()).or_default().push(row.pid);
        }
        if !row.stat.contains('+') || row.tty == "??" || row.args.is_empty() {
            continue;
        }
        found.foreground_args.entry(row.tty.to_string()).or_default().push(row.args.to_string());
        foreground.entry(row.tty).or_default().push(row);
    }
    for (tty, rows) in &foreground {
        let Some((shown, at)) = shown(rows) else { continue };
        found.panes.insert(
            tty.to_string(),
            Running { pid: shown.pid, pgid: shown.pgid, command: summarize(&shown.args[at..]) },
        );
    }
    found
}

/// The process a tty's foreground rows are showing, and the byte in its argv
/// where its label starts (see `runs`).
///
/// A TOP of the foreground: a row whose parent is not itself in the
/// foreground. A foreground pipeline's members are siblings under the shell
/// waiting for them, and a program's own children (claude's `caffeinate`, its
/// `sourcekit-lsp`) are below it and never compete with it, whatever their
/// pids are.
///
/// More than one top is a pipeline, or a process whose parent died: the kernel
/// reparents it to launchd (ppid 1), and it keeps its group and its tty, so a
/// `vite &` left behind by an exited tool shell is an `S+` row with no parent
/// in the foreground. Of several tops the group LEADER (`pid == pgid`) is the
/// answer. The leader is what the shell started the group with, so it is the
/// outer wrapper for a dispatch, the program for a typed command, and the
/// first member of a pipeline in bash, zsh and fish (`rg foo | less` reads as
/// `rg`). `ps` order is only the fallback, for a group whose leader has exited:
/// macOS wraps pids at 99998, and after a wrap the orphan sorts first.
///
/// Then, when that top is a shell wrapper, down through it to what it runs.
/// See `unwrap`.
fn shown<'r, 'a>(rows: &'r [Row<'a>]) -> Option<(&'r Row<'a>, usize)> {
    let mut tops = rows.iter().filter(|r| !rows.iter().any(|p| p.pid == r.ppid));
    let first = tops.clone().next()?;
    let top = tops.find(|r| r.pid == r.pgid).unwrap_or(first);
    Some(unwrap(top, rows, 0).unwrap_or((top, 0)))
}

/// What a shell wrapper is running, and where its label starts, or None when
/// that cannot be named yet and the wrapper stands for itself.
///
/// A shell that was handed `-c` is not what anybody typed; it is what started
/// it. Every agent the daemon launches is one: `fish -c env … fish -ilc
/// 'claude …'`, and a fish handed `-c` does no job control, so claude never
/// gets a group of its own and shares the foreground with both fishes. Read
/// live on 2026-09-26 from a real `task dispatch`: pids 74359, 74380 and 74390
/// all in group 74359, which was the tty's foreground group, and the first
/// row, the outer fish, labeled the pane `fish`.
///
/// Only the command string is followed. Of the wrapper's own children (by
/// ppid, never by row order), the answer is the one running the program the
/// string names: `fish` for the outer `env … fish -ilc …`, then `claude` for
/// the inner `-ilc 'claude …'`. A child that is itself a wrapper is recursed
/// into only when it is that program. Everything else under the wrapper is
/// ignored, wrapper or not: a `config.fish` helper (`starship init fish`), a
/// `&` job it left behind (`bash -c 'sleep 600; brew update' &`, which does
/// not exec and so is a wrapper with a child of its own). Those are the inner
/// fish's children too, and following them would name the pane after the job
/// for as long as it runs.
///
/// A command string whose program is not a child here, `sh -c 'cd x && make'`,
/// or whose program is a fish function or an alias (`claude` running `npm exec
/// …`), reads as the shell. That is the old answer, and never the wrong
/// program.
///
/// Under a fish default-shell, tmux runs the Changes and agent-mode panes as
/// `fish -c '/Applications/Far Cooler Canary.app/…/farcooler' pane-host …`.
/// `ps` prints no quoting, so the target is `Far`, the first word of the
/// shim's path, and the shim's own row matches it by the same split. The pane
/// reads as the shim, labeled `Far Cooler` by `summarize`. That is what zsh and
/// bash have always shown, because those shells exec the command, so it is
/// left as it is: the label is `summarize`'s to fix, not this walk's.
fn unwrap<'r, 'a>(
    row: &'r Row<'a>,
    rows: &'r [Row<'a>],
    depth: usize,
) -> Option<(&'r Row<'a>, usize)> {
    // The daemon's own launch nests two deep. Eight is any shape a person
    // could build, and a bound is what a ppid cycle in a torn read would need.
    if depth > 8 {
        return None;
    }
    let command = command_string(row.args)?;
    let target = target(&command)?;
    rows.iter()
        .filter(|c| c.ppid == row.pid && c.pid != row.pid)
        .filter_map(|c| runs(c.args, target).map(|at| (c, at)))
        .find_map(|(child, at)| match command_string(child.args) {
            Some(_) => unwrap(child, rows, depth + 1),
            None => Some((child, at)),
        })
}

/// The words of a shell's command string, when it was handed one.
///
/// `sh -c …`, a `-c` in a cluster (`fish -ilc`, which is the daemon's own
/// launch), and `--command`/`--command=` (fish) or `--commands` (nu). Options
/// before it that take a value have the value skipped, so `bash -o pipefail
/// -c …`, `bash -O extglob -c …`, `fish -C init -c …`, `bash +o posix -c …`
/// and `bash --rcfile f -c …` are all found. Only options count: the first
/// operand ends the search, so in `bash script.sh -c` the `-c` is the
/// script's, and a shell with no command string is a prompt somebody is at,
/// which is exactly what its pane is showing.
///
/// `ps` prints argv joined by spaces, so an option VALUE with a space in it
/// (`-C 'set x 1'`) splits into several words, the second is taken for an
/// operand, and the answer is None. That is the old reading, never a wrong one.
fn command_string<'a>(args: &'a str) -> Option<CommandString<'a>> {
    let mut parts = args.split_whitespace();
    // A login shell is started as `-fish`, with a dash for a name.
    let mut program = basename(parts.next()?).trim_start_matches('-');
    // `busybox sh -c …`: the applet is the shell.
    if program == "busybox" {
        program = basename(parts.next()?);
    }
    if !SHELLS.contains(&program) {
        return None;
    }
    let fish = program == "fish";
    let valued: &[char] = if fish { &['C', 'd', 'f', 'o'] } else { &['o', 'O'] };
    let found = |words: Vec<&'a str>| Some(CommandString { words, fish });
    while let Some(arg) = parts.next() {
        // `--` ends the options, and what follows is an operand.
        if arg == "--" {
            return None;
        }
        if let Some(long) = arg.strip_prefix("--") {
            if let Some(first) = long.strip_prefix("command=") {
                return found(std::iter::once(first).chain(parts).collect());
            }
            if long == "command" || long == "commands" {
                return found(parts.collect());
            }
            if LONG_VALUED.contains(&long) {
                parts.next();
            }
            continue;
        }
        if let Some(flags) = arg.strip_prefix('-') {
            if flags.contains('c') {
                return found(parts.collect());
            }
            if flags.ends_with(valued) {
                parts.next();
            }
            continue;
        }
        // `+x`, `+o posix`: the same options, turned off.
        if let Some(flags) = arg.strip_prefix('+') {
            if flags.ends_with(valued) {
                parts.next();
            }
            continue;
        }
        return None;
    }
    None
}

/// A shell's command string, as words, and whether fish is the shell that
/// reads it: fish's single quotes take `\'` and `\\` as escapes, and no
/// other shell's do (`open_quote`).
#[derive(Debug, PartialEq)]
struct CommandString<'a> {
    words: Vec<&'a str>,
    fish: bool,
}

impl<'a> std::ops::Deref for CommandString<'a> {
    type Target = [&'a str];

    fn deref(&self) -> &Self::Target {
        &self.words
    }
}

/// Long options that take their value as the next word.
const LONG_VALUED: &[&str] = &["rcfile", "init-file", "init-command", "features", "debug-output"];

/// The program a command string runs: its first word, past any `NAME=value`
/// and the prefixes that exec into or wait on what follows them (`env`,
/// `exec`, `command`, `nohup`, `nice`, `time`), with their flags, as a
/// basename. `ps` prints no quoting, so a quote the shell would have removed
/// is removed here, and so is a trailing `;` (`claude; exit`).
///
/// A prefix flag that takes its value as the next word has the value skipped:
/// `env -u NAME` and `env -P path` (and GNU `env -C dir`), `nice -n 10`, GNU
/// `time -f fmt` and `-o file`. Anything else not understood here names some
/// other program, and the wrapper reads as itself, which is the old answer.
///
/// An assignment whose value is quoted and holds a space arrives split
/// across words, and the words up to the closing quote are still its value.
/// An orchestrator's `FARCOOLER_CHARTER='…/Application Support/…'` is one;
/// read word by word, its second half would be taken for the program. Where
/// the quote closes is read as a shell reads it (`open_quote`), not from
/// how a word ends: `shell_quote` writes `it' b` as `'it'\'' b'`, whose
/// first word ends in a quote that opens, and `A='a b'c` has no word ending
/// in one.
fn target<'a>(command: &CommandString<'a>) -> Option<&'a str> {
    let mut words = command.words.iter();
    let mut prefix = "";
    while let Some(raw) = words.next() {
        let word = raw.trim_matches(QUOTES).trim_end_matches(';').trim_matches(QUOTES);
        if word.is_empty() {
            continue;
        }
        if let Some(flag) = word.strip_prefix('-') {
            if matches!(
                (prefix, flag),
                ("env", "u" | "P" | "C") | ("nice", "n") | ("time", "f" | "o")
            ) {
                words.next();
            }
            continue;
        }
        if let Some((name, _)) = word.split_once('=') {
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                // The untrimmed word, because trimming took its quotes.
                let mut open = open_quote(raw, None, command.fish);
                while let Some(quote) = open {
                    let Some(rest) = words.next() else { break };
                    open = open_quote(rest, Some(quote), command.fish);
                }
                continue;
            }
        }
        let program = basename(word);
        if PREFIXES.contains(&program) {
            prefix = program;
            continue;
        }
        return Some(program);
    }
    None
}

/// The quote still open at the end of `word`, which starts inside `open`.
///
/// As POSIX sh reads it: everything up to the next `'` is literal inside
/// single quotes, and elsewhere a backslash escapes the character after it.
/// fish differs in one place: inside single quotes, `\'` and `\\` are
/// escapes, so `'it\'s'` is one word (fish's own manual, "Quotes"). Skipping
/// whatever follows a backslash there is the same test, because only a `'`
/// could close the quote. What `shell_quote` writes reads the same in both,
/// since it never puts a backslash inside single quotes.
fn open_quote(word: &str, mut open: Option<char>, fish: bool) -> Option<char> {
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        match (open, c) {
            (Some('\''), '\'') | (Some('"'), '"') => open = None,
            (Some('\''), '\\') if fish => {
                chars.next();
            }
            (Some('\''), _) => {}
            (_, '\\') => {
                chars.next();
            }
            (None, '\'' | '"') => open = Some(c),
            _ => {}
        }
    }
    open
}

/// Words that run the rest of the command rather than being it.
const PREFIXES: &[&str] = &["env", "exec", "command", "nohup", "nice", "time"];

const QUOTES: &[char] = &['\'', '"'];

/// Whether a process is running `program`, and if so, the byte of its argv
/// where its label starts.
///
/// By its own name, which is the whole argv. Or, when the process is an
/// interpreter (see `INTERPRETERS`), by one of the next two words that is not
/// a flag: `bash ~/.local/bin/cursor-agent` before its `exec -a`, and an
/// npm-installed codex, which `ps` shows as `node …/node_modules/.bin/codex …`
/// (read live here from the npx-cached `@openai/codex` 0.156.1; the native
/// binary is node's child). Then the label starts at that word, so the pane
/// reads `codex`, which is how `Registry::rules_for_command` knows an agent,
/// and not `node codex`.
///
/// Only an interpreter's operands count. Any other process whose operand
/// happens to end in the name is not running it: a `config.fish` job `tail -F
/// /tmp/claude` or `rg -n claude`, or a `/usr/bin/time claude` that forked
/// (whose child is the real claude, a grandchild the wrapper never reaches, so
/// the pane reads as the wrapper).
fn runs(args: &str, program: &str) -> Option<usize> {
    let mut words = args.split_whitespace();
    let first = words.next()?;
    let reach = if interpreter(first) { 2 } else { 0 };
    let operands = words.filter(|w| !w.starts_with('-')).take(reach);
    std::iter::once(first)
        .chain(operands)
        .find(|word| basename(word).trim_start_matches('-') == program)
        .map(|word| word.as_ptr() as usize - args.as_ptr() as usize)
}

/// Whether a process's first word is a program that runs a script named by an
/// operand, by the name `ps` shows (`python3.12`, `Python`, `-bash` included).
fn interpreter(word: &str) -> bool {
    let name = basename(word).trim_start_matches('-');
    let stem = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    INTERPRETERS.iter().chain(SHELLS).any(|known| stem.eq_ignore_ascii_case(known))
}

/// Script runners beyond the shells. A shell counts because a launcher script
/// is one: cursor-agent's is `bash …/cursor-agent` until it execs node.
const INTERPRETERS: &[&str] = &["node", "nodejs", "python", "ruby", "perl", "deno", "bun"];

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The shells a pane can be launched through, by the name `ps` shows.
///
/// Any of them can be a passwd login shell, and `preset_command_with_hooks`
/// launches an agent as `<login shell> -ilc`. Whether a given one gives the
/// agent a group of its own does not matter here: one that does leaves the
/// wrapper outside the foreground, where nothing looks at it.
const SHELLS: &[&str] = &[
    "sh", "ash", "bash", "zsh", "fish", "dash", "ksh", "mksh", "oksh", "yash", "tcsh", "csh",
    "nu", "xonsh",
];

/// The columns of one `ps` row.
struct Row<'a> {
    pid: i32,
    ppid: i32,
    pgid: i32,
    tty: &'a str,
    stat: &'a str,
    args: &'a str,
}

fn row(line: &str) -> Option<Row<'_>> {
    let (pid, rest) = line.trim_start().split_once(char::is_whitespace)?;
    let (ppid, rest) = rest.trim_start().split_once(char::is_whitespace)?;
    let (pgid, rest) = rest.trim_start().split_once(char::is_whitespace)?;
    let (tty, rest) = rest.trim_start().split_once(char::is_whitespace)?;
    let (stat, args) = rest.trim_start().split_once(char::is_whitespace)?;
    Some(Row {
        pid: pid.parse().ok()?,
        ppid: ppid.parse().ok()?,
        pgid: pgid.parse().ok()?,
        tty,
        stat,
        args: args.trim(),
    })
}

/// A command line short enough to be a label.
///
/// The program plus the first argument that says something. The rule this
/// replaces kept an argument only when it was not a flag, on the reasoning that
/// a subcommand is the distinguishing part — which is true, and is exactly why
/// dropping everything after a flag was wrong. `python3 -m http.server 8099`
/// labelled as `Python`, `npm --silent run dev` as `npm`, and the informative
/// half of every modern runner invocation went in the bin.
///
/// So flags are skipped rather than treated as terminal, and a flag that takes
/// a value has its value skipped with it — `-p api` contributes `api`, not `-p`.
fn summarize(args: &str) -> String {
    let mut parts = args.split_whitespace();
    let Some(program) = parts.next() else { return String::new() };
    let program = program.rsplit('/').next().unwrap_or(program);
    // `python3`, and the framework build that calls itself `Python`, are both
    // just python to a person reading a row.
    let program = normalize(program);

    let mut chosen: Option<String> = None;
    while let Some(arg) = parts.next() {
        if arg.starts_with('-') {
            // A short flag that takes a value swallows the next token, or
            // `cargo run -p api` would read as `cargo run -p`.
            if takes_a_value(arg) {
                parts.next();
            }
            continue;
        }
        chosen = Some(arg.rsplit('/').next().unwrap_or(arg).to_string());
        break;
    }

    // `cargo run -p api` wants both words, so a runner keeps looking past its
    // subcommand for the thing being run.
    let mut label = match chosen {
        Some(arg) => format!("{program} {arg}"),
        None => return program.to_string(),
    };
    if RUNNERS.contains(&program) {
        if let Some(next) = parts.find(|a| !a.starts_with('-')) {
            let next = next.rsplit('/').next().unwrap_or(next);
            let wider = format!("{label} {next}");
            if wider.chars().count() <= 24 {
                label = wider;
            }
        }
    }

    if label.chars().count() <= 24 { label } else { program.to_string() }
}

/// Programs whose first argument is a verb, so the word after it is the noun.
const RUNNERS: &[&str] = &["cargo", "npm", "pnpm", "yarn", "bun", "deno", "go", "uv", "poetry"];

/// Whether a flag consumes the token after it.
///
/// `-m` is deliberately absent. `python -m http.server` is the exact case this
/// rewrite exists for, and treating `-m` as swallowing its operand would drop
/// the only informative word in the line — the same failure under a new rule.
/// `top -l 0` is why `-l` is present: without it the label reads `top 0`.
///
/// Short forms only. `--flag=value` carries its own value and needs none of
/// this, and a long flag taking a separate value is rare enough that guessing
/// wrong costs one word.
fn takes_a_value(flag: &str) -> bool {
    matches!(flag, "-l" | "-p" | "-c" | "-o" | "-f" | "-e" | "-u" | "-t")
}

/// What a person calls this program.
///
/// `python3` and the framework build that reports itself as `Python` are both
/// just python in a row someone is scanning.
fn normalize(program: &str) -> &str {
    let stem = program.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    if stem.eq_ignore_ascii_case("python") {
        return "python";
    }
    if stem.eq_ignore_ascii_case("node") {
        return "node";
    }
    program
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subcommand_survives_because_it_is_the_distinguishing_part() {
        assert_eq!(summarize("pnpm dev"), "pnpm dev");
        assert_eq!(summarize("cargo build --release"), "cargo build");
        assert_eq!(summarize("/opt/homebrew/bin/rg pattern"), "rg pattern");
    }

    /// The rule this replaces threw away the only informative part.
    ///
    /// `python3 -m http.server 8099` labelled as `Python`, because the first
    /// argument was a flag and everything after it was dropped. That is exactly
    /// backwards for every modern runner.
    #[test]
    fn a_flag_does_not_hide_the_thing_being_run() {
        assert_eq!(summarize("python3 -m http.server 8099"), "python http.server");
        assert_eq!(summarize("node --inspect server.js"), "node server.js");
        assert_eq!(summarize("npm --silent run dev"), "npm run dev");
        assert_eq!(summarize("cargo run -p api"), "cargo run api");
    }

    /// A flag with no operand behind it still says nothing.
    #[test]
    fn a_flag_that_leads_nowhere_leaves_the_program_alone() {
        assert_eq!(summarize("tail -f"), "tail");
        assert_eq!(summarize("top -l 0"), "top");
        assert_eq!(summarize("node"), "node");
    }

    /// A flag that takes a value swallows it, so the value is not the label.
    ///
    /// Every entry in `takes_a_value` needs an operand behind it to be tested
    /// at all: `tail -f` with nothing after it produces `tail` whether or not
    /// `-f` is in the table, so the case with no operand cannot tell the two
    /// apart. Only `tail -f log` can — it reads `tail` with the table and
    /// `tail log` without it.
    #[test]
    fn a_flag_that_takes_a_value_swallows_it() {
        assert_eq!(summarize("tail -f log"), "tail");
        for flag in ["-l", "-p", "-c", "-o", "-f", "-e", "-u", "-t"] {
            assert_eq!(summarize(&format!("tail {flag} log")), "tail", "{flag}");
        }
        // The contrast, and the reason `-m` is deliberately not in the table:
        // a flag that does NOT take a value leaves the informative word alone.
        assert_eq!(summarize("tail -m log"), "tail log");
    }

    #[test]
    fn a_long_argument_is_dropped_rather_than_truncated() {
        // Half a path is worse than none: it looks like a name and is not one.
        assert_eq!(summarize("vim src/some/deeply/nested/module.rs"), "vim module.rs");
        assert_eq!(summarize("python a_very_long_script_name_indeed_here.py"), "python");
    }

    /// The interpreter's real path is not the point.
    #[test]
    fn an_interpreter_reads_as_itself() {
        let real = "/opt/homebrew/Cellar/python@3.14/3.14.6/Frameworks/Python.framework/Versions/3.14/Resources/Python.app/Contents/MacOS/Python -m http.server 8099";
        assert_eq!(summarize(real), "python http.server");
    }

    #[test]
    fn nothing_in_means_nothing_out() {
        assert_eq!(summarize(""), "");
    }

    /// Real `ps -axo pid=,ppid=,pgid=,tty=,stat=,args=` output, right-aligned
    /// columns included.
    ///
    /// The pid and the pgid are what the ports lookup asks about, so a walk that
    /// parses but loses them is worse than one that fails.
    #[test]
    fn a_walk_yields_each_ttys_foreground_process() {
        let f = parse(PS);
        assert_eq!(
            f.pane("ttys001"),
            Some(&Running { pid: 5023, pgid: 5023, command: "claude".to_string() })
        );
        assert_eq!(
            f.pane("ttys003"),
            Some(&Running { pid: 22910, pgid: 22910, command: "python http.server".to_string() })
        );
        // The session leader is `Ss` and loses to the `S+` below it, which is
        // the process the pane is actually showing.
        assert_eq!(f.pane("ttys001").map(|r| r.pid), Some(5023));
        assert_eq!(f.pane("ttys002").map(|r| r.pid), Some(43992));
        // A tty nobody is looking at.
        assert_eq!(f.pane("ttys999"), Option::None);
    }

    /// A pane's processes, for claiming, are everything on its tty and
    /// everything under those by parent pid, including the tool shells that
    /// run on no tty at all.
    ///
    /// ttys000 is `PS` as read: claude's two `/bin/zsh -c` tool shells are
    /// `??` rows, children of claude (12618), and are the processes that go
    /// where a tool call goes. Its neighbors under the same tmux server
    /// (11924, the parent of every pane's first shell) are not its.
    ///
    /// `TOOLS` adds what the spike found for codex and cursor (spec, "Spike
    /// findings (2026-09-27)"): each command is a child of the agent with no
    /// tty, and a command's own children are found through it. Pids and
    /// arguments are made up; the shape is the finding.
    #[test]
    fn a_panes_processes_include_the_children_that_left_its_tty() {
        const TOOLS: &str = "\
  900   800   900 ttys020  Ss   /opt/homebrew/bin/fish -il
  901   900   901 ttys020  S+   codex
  902   901   902 ??       Ss   /bin/zsh -lc cargo test
  903   902   902 ??       S    cargo test
  910   800   910 ttys021  Ss   /opt/homebrew/bin/fish -il
  911   910   911 ttys021  S+   cursor-agent
  912   911   912 ??       Ss   zsh -c ls
";
        let f = parse(PS);
        let mut under = f.under_tty("ttys000");
        under.sort();
        assert_eq!(under, vec![8017, 11925, 11950, 12618, 27895, 31741]);
        assert!(f.under_tty("ttys999").is_empty(), "a tty with nothing on it");

        let f = parse(&format!("{PS}{TOOLS}"));
        let mut under = f.under_tty("ttys020");
        under.sort();
        assert_eq!(under, vec![900, 901, 902, 903], "codex's command, and what that runs");
        let mut under = f.under_tty("ttys021");
        under.sort();
        assert_eq!(under, vec![910, 911, 912], "cursor's shell, and nothing of codex's");
    }

    /// A wrapper holds no socket; its child does, and shares its group.
    ///
    /// This is the case the pid lookup missed, and it is most real dev servers:
    /// `pnpm dev`, `npm run dev`, anything behind a shell script. `lsof` names
    /// the child, and only the group joins it to the pane.
    #[test]
    fn a_socket_held_by_a_child_still_belongs_to_the_pane() {
        let f = parse(PS);
        let ports = HashMap::from([(60063, vec![18299]), (22910, vec![8099])]);
        let by_group = f.ports_by_group(&ports);

        // A shell wrapper is read past to what it runs (see
        // `a_shell_handed_a_command_is_read_past_to_what_it_runs`), and the
        // group the pane reports is still the wrapper's, which is the group
        // the socket's process is in.
        let pane = f.pane("ttys009").expect("a wrapped server is a foreground process");
        assert_eq!((pane.pid, pane.pgid), (60063, 60061));
        assert_eq!(by_group.get(&pane.pgid), Some(&vec![18299]), "its group holds the socket");

        // A server started directly still resolves, through a group of one.
        let direct = f.pane("ttys003").expect("a server is a foreground process");
        assert_eq!(by_group.get(&direct.pgid), Some(&vec![8099]));
    }

    /// **The structured field, from fixtures.** `lsof` output and a `ps` walk
    /// go in, and a pane's `Terminal.ports` comes out: the wrapped server's
    /// socket is held by a child (pid 60063 here, in the pane's group), a debugger
    /// port rides along, and a pane whose process holds nothing serves nothing.
    /// Reading the pid instead of the group makes the first assertion fail.
    #[test]
    fn a_panes_ports_reach_the_wire_field_through_its_group() {
        let f = parse(PS);
        let lsof = "p60063\nn*:18299\nn127.0.0.1:9229\np22910\nn[::1]:8099\n";
        let by_group = f.ports_by_group(&farcooler_core::ports::parse_lsof(lsof));

        assert_eq!(pane_ports(f.pane("ttys009"), &by_group), vec![9229, 18299], "lowest first");
        assert_eq!(pane_ports(f.pane("ttys003"), &by_group), vec![8099]);
        assert!(pane_ports(None, &by_group).is_empty(), "no foreground process, no ports");
        assert!(pane_ports(f.pane("ttys009"), &HashMap::new()).is_empty(), "nothing listening");
    }

    /// A pane `task dispatch` opened reads as its agent, not as the fish that
    /// started it, and not as anything the agent started.
    ///
    /// claude shares group 74359 with both fishes, because a fish handed `-c`
    /// does no job control, and group 74359 is the tty's foreground group.
    /// Taking the first `+` row labeled this pane `fish` and dated its
    /// conversation by the wrapper's pid. Its own children are in the same
    /// group (live: `caffeinate` on ttys000, `sourcekit-lsp` on ttys005), and
    /// none of them may take the pane from it. Its tool shells are not in the
    /// foreground at all (the `??` rows in `PS`, which are context, not
    /// coverage: `parse` drops them before any of this).
    #[test]
    fn a_shell_handed_a_command_is_read_past_to_what_it_runs() {
        let f = parse(PS);
        // The captured typed claudes, children and tool shells included.
        assert_eq!(
            f.pane("ttys000"),
            Some(&Running { pid: 12618, pgid: 12618, command: "claude".to_string() })
        );
        assert_eq!(f_pane(PS, "ttys005"), Some(49547));
        assert_eq!(
            f.pane("ttys011"),
            Some(&Running { pid: 74390, pgid: 74359, command: "claude".to_string() })
        );
        // The label keeps codex's first word of prompt (`-c` swallows the
        // config), and the program is what `Registry::rules_for_command`
        // identifies an agent by.
        assert_eq!(
            f.pane("ttys012"),
            Some(&Running { pid: 82895, pgid: 82706, command: "codex You're".to_string() })
        );
    }

    /// The wrapper's own descendants decide, not the order `ps` lists them in.
    ///
    /// macOS wraps pids at 99998. After a wrap, the outer fish can have the
    /// highest pid on the tty and claude's child the lowest, so the first row
    /// is `caffeinate` and the last is the outer fish.
    #[test]
    fn a_pid_that_wrapped_round_changes_nothing() {
        let wrapped = "\
    5    60    40 ttys030  S+   caffeinate -i -t 300
   50 99990    40 ttys030  S+   /opt/homebrew/bin/fish -ilc claude --session-id x
   60    50    40 ttys030  S+   claude --session-id x
99990 99989    40 ttys030  S+   fish -c env FARCOOLER_ACTOR=agent:x /opt/homebrew/bin/fish -ilc 'claude --session-id x'
";
        assert_eq!(f_pane(wrapped, "ttys030"), Some(60));
    }

    /// What `config.fish` starts is not what the pane is for.
    ///
    /// `fish -ilc` sources `config.fish` interactively, with no job control, so
    /// a helper it runs and a `&` job it leaves behind are the inner fish's
    /// children in the foreground group, beside the agent and before it.
    #[test]
    fn a_config_helper_or_background_job_never_names_the_pane() {
        // Before claude starts: a helper running, a job left behind. The pane
        // still reads as the wrapper, as it did before any of this.
        let starting = "\
  300   299   300 ttys031  S+   fish -c env FARCOOLER_ACTOR=agent:x /opt/homebrew/bin/fish -ilc 'claude --session-id x'
  301   300   300 ttys031  S+   /opt/homebrew/bin/fish -ilc claude --session-id x
  302   301   300 ttys031  S+   sleep 999
  303   301   300 ttys031  S+   starship init fish --print-full-init
";
        assert_eq!(f_pane(starting, "ttys031"), Some(300));
        // Once claude is up, it is the answer, however many siblings it has.
        let running = format!("{starting}  304   301   300 ttys031  S+   claude --session-id x\n");
        assert_eq!(f_pane(&running, "ttys031"), Some(304));
        // A `&` job that is a wrapper in its own right: several commands, so
        // bash does not exec, and its `sleep` is its child. It is not the
        // program the command string names, so nothing under it counts either,
        // before claude starts or after, whatever order `ps` lists them in.
        let job = "\
  300   299   300 ttys032  S+   fish -c env FARCOOLER_ACTOR=agent:x /opt/homebrew/bin/fish -ilc 'claude --session-id x'
  301   300   300 ttys032  S+   /opt/homebrew/bin/fish -ilc claude --session-id x
  302   301   300 ttys032  S+   bash -c sleep 600; brew update
  303   302   300 ttys032  S+   sleep 600
";
        assert_eq!(f_pane(job, "ttys032"), Some(300));
        let running = format!("{job}  304   301   300 ttys032  S+   claude --session-id x\n");
        assert_eq!(f_pane(&running, "ttys032"), Some(304));
    }

    /// A process whose parent died is a second top, and never the pane's.
    ///
    /// The kernel reparents it to launchd (ppid 1), and it keeps the group and
    /// the tty: a `vite &` a tool shell left behind is an `S+` row with no
    /// parent in the foreground. After a pid wrap it sorts before the outer
    /// fish, and the group leader is what decides, not `ps` order.
    #[test]
    fn an_orphan_in_the_group_never_takes_the_pane_from_its_leader() {
        let orphan = "\
    7     1 99990 ttys033  S+   node /x/node_modules/.bin/vite
   50 99990 99990 ttys033  S+   /opt/homebrew/bin/fish -ilc claude --session-id x
   60    50 99990 ttys033  S+   claude --session-id x
99990 99989 99990 ttys033  S+   fish -c env FARCOOLER_ACTOR=agent:x /opt/homebrew/bin/fish -ilc 'claude --session-id x'
";
        assert_eq!(f_pane(orphan, "ttys033"), Some(60));
        // A pipeline after a wrap: `rg | less` leads with rg, whose pid is the
        // group's, and less got a pid that wrapped round below it.
        let pipeline = "\
    5 99000 99999 ttys034  S+   less
99999 99000 99999 ttys034  S+   rg pattern
";
        assert_eq!(f_pane(pipeline, "ttys034"), Some(99999));
        // With the leader gone (`cat f | less` after cat exits) the group
        // still has a pane: the first top.
        let leaderless = "\
    5     4     3 ttys040  S+   less
";
        assert_eq!(f_pane(leaderless, "ttys040"), Some(5));
        // Two tops and no leader: `ps` order decides, which after a wrap is the
        // later member (`a | b | c`, a exited, c's pid wrapped below b's). That
        // is main's answer, and the documented fallback.
        let two = "\
    2 99000 99990 ttys041  S+   c
99995 99000 99990 ttys041  S+   b
";
        assert_eq!(f_pane(two, "ttys041"), Some(2));
    }

    /// A program an interpreter runs is named by the program, not the
    /// interpreter.
    ///
    /// The node rows are an npm-installed codex as `ps` showed it on this host
    /// (the npx-cached `@openai/codex` 0.156.1, run from its `.bin` link with a
    /// scratch `CODEX_HOME`): node, running the link, with the native binary
    /// as its child. The pids, node's ppid-to-child link and the program paths
    /// are as read. Supplied: node's ppid and both pgids (put under the
    /// ttys012 dispatch's inner fish; they were 40079 and 40073 there), the
    /// stat (`SN+`; it was `SN`, run off a terminal), and codex's arguments
    /// (`-c check_for_update_on_startup=false` for the `app-server` it ran).
    #[test]
    fn a_program_an_interpreter_runs_is_named_by_it() {
        let npm = "\
82706 82704 82706 ttys035  SNs+ fish -c env FARCOOLER_ACTOR=agent:x FARCOOLER_TASK=pn-4 /opt/homebrew/bin/fish -ilc 'codex -c check_for_update_on_startup=false'
82744 82706 82706 ttys035  SN+  /opt/homebrew/bin/fish -ilc codex -c check_for_update_on_startup=false
40082 82744 82706 ttys035  SN+  node /Users/e-liang/.npm/_npx/4877722a062902ce/node_modules/.bin/codex -c check_for_update_on_startup=false
40093 40082 82706 ttys035  SN+  /Users/e-liang/.npm/_npx/4877722a062902ce/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex -c check_for_update_on_startup=false
";
        assert_eq!(
            parse(npm).pane("ttys035"),
            Some(&Running { pid: 40082, pgid: 82706, command: "codex".to_string() })
        );
        // Flags between the interpreter and the program are not words of it.
        let flagged = "node --no-warnings --enable-source-maps /x/bin/agent";
        assert_eq!(runs(flagged, "agent"), flagged.find("/x/bin/agent"));
        // cursor-agent's launcher before its `exec -a`, on word 2.
        assert_eq!(runs("bash /Users/e-liang/.local/bin/cursor-agent --trust", "cursor-agent"), Some(5));
        // A login shell's dash is not part of its name.
        assert_eq!(runs("-fish", "fish"), Some(0));
        assert_eq!(runs("bash -c sleep 600; brew update", "claude"), Option::None);
        assert_eq!(runs("python3.12 /x/bin/claude", "claude"), Some(11));
    }

    /// Only an interpreter's operand is the program it runs.
    ///
    /// Any other process with the name as an operand is doing something else
    /// with it, and a `config.fish` job that is one sorts before the agent.
    #[test]
    fn an_operand_names_a_program_only_after_an_interpreter() {
        for args in ["tail -F /tmp/claude", "rg -n --color=never claude", "/usr/bin/time claude"] {
            assert_eq!(runs(args, "claude"), Option::None, "{args}");
        }
        // The job before claude starts leaves the wrapper standing, and after
        // it starts loses to it.
        let tail = "\
  700   699   700 ttys038  S+   fish -c env FARCOOLER_ACTOR=agent:x /opt/homebrew/bin/fish -ilc 'claude --session-id x'
  701   700   700 ttys038  S+   /opt/homebrew/bin/fish -ilc claude --session-id x
  702   701   700 ttys038  S+   tail -F /tmp/claude
";
        assert_eq!(f_pane(tail, "ttys038"), Some(700));
        let running = format!("{tail}  703   701   700 ttys038  S+   claude --session-id x\n");
        assert_eq!(f_pane(&running, "ttys038"), Some(703));
        // `time` is a prefix, so the target is claude; a `/usr/bin/time` that
        // forks does not name the pane with its own pid.
        let time = "\
  800   799   800 ttys039  S+   sh -c time claude --continue
  801   800   800 ttys039  S+   /usr/bin/time claude --continue
  802   801   800 ttys039  S+   claude --continue
";
        assert_eq!(f_pane(time, "ttys039"), Some(800));
    }

    /// Only a shell running a command string is a wrapper.
    ///
    /// A shell at its prompt is what its pane shows, and a pane with nothing
    /// but shells in its foreground is still showing the outermost of them.
    #[test]
    fn a_shell_that_wraps_nothing_is_still_the_answer() {
        // A prompt: the `S+` login shell of ttys002, unchanged.
        assert_eq!(f_pane(PS, "ttys002"), Some(43992));
        // Two wrappers and nothing under them yet: the first stands.
        let only_shells = "\
  100    99   100 ttys020  S+   fish -c env A=1 /opt/homebrew/bin/fish -ilc claude
  101   100   100 ttys020  S+   /opt/homebrew/bin/fish -ilc claude
";
        assert_eq!(f_pane(only_shells, "ttys020"), Some(100));
        // A pipeline's members are siblings under a shell that is not in the
        // foreground, and the first is the one that was typed: `rg | less` is
        // `rg`, and a `sh -c` in it does not change that.
        let pipeline = "\
  200   199   200 ttys021  S+   rg pattern
  201   199   200 ttys021  S+   sh -c less
  202   201   200 ttys021  S+   less
";
        assert_eq!(f_pane(pipeline, "ttys021"), Some(200));
        // A command string whose program is not running under it reads as the
        // shell, never as some other child.
        let other = "\
  400   399   400 ttys022  S+   sh -c cd /tmp && make
  401   400   400 ttys022  S+   make
";
        assert_eq!(f_pane(other, "ttys022"), Some(400));
    }

    #[test]
    fn a_command_string_is_told_apart_from_a_shell_at_a_prompt() {
        for (wrapper, first) in [
            ("fish -c env FARCOOLER_ACTOR=agent:x fish -ilc claude", "env"),
            ("/opt/homebrew/bin/fish -ilc claude --session-id x", "claude"),
            ("/bin/sh -c exec sleep 600", "exec"),
            ("bash --norc -c python3 -m http.server", "python3"),
            ("-zsh -lc make", "make"),
            // Options that take a value, before the `-c`.
            ("bash -o pipefail -c make", "make"),
            ("zsh -o nocorrect -c make", "make"),
            ("bash -O extglob -c make", "make"),
            ("fish -C init -c claude", "claude"),
            ("bash +x -c make", "make"),
            ("bash +o posix -c make", "make"),
            ("bash --rcfile f -c make", "make"),
            ("fish --init-command init -c claude", "claude"),
            ("fish --command claude", "claude"),
            ("fish --command=claude --continue", "claude"),
            ("nu --commands claude", "claude"),
            ("busybox sh -c make", "make"),
            ("/bin/ash -c make", "make"),
        ] {
            let words = command_string(wrapper);
            assert_eq!(words.as_deref().and_then(|w| w.first().copied()), Some(first), "{wrapper}");
        }
        for not_one in [
            "/opt/homebrew/bin/fish -il",
            "-fish",
            "bash script.sh -c",
            "bash -- -c",
            "claude -c",
            "/Users/e-liang/.local/bin/claude --continue",
            "busybox ls -c",
            // A value with a space in it splits, and falls back to "not one".
            "fish -C 'set x 1' -c claude",
            "",
        ] {
            assert_eq!(command_string(not_one), Option::None, "{not_one}");
        }
    }

    /// The program a command string runs, past `env` and its assignments.
    #[test]
    fn a_command_strings_program_is_past_env_and_quotes() {
        fn target_of(args: &str) -> Option<&str> {
            command_string(args).and_then(|w| target(&w))
        }
        assert_eq!(
            target_of("fish -c env FARCOOLER_ACTOR=agent:x FARCOOLER_TASK=pn-1 /opt/homebrew/bin/fish -ilc 'claude'"),
            Some("fish")
        );
        assert_eq!(target_of("fish -ilc 'claude --session-id x'"), Some("claude"));
        assert_eq!(target_of("sh -c exec /bin/sleep 600"), Some("sleep"));
        // An orchestrator's charter is a quoted path, and on a Mac it has a
        // space in it. `ps` prints it split, and the half after the space is
        // still the value, not the program.
        assert_eq!(
            target_of(
                "fish -c env FARCOOLER_ACTOR=agent:x FARCOOLER_WORKSPACE=w \
                 FARCOOLER_CHARTER='/Users/x/Library/Application Support/FarCooler/workspaces/w/charter.md' \
                 /opt/homebrew/bin/fish -ilc 'claude'"
            ),
            Some("fish")
        );
        assert_eq!(target_of("sh -c env A='one two three' claude"), Some("claude"));
        assert_eq!(target_of("sh -c env A='one' claude"), Some("claude"));
        assert_eq!(target_of("sh -c env A=\"one two\" claude"), Some("claude"));
        // Read as a shell would, one quote at a time, and every case is
        // checked before any fails, so a regression names all it broke.
        let misread: Vec<_> = [
            // `shell_quote` writes a `'` as `'\''`, which leaves the first
            // half of `it' b` ending in a quote that opens, not closes.
            (
                "fish -c env FARCOOLER_WORKSPACE=w \
                 FARCOOLER_CHARTER='/Users/x/Library/Application Support/it'\\'' b/charter.md' \
                 /opt/homebrew/bin/fish -ilc 'claude'",
                Some("fish"),
            ),
            // A value that goes on past its closing quote, so no word ends
            // in one.
            ("sh -c env A='one two'three claude", Some("claude")),
            // A double quote a backslash escapes doesn't close the value.
            ("sh -c env A=\"one \\\" two\" claude", Some("claude")),
            // Never closed, as `ps` cutting a long command short leaves it.
            ("sh -c env A='one two", Option::None),
            // fish takes `\'` inside single quotes as a quote, so this value
            // is `it's here`. sh ends the quote at `\'`, so its value is
            // `it\s` and the next word is where its program starts (sh then
            // refuses the line for the quote `here'` leaves open).
            ("fish -c env A='it\\'s here' claude", Some("claude")),
            ("sh -c env A='it\\'s here' claude", Some("here")),
            // And `\\` is fish's backslash, so the `'` after it closes.
            ("fish -c env A='one \\\\' claude", Some("claude")),
        ]
        .into_iter()
        .filter(|(args, program)| target_of(args) != *program)
        .map(|(args, _)| (args, target_of(args)))
        .collect();
        assert!(misread.is_empty(), "{misread:#?}");
        assert_eq!(target_of("sh -c"), Option::None);
    }

    /// Past the prefixes that exec into, or wait on, what follows them.
    #[test]
    fn a_command_strings_program_is_past_the_prefixes_that_run_it() {
        fn target_of(args: &str) -> Option<&str> {
            command_string(args).and_then(|w| target(&w))
        }
        for (wrapper, program) in [
            ("sh -c env -u CLAUDECODE claude", "claude"),
            ("sh -c /usr/bin/env -u A -P /bin B=1 claude", "claude"),
            ("sh -c env -C /tmp claude", "claude"),
            ("sh -c nohup claude", "claude"),
            ("sh -c nice claude", "claude"),
            ("sh -c nice -n 10 claude", "claude"),
            ("sh -c nice -10 claude", "claude"),
            ("sh -c time claude", "claude"),
            ("sh -c time -p claude", "claude"),
            ("sh -c /usr/bin/time -o log claude", "claude"),
            ("fish -ilc claude; exit", "claude"),
            ("fish -ilc 'claude;' exit", "claude"),
            ("sh -c exec nohup nice -n 5 env A=1 claude", "claude"),
        ] {
            assert_eq!(target_of(wrapper), Some(program), "{wrapper}");
        }
        // A flag's value is skipped only for the prefix that takes one: `-n`
        // is `nice`'s, and after `env` it is a flag like any other.
        assert_eq!(target_of("sh -c env -n claude"), Some("claude"));

        // The prefix reaches the pane: nohup execs, so its row is claude's.
        let nohup = "\
  500   499   500 ttys036  S+   sh -c nohup claude --continue
  501   500   500 ttys036  S+   claude --continue
";
        assert_eq!(f_pane(nohup, "ttys036"), Some(501));
        // Anything not understood here falls back to the wrapper: a fish
        // function or alias named `claude` that runs `npm exec`.
        let alias = "\
  600   599   600 ttys037  S+   /opt/homebrew/bin/fish -ilc claude
  601   600   600 ttys037  S+   npm exec @anthropic-ai/claude-code
";
        assert_eq!(f_pane(alias, "ttys037"), Some(600));
    }

    fn f_pane(ps: &str, tty: &str) -> Option<i32> {
        parse(ps).pane(tty).map(|r| r.pid)
    }

    /// A socket whose process `ps` never saw belongs to no group we can name.
    #[test]
    fn a_process_that_started_between_the_two_reads_is_dropped() {
        let f = parse(PS);
        let by_group = f.ports_by_group(&HashMap::from([(99999, vec![7000])]));
        assert!(by_group.is_empty(), "{by_group:?}");
    }

    /// Verbatim `ps -o lstart= -p <pid>`, trailing spaces and all.
    ///
    /// Read off this machine: `ps -o lstart= -p $$` answers
    /// `Tue Sep  8 21:07:05 2026    `. The day of the month is space padded,
    /// so a parser that splits on a fixed width sees a different field than
    /// one that splits on whitespace, and only the second is right.
    #[test]
    fn a_start_time_is_a_wall_clock_reading_with_a_padded_day() {
        let parsed = parse_lstart("Tue Sep  8 21:07:05 2026    \n")
            .expect("the format `ps` actually writes");
        // The value is local-time dependent, so what is asserted is that it
        // landed in the right YEAR rather than an exact instant: a timezone
        // mistake is hours, and the epoch fallback this whole change exists to
        // remove is decades.
        let secs = parsed.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
        assert!((1_767_000_000..1_800_000_000).contains(&secs), "{secs}");
        // Single-digit days arrive with one space, double-digit with none.
        assert!(parse_lstart("Wed Dec 31 23:59:59 2025").is_some());
    }

    /// Anything that is not that format is "we do not know", never a default.
    ///
    /// The bug behind this function was a caller substituting a floor it had
    /// made up. A parser that quietly rounds a bad field into a good one hands
    /// the caller the same lie one layer down, so every field is refused
    /// rather than normalized — `mktime` would have turned month 12 into
    /// January of the next year without a word.
    #[test]
    fn a_line_that_is_not_that_format_is_refused_rather_than_guessed() {
        // What a pid that is gone leaves behind: `ps` exits non-zero and says
        // nothing.
        assert_eq!(parse_lstart(""), Option::None);
        assert_eq!(parse_lstart("   \n\n"), Option::None);
        // A month name from a locale this call did not pin.
        assert_eq!(parse_lstart("mar. sept.  8 21:07:05 2026"), Option::None);
        // Fields out of range, each of which `mktime` would have normalized
        // into a plausible-looking wrong answer.
        assert_eq!(parse_lstart("Tue Sep 32 21:07:05 2026"), Option::None);
        assert_eq!(parse_lstart("Tue Sep  8 24:07:05 2026"), Option::None);
        assert_eq!(parse_lstart("Tue Sep  8 21:60:05 2026"), Option::None);
        assert_eq!(parse_lstart("Tue Sep  8 21:07:61 2026"), Option::None);
        assert_eq!(parse_lstart("Tue Sep  8 21:07:05 1969"), Option::None);
        // Too few fields, too many fields, and a clock that is not a clock.
        assert_eq!(parse_lstart("Tue Sep  8 21:07:05"), Option::None);
        assert_eq!(parse_lstart("Tue Sep  8 21:07:05 2026 UTC"), Option::None);
        assert_eq!(parse_lstart("Tue Sep  8 21:07 2026"), Option::None);
        assert_eq!(parse_lstart("Tue Sep  8 21:07:05:00 2026"), Option::None);
    }

    /// The whole chain against a process whose start time is known: this one.
    ///
    /// The unit test above cannot catch a timezone mistake, because it has no
    /// second opinion about what the string means. This does: the test binary
    /// started moments ago, so `started_at` must answer with a time in the
    /// recent past. A `mktime` fed a UTC reading as local — or a `tm_isdst` of
    /// 0 instead of -1 — is off by whole hours and fails here.
    #[tokio::test]
    async fn a_live_process_started_a_moment_ago() {
        let mine = std::process::id() as i32;
        let started = started_at(mine).await.expect("this process is running");
        let now = SystemTime::now();
        let age = now.duration_since(started).expect("a running process started in the past");
        assert!(age < Duration::from_secs(3600), "{age:?} old, so the timezone is wrong");

        // A pid nothing can be running under. `ps` refuses it, and a refusal
        // must read as "unknown" rather than as a time.
        assert_eq!(started_at(i32::MAX).await, Option::None);
    }

    /// `ps -axo pid=,ppid=,pgid=,tty=,stat=,args=` as read on this host, with a
    /// wrapper pairing added: the two rows 60061/60063 were observed live.
    ///
    /// ttys000, ttys002 and ttys005 are verbatim, read on 2026-09-27 from the
    /// owner's own panes with `ps -axo pid,ppid,pgid,tpgid,tty,stat,comm,args`
    /// (tpgid and comm dropped, the columns the walk reads kept): a claude
    /// someone typed, `claude -r`, with its `caffeinate` in its group; a fish
    /// at its prompt; and a claude with its `sourcekit-lsp`. The `/bin/zsh -c`
    /// rows are claude's tool shells, arguments cut after the snapshot path.
    /// They are NOT in claude's group: each is `Ss` on no tty, a session of its
    /// own, so a tool call never enters the foreground at all. They are kept as
    /// the record of that, not as coverage: `parse` skips a row on `??` or
    /// without `+`, so no assertion can depend on them.
    ///
    /// ttys011 is the round-1 `task dispatch` of claude, read at the trust
    /// dialog: pid, pgid, stat and argv as read, arguments cut short. That walk
    /// had no ppid column, so the ppids are the only shape the pgids allow
    /// (74380 under 74359, 74390 under 74380) and 74358 is a placeholder.
    ///
    /// ttys012 is the be0c6a56 live check's codex dispatch (on ttys001 there,
    /// renamed so both fit in one walk): pid, ppid, pgid and stat as read. The
    /// argv was cut in that capture where the `…` stands; the outer row's
    /// middle, after `FARCOOL`, is the daemon's launch (`service.rs`,
    /// `preset_command_with_hooks`), since the walk needs the `-ilc` in it.
    const PS: &str = "\
11925 11924 11925 ttys000  Ss   fish -c /opt/homebrew/bin/fish -il
11950 11925 11950 ttys000  S    /opt/homebrew/bin/fish -il
12618 11950 12618 ttys000  S+   claude -r
27895 12618 12618 ttys000  S+   caffeinate -i -t 300
 8017 12618  8017 ??       Ss   /bin/zsh -c source /Users/e-liang/.claude/shell-snapshots/snapshot-zsh-1790218583336-m8r8bj.sh
31741 12618 31741 ??       Ss   /bin/zsh -c source /Users/e-liang/.claude/shell-snapshots/snapshot-zsh-1790443015834-hu8xa3.sh
 1758  1000  1758 ttys001  Ss   /opt/homebrew/bin/fish -l
 5023  1758  5023 ttys001  S+   /Users/e-liang/.local/bin/claude
43971 11924 43971 ttys002  Ss   fish -c /opt/homebrew/bin/fish -il
43992 43971 43992 ttys002  S+   /opt/homebrew/bin/fish -il
22910 22900 22910 ttys003  S+   /usr/bin/python3 -m http.server 8099
49363 11924 49363 ttys005  Ss   fish -c /opt/homebrew/bin/fish -il
49381 49363 49381 ttys005  S    /opt/homebrew/bin/fish -il
49547 49381 49547 ttys005  S+   claude
63629 49547 49547 ttys005  S+   /Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/sourcekit-lsp
60061 60000 60061 ttys009  S+   bash -c python3 -m http.server 18299 & wait
60063 60061 60061 ttys009  S+   /usr/bin/python3 -m http.server 18299
74359 74358 74359 ttys011  SNs+ fish -c env FARCOOLER_ACTOR=agent:01a0dad0-afd0-7bd1-9d62-3d125ede9ae2 FARCOOLER_TASK=pn-1 /opt/homebrew/bin/fish -ilc 'claude --session-id 01a0dad0-afd0-7bd1-9d62-3d2f51027e9c'
74380 74359 74359 ttys011  SN+  /opt/homebrew/bin/fish -ilc claude --session-id 01a0dad0-afd0-7bd1-9d62-3d2f51027e9c --settings '/tmp/fc-pn/home/claude-hooks.json'
74390 74380 74359 ttys011  SN+  claude --session-id 01a0dad0-afd0-7bd1-9d62-3d2f51027e9c --settings /tmp/fc-pn/home/claude-hooks.json
82706 82704 82706 ttys012  SNs+ fish -c env FARCOOLER_ACTOR=agent:… FARCOOLER_TASK=pn-4 /opt/homebrew/bin/fish -ilc 'codex -c check_for_update_on_startup=false …'
82744 82706 82706 ttys012  SN+  /opt/homebrew/bin/fish -ilc codex -c check_for_update_on_startup=false …
82895 82744 82706 ttys012  SN+  codex -c check_for_update_on_startup=false You're working pn-4 …
  742     1   742 ??       Ss   /usr/sbin/cfprefsd
";
}
