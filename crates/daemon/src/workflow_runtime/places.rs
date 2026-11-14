//! The places effects run in, by pool, and what waits for one (ov-1198). Agent sessions keep a cap that bounds CPU;
//! a landing group and short steps have places of their own, so neither ever waits behind a build.
use super::*;

/// Most agent sessions (builds and reviews) that run at once across all workspaces.
pub const DEFAULT_EFFECT_CAP: usize = 4;
/// Most steps of landing groups (fold, gate, push and the rest) that run at once. Each group holds its landing
/// target, so one group per target works at a time anyway.
pub const DEFAULT_LANDING_PLACES: usize = 2;
/// Most other effects (provisions, checks, CI, pull request steps) that run at once.
pub const DEFAULT_WORK_PLACES: usize = 4;

/// Which places an effect takes while it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Pool { Agent, Landing, Work }
impl Pool {
    const ALL: [Pool; 3] = [Pool::Agent, Pool::Landing, Pool::Work];
    /// The pool an effect of `item` runs in. None for an effect that takes no place: a stop, which must always get
    /// through, and a wait on the base, CI or a pull request, which does no work of its own.
    pub(crate) fn of(effect: &Effect, item: &Item) -> Option<Pool> {
        match &effect.kind {
            EffectKind::Stop { .. } => None,
            EffectKind::Execute(StageKind::AwaitBase | StageKind::AwaitCi | StageKind::AwaitPr) => None,
            _ if item.group_of.is_some() => Some(Pool::Landing),
            EffectKind::Execute(StageKind::Build | StageKind::Review(_)) => Some(Pool::Agent),
            EffectKind::Execute(_) => Some(Pool::Work),
        }
    }
}

/// One effect that holds a place, or waits for one.
#[derive(Clone)]
pub(crate) struct Placed { pub workspace: String, pub item: String, pub effect: String, pub kind: EffectKind,
    pub pool: Pool, pub since_ms: i64 }
impl Placed {
    pub(crate) fn new(workspace: &str, effect: &Effect, pool: Pool) -> Self {
        Self { workspace: workspace.into(), item: effect.item.clone(), effect: effect.id.clone(), kind: effect.kind.clone(),
            pool, since_ms: effect.created_ms }
    }
}

/// How many places each pool has, what holds them and what waits, as the last scan left them. Shared, so a test can
/// read it while a sweep runs; `workflow get` reads it from the runtime.
pub(crate) struct Places { pub agent: usize, pub landing: usize, pub work: usize, pub holding: Vec<Placed>, pub waiting: Vec<Placed> }
impl Default for Places {
    fn default() -> Self {
        Self { agent: DEFAULT_EFFECT_CAP, landing: DEFAULT_LANDING_PLACES, work: DEFAULT_WORK_PLACES,
            holding: Vec::new(), waiting: Vec::new() }
    }
}
impl Places {
    pub(crate) fn cap(&self, pool: Pool) -> usize {
        match pool { Pool::Agent => self.agent, Pool::Landing => self.landing, Pool::Work => self.work }
    }
    /// What `workflow get` shows for `workspace`: each pool's cap, what holds its places and what waits, with each
    /// waiting effect's place in its pool's line. Another board's effects are counted, never named.
    pub(crate) fn view(&self, workspace: &str) -> serde_json::Value {
        let entry = |p: &Placed| serde_json::json!({"item": p.item, "effect": p.effect, "kind": p.kind, "since_ms": p.since_ms});
        let mut view = serde_json::Map::new();
        for pool in Pool::ALL {
            let held: Vec<_> = self.holding.iter().filter(|p| p.pool == pool).collect();
            let key = serde_json::to_value(pool).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
            view.insert(key, serde_json::json!({"cap": self.cap(pool), "used": held.len(),
                "holding": held.iter().filter(|p| p.workspace == workspace).map(|p| entry(p)).collect::<Vec<_>>(),
                "elsewhere": held.iter().filter(|p| p.workspace != workspace).count()}));
        }
        let waiting: Vec<_> = self.waiting.iter().enumerate().filter(|(_, p)| p.workspace == workspace).map(|(index, p)| {
            let position = 1 + self.waiting[..index].iter().filter(|q| q.pool == p.pool).count();
            let mut value = entry(p);
            value["pool"] = serde_json::json!(p.pool);
            value["position"] = serde_json::json!(position);
            value
        }).collect();
        view.insert("waiting".into(), serde_json::json!(waiting));
        serde_json::Value::Object(view)
    }
}
