//! `workspace set`'s settings: what each flag asks the runner for, and what
//! is said back.

use clap::Parser;
use farcooler_protocol::capability;

use super::*;
use crate::workspaces::WorkspaceCmd;

fn parsed(line: &str) -> SettingsArgs {
    match crate::Cli::try_parse_from(line.split_whitespace()).expect(line).command {
        crate::Command::Workspace(WorkspaceCmd::Set { settings, .. }) => settings,
        _ => panic!("{line}"),
    }
}

fn sent(settings: &SettingsArgs) -> (Vec<String>, pb::WorkspaceSetSettings) {
    let r = settings.request(Uuid::from_u128(1), 7);
    let Some(request::Payload::WorkspaceSetSettings(p)) = r.payload else { panic!("payload") };
    (r.required_capabilities, p)
}

#[test]
fn each_landing_flag_reaches_the_wire_and_names_the_capability() {
    let s = parsed("farcooler workspace set Billing --landing pull-requests --base trunk --budget-lines 400 --pr-cost-line on");
    let (caps, p) = sent(&s);
    assert_eq!(caps, [capability::WORKSTREAMS, capability::LANDING], "an older runner refuses rather than dropping the fields");
    assert_eq!(p.landing, Some(pb::LandingMode::PullRequests as i32));
    assert_eq!((p.base.as_deref(), p.pr_max_lines, p.pr_cost_line), (Some("trunk"), Some(400), Some(true)));
    assert_eq!((p.wake_on_answer, p.expected_version), (None, Some(7)));

    let (_, p) = sent(&parsed("farcooler workspace set Billing --landing direct --pr-cost-line off"));
    assert_eq!((p.landing, p.pr_cost_line), (Some(pb::LandingMode::Direct as i32), Some(false)));
}

#[test]
fn a_wake_only_write_names_no_landing_capability_and_both_name_both() {
    let (caps, _) = sent(&parsed("farcooler workspace set Billing --wake-on-answer off"));
    assert_eq!(caps, [capability::WORKSTREAMS, capability::WAKE_ON_ANSWER]);
    let (caps, _) = sent(&parsed("farcooler workspace set Billing --wake-on-answer off --landing direct"));
    assert_eq!(caps, [capability::WORKSTREAMS, capability::WAKE_ON_ANSWER, capability::LANDING]);
}

#[test]
fn an_empty_base_and_zero_lines_ask_for_the_value_to_go() {
    let s = parsed("farcooler workspace set Billing --budget-lines 0");
    assert_eq!(sent(&s).1.pr_max_lines, Some(0));
    let cleared = SettingsArgs { base: Some("  ".into()), ..Default::default() };
    assert_eq!(sent(&cleared).1.base.as_deref(), Some(""));
}

#[test]
fn a_bad_mode_is_refused_by_the_parser_and_none_given_is_empty() {
    assert!(crate::Cli::try_parse_from(["farcooler", "workspace", "set", "Billing", "--landing", "squash"]).is_err());
    assert!(crate::Cli::try_parse_from(["farcooler", "workspace", "set", "Billing", "--budget-lines", "many"]).is_err());
    assert!(parsed("farcooler workspace set Billing").is_empty());
    assert!(!parsed("farcooler workspace set Billing --base trunk").is_empty());
    assert!(!parsed("farcooler workspace set Billing --wake-on-answer on").touches_landing());
}

fn board(name: &str) -> pb::Workspace {
    pb::Workspace { name: name.into(), ..Default::default() }
}

#[test]
fn what_was_set_is_said_one_line_each() {
    let s = parsed("farcooler workspace set Billing --landing pull-requests --base trunk --budget-lines 400 --pr-cost-line on");
    let w = pb::Workspace {
        landing: Some(pb::LandingMode::PullRequests as i32),
        base: Some("trunk".into()),
        pr_max_lines: Some(400),
        pr_cost_line: Some(true),
        ..board("Billing")
    };
    assert_eq!(
        s.said(&w),
        "Billing lands through pull requests now\n\
         Billing lands on trunk now\n\
         a pull request from Billing should stay under 400 changed lines\n\
         Billing's pull request descriptions show a cost line now"
    );
    let cleared = SettingsArgs {
        base: Some(String::new()),
        budget_lines: Some(0),
        pr_cost_line: Some(OnOff::Off),
        ..Default::default()
    };
    let none = pb::Workspace { pr_cost_line: Some(false), ..board("Billing") };
    assert_eq!(
        cleared.said(&none),
        "Billing lands on the repository's default branch now\n\
         Billing has no line budget now\n\
         Billing's pull request descriptions leave out a cost line now"
    );
}

#[test]
fn a_direct_choice_that_cannot_work_says_so_and_says_nothing_was_switched() {
    let s = parsed("farcooler workspace set Billing --landing direct");
    let w = pb::Workspace {
        landing: Some(pb::LandingMode::Direct as i32),
        direct_refused: Some("Landing straight on main won't work. main requires pull requests.".into()),
        ..board("Billing")
    };
    assert_eq!(
        s.said(&w),
        "Billing lands straight on its base branch now\n\
         Landing straight on main won't work. main requires pull requests. Nothing was switched for you."
    );
}
