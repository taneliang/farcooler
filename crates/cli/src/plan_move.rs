//! `plan theme move <name> --to <n>` (ov-755): put a theme at place `n` in
//! the plan's ranking, 1 being first. One call, `board_theme.move`: the
//! runner renumbers the themes in one write and records who moved it, so two
//! people moving at once never leave a half-numbered ranking, and a rep's
//! move is filed under its member. The ranking is one the whole team shares:
//! anyone, a rep for its person included, may move a theme.

use super::*;

/// `board_theme.move` for `theme` to place `to`.
pub(super) fn move_payload(theme: bytes::Bytes, to: u32, actor: &str) -> request::Payload {
    request::Payload::BoardThemeMove(pb::BoardThemeMove { theme_id: theme, to, actor: actor.into() })
}

/// `plan theme move`: the theme `name` at place `to`.
pub(super) async fn move_theme<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    plan: &pb::Plan,
    name: &str,
    to: u32,
    actor: &str,
    json: bool,
) -> Result<String, Failed> {
    if !link.capabilities().iter().any(|c| c == capability::BOARD_THEME_MOVE) {
        return Err("This runner needs an update to move a theme.".into());
    }
    let moving = find_theme(plan, name)?;
    let Some(theme) = moving.theme.as_ref() else { return Err(unreadable()) };
    let of = plan.themes.len();
    if to == 0 || to as usize > of {
        return Err(format!("There are {of} themes here, so --to takes 1 to {of}.").into());
    }
    let mut r = with(req_for("board_theme.move", board.repository), move_payload(theme.id.clone(), to, actor));
    r.required_capabilities.extend([capability::BOARD_PLAN, capability::BOARD_THEME_MOVE].map(String::from));
    let answer = link.call(r).await.map_err(|e| refused_here(e, "The runner couldn't record that. Try again."))?;
    expect_value(answer.value)?;
    Ok(if json {
        serde_json::json!({ "theme": theme.name, "place": to, "of": of }).to_string()
    } else {
        format!("Moved theme {:?} to place {to} of {of}.", theme.name)
    })
}
