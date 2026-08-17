//! `farcooler client` — the command lines the Mac app builds.
//!
//! ARGUMENT PARSING, and deliberately nothing else. What the three methods do is
//! covered where the rules live: `crates/daemon/src/enrollment.rs` for the file,
//! `crates/client/src/session.rs` for the JSON shape three apps decode. It also
//! could not be exercised from out here — the scratch `authorized_keys` a test
//! writes into is a `Service` field with no environment override, so a daemon
//! started by this test would enroll into the real `~/.ssh/authorized_keys` of
//! whoever ran the suite. That is the one file in this product worth refusing to
//! touch from a test.
//!
//! Which leaves the half that was actually broken. `apps/macos/.../Enrollment.swift`
//! has been written against these command lines since before they existed, and
//! every one of them was a clap usage error: the `Command` enum had no `client`,
//! so a Mac could not enroll anything on any runner. These tests are the contract
//! between that file and this binary.

/// A runner that cannot exist, so nothing here reaches a daemon.
///
/// `.invalid` is reserved for exactly this by RFC 2606: no resolver anywhere will
/// ever answer for it, so ssh fails fast and locally rather than opening a
/// connection to somebody's real machine because a test fixture was a typo.
const NOWHERE: &str = "nobody@nowhere.invalid";

/// One key, in the shape a device generates it. Never used: ssh never connects.
const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGxampleExampleExampleExampleExampleExa";

