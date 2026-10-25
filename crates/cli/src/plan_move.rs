//! `plan theme move <name> --to <n>` (ov-755): put a theme at place `n` in
//! the plan's ranking, 1 being first.
//!
//! The ranking is the themes' `ordinal`, which is all this writes. A bare
//! write of `n` would tie with whichever theme already holds it, and a tie
//! falls to the older theme, so the move would not put it first. So the
//! themes are renumbered 1, 2, 3 in their new order, and only the ones whose
//! number changes are written. The ranking is one the whole team shares
//! (ov-755): anyone, a rep for its person included, may move a theme.

use super::*;

/// The ordinal each theme in `ranked` (id and current ordinal, in the
/// ranking's order) should hold once `moving` sits at place `to`: only those
/// whose ordinal changes, in the order to write them.
pub(super) fn renumbered(ranked: &[(bytes::Bytes, i64)], moving: &bytes::Bytes, to: usize) -> Vec<(bytes::Bytes, i64)> {
    let mut order: Vec<&(bytes::Bytes, i64)> = ranked.iter().filter(|(id, _)| id != moving).collect();
    if let Some(held) = ranked.iter().find(|(id, _)| id == moving) {
        order.insert((to - 1).min(order.len()), held);
    }
    order
        .iter()
        .enumerate()
        .filter(|(i, (_, ordinal))| *ordinal != *i as i64 + 1)
        .map(|(i, (id, _))| (id.clone(), i as i64 + 1))
        .collect()
}

/// `plan theme move`: the themes renumbered with `name` at place `to`.
pub(super) async fn move_theme<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    plan: &pb::Plan,
    name: &str,
    to: u32,
    actor: &str,
    json: bool,
) -> Result<String, Failed> {
    let moving = find_theme(plan, name)?;
    let Some(theme) = moving.theme.as_ref() else { return Err(unreadable()) };
    let ranked: Vec<(bytes::Bytes, i64)> =
        plan.themes.iter().filter_map(|v| v.theme.as_ref()).map(|t| (t.id.clone(), t.ordinal)).collect();
    if to == 0 || to as usize > ranked.len() {
        return Err(format!("There are {} themes here, so --to takes 1 to {}.", ranked.len(), ranked.len()).into());
    }
    for (id, ordinal) in renumbered(&ranked, &theme.id, to as usize) {
        let p = request::Payload::BoardThemeUpdate(pb::BoardThemeUpdate {
            theme_id: id,
            ordinal: Some(ordinal),
            actor: actor.into(),
            ..Default::default()
        });
        send(link, board, "board_theme.update", p).await?;
    }
    let said = format!("Moved theme {:?} to place {to} of {}.", theme.name, ranked.len());
    Ok(if json { serde_json::json!({ "theme": theme.name, "place": to, "of": ranked.len() }).to_string() } else { said })
}

