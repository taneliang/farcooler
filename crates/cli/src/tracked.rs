//! `task dispatch` asks the engine whether a ticket is tracked before it opens a pane (ov-962). The
//! question is answered by the agent dispatch code; this names it so the board's `task` verbs reach it
//! without naming the plan layer (ov-268).

pub(crate) use crate::plan::agent::refuse_tracked;