struct Ran {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn farcooler(args: &[&str]) -> Ran {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .args(args)
        .output()
        .expect("run farcooler");
    Ran {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

impl Ran {
    /// Did clap refuse this before the command ever ran?
    ///
    /// Exit 2 is clap's usage failure and nothing else in this binary produces
    /// it — a command that got as far as the network and failed there exits 1 —
    /// and clap prints a `Usage:` line with every one. So this separates "your
    /// arguments are wrong", which is what these tests are about, from "that
    /// runner does not exist", which is the expected outcome of every command
    /// here and not a failure.
    fn was_refused_by_clap(&self) -> bool {
        self.code == Some(2) || self.stderr.contains("Usage:")
    }
}

/// `--help` teaches the subcommand exists at all.
#[test]
fn help_lists_the_client_subcommand() {
    let ran = farcooler(&["--help"]);
    assert!(ran.code == Some(0), "`farcooler --help` failed: {}", ran.stderr);
    // A LINE that starts with the word, not the word anywhere in the page: the
    // summary for `pane-host` already contains "client", so a substring search
    // passed against a binary that had no such subcommand at all.
    assert!(
        ran.stdout.lines().any(|line| line.trim_start().starts_with("client ")),
        "no client subcommand in help:\n{}",
        ran.stdout
    );
}

/// All three methods the daemon serves have a command.
#[test]
fn client_help_lists_list_enroll_and_revoke() {
    let ran = farcooler(&["client", "--help"]);
    assert!(ran.code == Some(0), "`farcooler client --help` failed: {}", ran.stderr);
    for word in ["list", "enroll", "revoke"] {
        assert!(ran.stdout.contains(word), "`client --help` never names {word}:\n{}", ran.stdout);
    }
}

/// A missing argument is named, not merely counted.
///
/// The whole point of a required flag here: `--key` and `--client-id` are what a
/// ceremony holds and `--scope` is what it decides, and being told "invalid
/// arguments" would send somebody re-reading a command line with five flags on it.
#[test]
fn enroll_names_the_argument_it_is_missing() {
    let ran = farcooler(&[
        "client", "enroll", "--label", "MacBook Air", "--client-id", "device-1", "--scope",
        "control",
    ]);
    assert!(ran.was_refused_by_clap(), "a missing --key was accepted:\n{}", ran.stderr);
    assert!(ran.stderr.contains("--key"), "the refusal never names --key:\n{}", ran.stderr);
}

/// `--shell-access` is a flag, so its ABSENCE is the restricted Key A line.
///
/// A value here — `--shell-access true` — would mean every caller written before
/// Key B existed had to be changed to go on asking for what it always asked for.
#[test]
fn shell_access_takes_no_value() {
    let ran = farcooler(&[
        "--json", "--runner", NOWHERE, "client", "enroll", "--key", KEY, "--label", "MacBook Air",
        "--client-id", "device-1", "--scope", "host_admin", "--shell-access",
    ]);
    assert!(!ran.was_refused_by_clap(), "--shell-access was not accepted bare:\n{}", ran.stderr);
}

/// The CLI does not re-implement the daemon's pairing rule.
///
/// `--shell-access --scope control` is refused, and refused BY THE DAEMON, which
/// owns every rule about what may be written into `authorized_keys`. A second copy
/// of that check here would be a second place for it to drift from the file's
/// authority — so this asserts the CLI hands the request over rather than judging
/// it, which from out here means: not a usage error.
#[test]
fn the_cli_does_not_second_guess_the_shell_access_pairing() {
    let ran = farcooler(&[
        "--json", "--runner", NOWHERE, "client", "enroll", "--key", KEY, "--label", "Some Mac",
        "--client-id", "device-1", "--scope", "control", "--shell-access",
    ]);
    assert!(
        !ran.was_refused_by_clap(),
        "the CLI judged a pairing that belongs to the daemon:\n{}",
        ran.stderr
    );
}

/// A scope this build does not have is refused, and the refusal says the words.
///
/// Refused rather than defaulted, because a key with no scope already means
/// `host_admin` to sshd: rounding a misspelling up is privilege escalation by
/// typo. The same rule `fence::scope_from_word` states, using that function.
#[test]
fn an_unknown_scope_is_refused_with_the_words_that_work() {
    let ran = farcooler(&[
        "--runner", NOWHERE, "client", "enroll", "--key", KEY, "--label", "Some Mac", "--client-id",
        "device-1", "--scope", "hostadmin",
    ]);
    assert!(ran.code != Some(0), "a misspelled scope was accepted");
    let said = format!("{}{}", ran.stdout, ran.stderr);
    for word in ["read", "control", "host_admin"] {
        assert!(said.contains(word), "the refusal never offers {word}:\n{said}");
    }
}

/// `--json` is accepted on all three, because it is what the Mac app parses.
#[test]
fn json_is_accepted_on_all_three() {
    let lines: [&[&str]; 3] = [
        &["--json", "--runner", NOWHERE, "client", "list"],
        &[
            "--json", "--runner", NOWHERE, "client", "enroll", "--key", KEY, "--label", "iPhone 17",
            "--client-id", "device-1", "--scope", "control",
        ],
        &["--json", "--runner", NOWHERE, "client", "revoke", "device-1"],
    ];
    for line in lines {
        let ran = farcooler(line);
        assert!(!ran.was_refused_by_clap(), "`{}` is not a command:\n{}", line.join(" "), ran.stderr);
    }
}

/// **The contract.** The two command lines `Enrollment.swift`'s doc comment
/// names, verbatim, in the order that file builds them.
///
/// A Mac is TWO enrollments under ONE client id — Key A restricted, Key B plain —
/// and these are the exact argument vectors `Enrollment.write` assembles. If this
/// test fails, the app cannot enroll this Mac anywhere, and it fails silently at
/// the last step of a ceremony somebody has already scanned a code for.
#[test]
fn the_two_command_lines_the_mac_builds_are_commands() {
    let client_id = "9f2a1c44-0d3b-4e77-9c21-6b5e0a7f1d88";

    let key_a: &[&str] = &[
        "--json", "--runner", "you@box", "client", "enroll", "--key", KEY, "--label", "iPhone 17",
        "--client-id", client_id, "--scope", "control",
    ];
    let key_b: &[&str] = &[
        "--json", "--runner", "you@box", "client", "enroll", "--key", KEY, "--label", "MacBook Air",
        "--client-id", client_id, "--scope", "host_admin", "--shell-access",
    ];

    for line in [key_a, key_b] {
        // `you@box` is the doc comment's own placeholder and is left exactly as
        // written: what is under test is that clap accepts the vector, and the
        // ssh that follows is expected to fail. Asserting on the failure would be
        // asserting on somebody's `~/.ssh/config`, which may well have a `box`.
        let ran = farcooler(line);
        assert!(
            !ran.was_refused_by_clap(),
            "Enrollment.swift builds a command this CLI refuses:\n  {}\n{}",
            line.join(" "),
            ran.stderr
        );
    }
}
