//! `plan theme move` (ov-755): one `board_theme.move` call naming the theme,
//! the place and the actor, which a rep's pane may make.

use super::*;

/// A plan of three themes, Alpha first.
fn three() -> pb::Plan {
    let mut plan = the_plan();
    plan.themes = ["Alpha", "Beta", "Gamma"].iter().enumerate().map(|(i, name)| theme(i as u128 + 10, name, &[], 0)).collect();
    plan
}

fn runner_with(plan: pb::Plan) -> Runner {
    Runner { plan: Some(plan), ..runner() }
}

/// `theme move` is one write: the theme, the place and the actor, naming the
/// capabilities, and a request a rep's pane may make.
#[tokio::test]
async fn a_theme_is_moved_in_one_call() {
    let mut link = runner_with(three());
    let out = say(&mut link, "theme move gamma --to 1").await.unwrap();
    assert_eq!(out, "Moved theme \"Gamma\" to place 1 of 3.");
    let moves: Vec<_> = link.sent.iter().filter(|r| r.method.starts_with("board_theme.")).collect();
    assert_eq!(moves.len(), 1, "one write, never one per theme");
    let r = moves[0];
    assert_eq!(r.method, "board_theme.move");
    assert_eq!(r.required_capabilities, [capability::BOARD_PLAN, capability::BOARD_THEME_MOVE].map(String::from));
    let Some(request::Payload::BoardThemeMove(p)) = &r.payload else { panic!("{:?}", r.payload) };
    assert_eq!((p.theme_id.clone(), p.to, p.actor.as_str()), (tid(0x3000 + 12), 1, "manager"));
    assert!(farcooler_daemon::people::rep_may(r), "a rep's pane may send it");
}

/// A place off the ranking and a theme that isn't there are refused before
/// anything is written; a runner without the method is told to update.
#[tokio::test]
async fn a_move_that_cannot_be_made_sends_nothing() {
    let mut link = runner_with(three());
    let err = say(&mut link, "theme move beta --to 4").await.unwrap_err();
    assert_eq!(err.to_string(), "There are 3 themes here, so --to takes 1 to 3.");
    let err = say(&mut link, "theme move nope --to 1").await.unwrap_err();
    assert_eq!(err.to_string(), "No theme here is called \"nope\".");
    link.capabilities.retain(|c| c != capability::BOARD_THEME_MOVE);
    let err = say(&mut link, "theme move beta --to 1").await.unwrap_err();
    assert_eq!(err.to_string(), "This runner needs an update to move a theme.");
    assert!(link.sent.iter().all(|r| r.method != "board_theme.move"));
}
