//! A theme's `last_moved_at` (ov-331), term by term: its story, its cards'
//! own clock, a live lane, a dropped one, and its rulings. Each theme below is
//! built so that removing one term, or the filter that keeps another theme's
//! ruling off it, changes exactly one answer.

use super::*;

use crate::models::Actor;
use crate::plan::{LaneCard, NewLane, NewTheme};
use crate::rulings::NewRuling;

fn conn_set(store: &Store, sql: &str) {
    store.conn().execute_batch(sql).unwrap();
}

#[test]
fn a_theme_last_moved_is_the_newest_of_its_story_cards_lanes_and_rulings() {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let theme_and_card = |name: &str| {
        let task = store.create_task(main, &format!("card of {name}"), Actor::Manager).unwrap();
        let theme = store
            .create_theme(main, &NewTheme { name: name.into(), outcome: String::new() }, &[task.id], Actor::Manager)
            .unwrap();
        (theme, task)
    };
    let (ruled, ruled_card) = theme_and_card("Ruled");
    let (_dropped, dropped_card) = theme_and_card("Dropped");
    let (_live, live_card) = theme_and_card("Live");
    let (told, _) = theme_and_card("Told");
    let (edited, edited_card) = theme_and_card("Edited");
    let (other, _) = theme_and_card("Other");
    let _ = other;

    // One baseline for every card and story, so each term below is the only newer thing on its theme.
    conn_set(&store, "UPDATE tasks SET created_at = 1000, status_since = 1000, edited_at = NULL");
    conn_set(&store, "UPDATE board_themes SET story_at = 1000");

    // A ruling on Ruled only, made at 9000 and settled at 9500.
    let ruling = store
        .add_ruling(
            main,
            &NewRuling { decision: "d".into(), why: "w".into(), reversal: "r".into(), theme_id: Some(ruled.id) },
            &[],
            Actor::Manager,
        )
        .unwrap();
    conn_set(&store, &format!("UPDATE board_rulings SET created_at = 9000, settled_at = 9500, state = 'confirmed' WHERE number = {}", ruling.number));

    // A dropped lane on Dropped's card, newer than anything: it is gone, and must not count.
    let dropped_lane = store
        .create_lane(
            main,
            &NewLane { name: "gone".into(), ..Default::default() },
            &[LaneCard { task_id: dropped_card.id, slice: String::new() }],
            None,
            Actor::Manager,
        )
        .unwrap();
    conn_set(&store, &format!("UPDATE lanes SET state = 'dropped', state_since = 20000 WHERE name = '{}'", dropped_lane.name));

    // A live lane on Live's card at 7000.
    store
        .create_lane(
            main,
            &NewLane { name: "busy".into(), ..Default::default() },
            &[LaneCard { task_id: live_card.id, slice: String::new() }],
            None,
            Actor::Manager,
        )
        .unwrap();
    conn_set(&store, "UPDATE lanes SET state_since = 7000 WHERE name = 'busy'");

    // A story written at 12000 on Told; a card edited at 15000 on Edited (an edit isn't a status move).
    conn_set(&store, &format!("UPDATE board_themes SET story_at = 12000 WHERE name = '{}'", told.name));
    conn_set(&store, &format!("UPDATE tasks SET edited_at = 15000 WHERE id = x'{}'", hex(edited_card.id)));
    let _ = (ruled_card, edited);

    let plan = store.plan(main, 0).unwrap();
    let at = |name: &str| plan.themes.iter().find(|v| v.theme.name == name).unwrap().last_moved_at;
    assert_eq!(at("Ruled"), 9500, "its ruling, settled");
    assert_eq!(at("Dropped"), 1000, "a dropped lane, and another theme's ruling, don't move it");
    assert_eq!(at("Live"), 7000, "its live lane");
    assert_eq!(at("Told"), 12000, "its story");
    assert_eq!(at("Edited"), 15000, "its card's own clock, which counts an edit");
    assert_eq!(at("Other"), 1000, "nothing newer than its card and story touches it");
}

fn hex(id: Uuid) -> String {
    id.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}
