//! What is actually running in each terminal, arguments included.
//!
//! `pane_current_command` is a process NAME, which is the wrong grain for a label
//! people read: `pnpm dev` shows as `node`, `cargo build` as `cargo`, and every
//! idle shell as `zsh`. What distinguishes one pane from another is usually the
//! arguments — `pnpm dev` from `pnpm test`, `cargo build` from `cargo test`.
//!
//! So the foreground process group of each pane's tty is read from `ps`, which
//! has the argv. One call for the whole host per sample, not one per pane: this
//! sits on the watcher's loop, and a fleet of thirty panes must not mean thirty
//! processes a second.

use std::collections::HashMap;

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
}

impl Foreground {
    /// What a tty is showing, if anything.
    pub fn pane(&self, tty: &str) -> Option<&Running> {
        self.panes.get(tty)
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
}

/// Read every tty's foreground command, and every process's group.
///
/// `stat` carries `+` for a process in its terminal's foreground group, which is
/// exactly "the thing you are looking at" — the shell itself is `Ss` and gets
/// skipped, so an idle pane reports nothing and keeps whatever tmux called it.
pub async fn read() -> Foreground {
    let out = tokio::process::Command::new("ps")
        .args(["-axo", "pid=,pgid=,tty=,stat=,args="])
        .stdin(std::process::Stdio::null())
        .output()
        .await;
    let Ok(out) = out else { return Foreground::default() };
    parse(&String::from_utf8_lossy(&out.stdout))
}

/// Split out from `read` so the column layout is testable.
///
/// It has changed twice now, to carry the pid and then the pgid, and a silent
/// misparse would cost every label its arguments while everything kept running.
fn parse(stdout: &str) -> Foreground {
    let mut found = Foreground::default();
    for line in stdout.lines() {
        let Some(row) = row(line) else { continue };
        // Every process, including the ones with no terminal: this is the table
        // the ports join reads, and the process holding a socket may be a
        // daemonized child that has left its tty behind.
        found.groups.insert(row.pid, row.pgid);
        if !row.stat.contains('+') || row.tty == "??" || row.args.is_empty() {
            continue;
        }
        // First wins. `ps` lists a foreground pipeline's members in order, and the
        // first is the one that was typed — `rg foo | less` should read as `rg`.
        found.panes.entry(row.tty.to_string()).or_insert_with(|| Running {
            pid: row.pid,
            pgid: row.pgid,
            command: summarize(row.args),
        });
    }
    found
}

/// The columns of one `ps` row.
struct Row<'a> {
    pid: i32,
    pgid: i32,
    tty: &'a str,
    stat: &'a str,
    args: &'a str,
}

fn row(line: &str) -> Option<Row<'_>> {
    let (pid, rest) = line.trim_start().split_once(char::is_whitespace)?;
    let (pgid, rest) = rest.trim_start().split_once(char::is_whitespace)?;
    let (tty, rest) = rest.trim_start().split_once(char::is_whitespace)?;
    let (stat, args) = rest.trim_start().split_once(char::is_whitespace)?;
    Some(Row {
        pid: pid.parse().ok()?,
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

    /// Real `ps -axo pid=,pgid=,tty=,stat=,args=` output, right-aligned columns
    /// included.
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
        assert_eq!(f.pane("ttys000").map(|r| r.pid), Some(48436));
        // A tty nobody is looking at.
        assert_eq!(f.pane("ttys999"), Option::None);
    }

    /// A wrapper holds no socket; its child does, and shares its group.
    ///
    /// This is the case the pid lookup missed, and it is most real dev servers:
    /// `pnpm dev`, `npm run dev`, anything behind a shell script. `lsof` names
    /// the child, the pane shows the wrapper, and only the group joins them.
    #[test]
    fn a_socket_held_by_a_child_still_belongs_to_the_pane() {
        let f = parse(PS);
        let ports = HashMap::from([(60063, vec![18299]), (22910, vec![8099])]);
        let by_group = f.ports_by_group(&ports);

        // The pane is showing the wrapper, which holds nothing itself.
        let wrapper = f.pane("ttys009").expect("a wrapper is a foreground process");
        assert_eq!(wrapper.pid, 60061);
        assert_eq!(ports.get(&wrapper.pid), Option::None, "the wrapper holds no socket");
        assert_eq!(by_group.get(&wrapper.pgid), Some(&vec![18299]), "its group does");

        // A server started directly still resolves, through a group of one.
        let direct = f.pane("ttys003").expect("a server is a foreground process");
        assert_eq!(by_group.get(&direct.pgid), Some(&vec![8099]));
    }

    /// A socket whose process `ps` never saw belongs to no group we can name.
    #[test]
    fn a_process_that_started_between_the_two_reads_is_dropped() {
        let f = parse(PS);
        let by_group = f.ports_by_group(&HashMap::from([(99999, vec![7000])]));
        assert!(by_group.is_empty(), "{by_group:?}");
    }

    /// Verbatim `ps -axo pid=,pgid=,tty=,stat=,args=`, with a wrapper pairing
    /// added — the two rows 60061/60063 were observed live.
    const PS: &str = "\
48417 48417 ttys000  Ss   fish -c /opt/homebrew/bin/fish -il
48436 48436 ttys000  S+   /opt/homebrew/bin/fish -il
 1758  1758 ttys001  Ss   /opt/homebrew/bin/fish -l
 5023  5023 ttys001  S+   /Users/e-liang/.local/bin/claude
22910 22910 ttys003  S+   /usr/bin/python3 -m http.server 8099
60061 60061 ttys009  S+   bash -c python3 -m http.server 18299 & wait
60063 60061 ttys009  S+   /usr/bin/python3 -m http.server 18299
  742   742 ??       Ss   /usr/sbin/cfprefsd
";
}
