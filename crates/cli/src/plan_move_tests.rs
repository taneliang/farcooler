//! `plan theme move` (ov-755): the themes renumbered so the moved one holds
//! the place asked, and a request a rep's pane may make.

use super::*;

fn ranked(ordinals: &[i64]) -> Vec<(bytes::Bytes, i64)> {
    ordinals.iter().enumerate().map(|(i, o)| (tid(0x3100 + i as u128), *o)).collect()
}

/// A plan of three themes, Alpha first, at the ordinals given.
fn three(ordinals: [i64; 3]) -> pb::Plan {
    let mut plan = the_plan();
    plan.themes = ["Alpha", "Beta", "Gamma"]
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let mut view = theme(i as u128 + 10, name, &[], 0);
            view.theme.as_mut().unwrap().ordinal = ordinals[i];
            view
        })
        .collect();
    plan
}

/// What `theme move` wrote: each theme named by its first letter, with its
/// new ordinal.
fn written(link: &Runner) -> Vec<(char, i64)> {
    link.sent
        .iter()
        .filter_map(|r| match &r.payload {
            Some(request::Payload::BoardThemeUpdate(p)) => {
                let n = Uuid::from_slice(&p.theme_id).unwrap().as_u128() - 0x3000 - 10;
                Some((['A', 'B', 'G'][n as usize], p.ordinal.expect("only the place is written")))
            }
            _ => None,
        })
        .collect()
}

/// A theme moved up takes the place asked and pushes the ones it passes
/// down; only the themes whose number changes are written.
#[test]
fn the_themes_are_renumbered_around_the_one_moved() {
    let r = ranked(&[1, 2, 3]);
    let ids: Vec<_> = r.iter().map(|(id, _)| id.clone()).collect();
    let moved = |to| theme_move::renumbered(&r, &ids[2], to);
    assert_eq!(moved(1), [(ids[2].clone(), 1), (ids[0].clone(), 2), (ids[1].clone(), 3)]);
    assert_eq!(moved(2), [(ids[2].clone(), 2), (ids[1].clone(), 3)]);
    assert!(moved(3).is_empty(), "already there: nothing to write");
    assert_eq!(theme_move::renumbered(&r, &ids[0], 3), [(ids[1].clone(), 1), (ids[2].clone(), 2), (ids[0].clone(), 3)]);
    // A ranking with gaps and ties is made whole, not left to the tie-break.
    let tied = ranked(&[1, 1, 7]);
    assert_eq!(theme_move::renumbered(&tied, &tied[1].0, 1), [(tied[0].0.clone(), 2), (tied[2].0.clone(), 3)]);
}

/// `theme move` writes only `ordinal`, as the actor, naming the capability,
/// and every request is one a rep's pane may make.
#[tokio::test]
async fn a_theme_is_moved_by_writing_the_ordinals() {
    let mut link = runner();
    link.plan = Some(three([1, 2, 3]));
    let out = say(&mut link, "theme move gamma --to 1").await.unwrap();
    assert_eq!(out, "Moved theme \"Gamma\" to place 1 of 3.");
    assert_eq!(written(&link), [('G', 1), ('A', 2), ('B', 3)]);
    for r in link.sent.iter().filter(|r| r.method == "board_theme.update") {
        assert_eq!(r.required_capabilities, [capability::BOARD_PLAN.to_string()]);
        let Some(request::Payload::BoardThemeUpdate(p)) = &r.payload else { panic!() };
        assert_eq!((p.actor.as_str(), p.name.is_none(), p.story.is_none(), p.state.is_none(), p.budget_tokens.is_none()), ("manager", true, true, true, true));
        assert!(farcooler_daemon::people::rep_may(r), "a rep's pane may send it");
    }
}

fn runner_with(plan: pb::Plan) -> Runner {
    Runner { plan: Some(plan), ..runner() }
}

/// A place off the ranking is refused before anything is written, and a move
/// to where the theme already is writes nothing.
#[tokio::test]
async fn a_place_off_the_ranking_is_refused() {
    let mut link = runner_with(three([1, 2, 3]));
    let err = say(&mut link, "theme move beta --to 4").await.unwrap_err();
    assert_eq!(err.to_string(), "There are 3 themes here, so --to takes 1 to 3.");
    assert!(link.sent.iter().all(|r| r.method != "board_theme.update"), "nothing written");
    let mut link = runner_with(three([1, 2, 3]));
    say(&mut link, "theme move beta --to 2").await.unwrap();
    assert!(written(&link).is_empty());
    let err = say(&mut link, "theme move nope --to 1").await.unwrap_err();
    assert_eq!(err.to_string(), "No theme here is called \"nope\".");
}
