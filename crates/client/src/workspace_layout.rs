//! The web workspace's two-leaf layout and command policy.

use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Slot {
    #[default]
    Main,
    Side,
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
enum Side {
    Empty,
    Orchestrator,
    Terminal(String),
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    #[default]
    Conversation,
    Terminal,
}

#[derive(Debug, Deserialize, Serialize)]
struct Layout {
    version: u8,
    main: Option<String>,
    side: Option<Side>,
    focus: Slot,
    modes: BTreeMap<String, Mode>,
}

impl Default for Layout {
    fn default() -> Self {
        Self { version: 1, main: None, side: None, focus: Slot::Main, modes: BTreeMap::new() }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Page {
    Terminals,
    Needs,
    Board,
    Plan,
    Ticket,
    Workflow,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Terminal {
    id: String,
    title: String,
    running: bool,
    agent: bool,
    attach: bool,
    attach_note: String,
    rename: Option<WorkAction>,
    close: Option<WorkAction>,
    recover: Option<WorkAction>,
}

#[derive(Deserialize)]
struct WorkAction {
    label: String,
}

#[derive(Deserialize)]
struct Context {
    page: Page,
    workspace: bool,
    workspace_name: Option<String>,
    workspace_count: usize,
    runner_name: String,
    start_work: Option<String>,
    #[serde(default)]
    missing_terminal: bool,
    terminals: Vec<Terminal>,
    orchestrator: Option<String>,
}

impl Context {
    fn terminal_notice(&self) -> Cow<'static, str> {
        if !self.terminals.is_empty() {
            return "Select a terminal to open in this pane.".into();
        }
        let name = match &self.workspace_name {
            Some(name) if self.workspace_count > 1 => name,
            _ => &self.runner_name,
        };
        match &self.start_work {
            Some(action) => format!("{name} has no terminals yet. Use {action} to open one.").into(),
            None => format!("{name} has no terminals yet.").into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
enum Command {
    Commands,
    Shortcuts,
    Terminals,
    Needs,
    Board,
    Plan,
    Workflow,
    Orchestrator,
    Split,
    CloseSplit,
    FocusOther,
    Conversation,
    RawTerminal,
    ClosePage,
    StartWork,
    RenameTerminal,
    CloseTerminal,
    RecoverTerminal,
}

impl Command {
    fn label(self) -> &'static str {
        match self {
            Self::Commands => "Commands",
            Self::Shortcuts => "Keyboard Shortcuts",
            Self::Terminals => "Terminals",
            Self::Needs => "Needs You",
            Self::Board => "Board",
            Self::Plan => "Plan",
            Self::Workflow => "Workflow Runs",
            Self::Orchestrator => "Orchestrator",
            Self::Split => "Split Pane",
            Self::CloseSplit => "Close Split",
            Self::FocusOther => "Focus Other Pane",
            Self::Conversation => "Chat",
            Self::RawTerminal => "Raw Terminal",
            Self::ClosePage => "Close Page",
            Self::StartWork => "Start Work",
            Self::RenameTerminal => "Rename Terminal",
            Self::CloseTerminal => "Close Terminal",
            Self::RecoverTerminal => "Recover Terminal",
        }
    }

    fn default_key(self) -> Option<&'static str> {
        match self {
            Self::Commands => Some("k"),
            Self::Orchestrator => Some("o"),
            Self::Split => Some("s"),
            Self::FocusOther => Some("f"),
            _ => None,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Intent {
    #[default]
    Present,
    Focus { slot: Slot },
    Open { id: String, slot: Option<Slot> },
    Run { command: Command, slot: Option<Slot> },
    Key { key: String, alt: bool, shift: bool, ctrl: bool, meta: bool, editing: bool, terminal: bool, modal: bool },
    Bind { command: Command, key: Option<String> },
}

#[derive(Deserialize)]
struct Request {
    #[serde(default)]
    layout: serde_json::Value,
    context: Context,
    #[serde(default)]
    bindings: BTreeMap<Command, Option<String>>,
    #[serde(default)]
    intent: Intent,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Effect {
    None,
    Focus { slot: Slot },
    Navigate { page: Page },
    Commands,
    Shortcuts,
    ClosePage,
    RefreshPane,
    Work { command: Command },
}

#[derive(Serialize)]
struct Pane<'a> {
    terminal: Option<&'a str>,
    title: &'a str,
    mode: Mode,
    agent: bool,
    attach: bool,
    notice: Cow<'a, str>,
}

#[derive(Serialize)]
struct Action<'a> {
    id: Command,
    label: &'a str,
    key: Option<String>,
    enabled: bool,
}

const COMMANDS: [Command; 18] = [
    Command::Commands, Command::Shortcuts, Command::Terminals, Command::Needs, Command::Board, Command::Plan,
    Command::Workflow, Command::Orchestrator, Command::Split, Command::CloseSplit, Command::FocusOther,
    Command::Conversation, Command::RawTerminal, Command::ClosePage, Command::StartWork,
    Command::RenameTerminal, Command::CloseTerminal, Command::RecoverTerminal,
];

impl Layout {
    fn normalize(&mut self, context: &Context) {
        let present = |id: &str| context.terminals.iter().any(|t| t.id == id);
        if matches!(&self.side, Some(Side::Terminal(id)) if !present(id)) {
            self.side = Some(Side::Empty);
        }
        if self.main.as_deref().is_none_or(|id| !present(id)) {
            self.main = context.terminals.iter().min_by_key(|t| !t.running).map(|t| t.id.clone());
        }
        if self.side.is_none() {
            self.focus = Slot::Main;
        }
    }

    fn side_id<'a>(&'a self, context: &'a Context) -> Option<&'a str> {
        match &self.side {
            Some(Side::Orchestrator) => context.orchestrator.as_deref(),
            Some(Side::Terminal(id)) => Some(id),
            _ => None,
        }
    }

    fn pane<'a>(&'a self, slot: Slot, context: &'a Context) -> Pane<'a> {
        let orchestrator = slot == Slot::Side && self.side == Some(Side::Orchestrator);
        let id = match slot {
            Slot::Main if context.page == Page::Terminals && !context.missing_terminal => {
                let side = self.side_id(context);
                self.main.as_deref().filter(|id| Some(*id) != side).or_else(|| {
                    context.terminals.iter().filter(|t| Some(t.id.as_str()) != side)
                        .min_by_key(|t| !t.running).map(|t| t.id.as_str())
                })
            }
            Slot::Side => self.side_id(context),
            _ => None,
        };
        let listed = id.and_then(|id| context.terminals.iter().find(|t| t.id == id));
        let agent = id.is_some() && (context.orchestrator.as_deref() == id || listed.is_some_and(|t| t.agent));
        Pane {
            terminal: id,
            title: if orchestrator { "Orchestrator" } else {
                listed.map(|t| t.title.as_str()).filter(|title| !title.is_empty()).unwrap_or(match slot {
                    Slot::Main => "Main Pane",
                    Slot::Side => "Split Pane",
                })
            },
            mode: if agent { id.and_then(|id| self.modes.get(id)).copied().unwrap_or_default() } else { Mode::Terminal },
            agent,
            attach: listed.is_some_and(|terminal| terminal.attach),
            notice: match (slot, listed, orchestrator) {
                (_, Some(terminal), _) => terminal.attach_note.as_str().into(),
                (Slot::Side, None, true) => "No orchestrator is available for this workspace yet.".into(),
                (Slot::Side, None, false) => context.terminal_notice(),
                (Slot::Main, None, _) if context.page == Page::Terminals && !context.missing_terminal => context.terminal_notice(),
                _ => "".into(),
            },
        }
    }

    fn work_label<'a>(&self, command: Command, slot: Slot, context: &'a Context) -> Option<&'a str> {
        if command == Command::StartWork {
            return context.start_work.as_deref();
        }
        if !matches!(command, Command::RenameTerminal | Command::CloseTerminal | Command::RecoverTerminal) {
            return None;
        }
        let id = self.pane(slot, context).terminal?;
        let terminal = context.terminals.iter().find(|terminal| terminal.id == id)?;
        match command {
            Command::RenameTerminal => terminal.rename.as_ref(),
            Command::CloseTerminal => terminal.close.as_ref(),
            Command::RecoverTerminal => terminal.recover.as_ref(),
            _ => None,
        }.map(|action| action.label.as_str())
    }

    fn allowed(&self, command: Command, slot: Slot, context: &Context) -> bool {
        match command {
            Command::Board | Command::Orchestrator | Command::Workflow => context.workspace,
            Command::Split => self.side.is_none(),
            Command::CloseSplit | Command::FocusOther => self.side.is_some(),
            Command::Conversation | Command::RawTerminal => {
                let pane = self.pane(slot, context);
                pane.agent && pane.attach
            }
            Command::StartWork | Command::RenameTerminal | Command::CloseTerminal | Command::RecoverTerminal =>
                self.work_label(command, slot, context).is_some(),
            Command::ClosePage => matches!(context.page, Page::Ticket | Page::Workflow),
            _ => true,
        }
    }

    fn run(&mut self, command: Command, slot: Slot, context: &Context) -> Effect {
        if !self.allowed(command, slot, context) {
            return Effect::None;
        }
        match command {
            Command::Commands => Effect::Commands,
            Command::Shortcuts => Effect::Shortcuts,
            Command::Terminals => Effect::Navigate { page: Page::Terminals },
            Command::Needs => Effect::Navigate { page: Page::Needs },
            Command::Board => Effect::Navigate { page: Page::Board },
            Command::Plan => Effect::Navigate { page: Page::Plan },
            Command::Workflow => Effect::Navigate { page: Page::Workflow },
            Command::ClosePage => Effect::ClosePage,
            Command::StartWork | Command::RenameTerminal | Command::CloseTerminal | Command::RecoverTerminal =>
                Effect::Work { command },
            Command::Orchestrator => self.split(Side::Orchestrator),
            Command::Split => self.split(Side::Empty),
            Command::CloseSplit => {
                self.side = None;
                self.focus = Slot::Main;
                Effect::Focus { slot: self.focus }
            }
            Command::FocusOther => {
                self.focus = if self.focus == Slot::Main { Slot::Side } else { Slot::Main };
                Effect::Focus { slot: self.focus }
            }
            Command::Conversation | Command::RawTerminal => {
                if let Some(id) = self.pane(slot, context).terminal.map(str::to_owned) {
                    let mode = if command == Command::Conversation { Mode::Conversation } else { Mode::Terminal };
                    self.modes.insert(id, mode);
                }
                Effect::RefreshPane
            }
        }
    }

    fn split(&mut self, content: Side) -> Effect {
        self.side = Some(content);
        self.focus = Slot::Side;
        Effect::Focus { slot: self.focus }
    }

    fn open(&mut self, id: String, slot: Option<Slot>, context: &Context) -> Effect {
        if !context.terminals.iter().any(|t| t.id == id) {
            return Effect::None;
        }
        if let Some(slot) = slot.filter(|slot| *slot == Slot::Main || self.side.is_some()) {
            self.focus = slot;
        }
        if self.side_id(context) == Some(&id) {
            self.focus = Slot::Side;
        } else if self.focus == Slot::Side {
            self.side = Some(Side::Terminal(id));
        } else {
            self.main = Some(id);
        }
        if self.focus == Slot::Main && context.page != Page::Terminals {
            Effect::Navigate { page: Page::Terminals }
        } else {
            Effect::Focus { slot: self.focus }
        }
    }
}

fn binding(bindings: &BTreeMap<Command, Option<String>>, command: Command) -> Option<&str> {
    bindings.get(&command).map_or_else(|| command.default_key(), |key| key.as_deref())
}

fn valid_key(key: &str) -> bool {
    key.len() == 1 && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

pub fn json(request: &str) -> String {
    let Ok(mut request) = serde_json::from_str::<Request>(request) else { return "{}".into() };
    let context = &request.context;
    let mut layout = serde_json::from_value::<Layout>(request.layout).ok()
        .filter(|layout| layout.version == 1).unwrap_or_default();
    layout.normalize(context);
    request.bindings.retain(|_, key| key.as_deref().is_none_or(valid_key));
    let bindings = &mut request.bindings;
    let mut problem = "";
    let mut handled = false;
    let effect = match request.intent {
        Intent::Present => Effect::None,
        Intent::Focus { slot } => {
            if slot == Slot::Main || layout.side.is_some() { layout.focus = slot; }
            Effect::None
        }
        Intent::Open { id, slot } => layout.open(id, slot, context),
        Intent::Run { command, slot } => layout.run(command, slot.unwrap_or(layout.focus), context),
        Intent::Key { key, alt, shift, ctrl, meta, editing, terminal, modal } => {
            let key = key.to_ascii_lowercase();
            if alt && shift && !ctrl && !meta && !editing && !terminal && !modal {
                COMMANDS.into_iter().find(|c| binding(bindings, *c) == Some(&key) &&
                    layout.allowed(*c, layout.focus, context))
                    .map_or(Effect::None, |c| {
                        handled = true;
                        layout.run(c, layout.focus, context)
                    })
            } else { Effect::None }
        }
        Intent::Bind { command, key } => {
            let key = key.map(|key| key.trim().to_ascii_lowercase()).filter(|key| !key.is_empty());
            if key.as_deref().is_some_and(|key| !valid_key(key)) {
                problem = "Enter one letter or digit, or clear the field to remove the shortcut.";
            } else if key.as_deref().is_some_and(|key| COMMANDS.into_iter().any(|c| c != command && binding(bindings, c) == Some(key))) {
                problem = "That shortcut is already assigned.";
            } else {
                bindings.insert(command, key);
            }
            Effect::None
        }
    };
    layout.normalize(context);
    let actions: Vec<_> = COMMANDS.into_iter().map(|id| Action {
        id, label: layout.work_label(id, layout.focus, context).unwrap_or_else(|| id.label()), key: binding(bindings, id).map(str::to_owned),
        enabled: layout.allowed(id, layout.focus, context),
    }).collect();
    serde_json::json!({
        "layout": layout, "bindings": bindings, "focus": layout.focus, "effect": effect, "handled": handled, "problem": problem,
        "main": layout.pane(Slot::Main, context),
        "side": layout.side.as_ref().map(|_| layout.pane(Slot::Side, context)),
        "actions": actions, "close_label": "Close", "shortcut_prefix": "Alt+Shift+", "shortcut_help": "Shortcuts work outside terminals and text fields.",
    }).to_string()
}

/// Restore a page when its workspace or ticket can still be read.
pub fn page_available(page: &str, workspace: bool, ticket: bool) -> bool {
    match page {
        "board" | "workflow" => workspace,
        "ticket" => ticket,
        _ => true,
    }
}

/// Read the named orchestrator from the workspace's Plan reply.
pub fn orchestrator(plan: &str) -> Option<String> {
    let plan: serde_json::Value = serde_json::from_str(plan).ok()?;
    let agent = plan.get("agents")?.as_array()?.iter().find(|a| a["role"] == "orchestrator")?;
    agent["terminal"].as_str().filter(|id| !id.is_empty())
        .or_else(|| agent["id"].as_str()?.strip_prefix("pane:").filter(|id| !id.is_empty()))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{json, orchestrator, page_available};
    use serde_json::{Value, json as value};

    fn context(page: &str) -> Value {
        value!({
            "page": page, "workspace": true, "orchestrator": "orch",
            "workspace_name": "Billing", "workspace_count": 2, "runner_name": "Build Mac", "start_work": "Start Work",
            "terminals": [
                {"id": "orch", "title": "Runner agent", "running": true, "agent": true, "attach": true, "attachNote": ""},
                {"id": "shell", "title": "Build", "running": true, "agent": false, "attach": true, "attachNote": ""},
                {"id": "logs", "title": "Logs", "running": false, "agent": false, "attach": false, "attachNote": "This terminal is currently unavailable."},
            ],
        })
    }

    fn step(before: &Value, context: &Value, intent: Value) -> Value {
        serde_json::from_str(&json(&value!({
            "context": context, "layout": before["layout"],
            "bindings": before.get("bindings").cloned().unwrap_or_else(|| value!({})),
            "intent": intent,
        }).to_string())).unwrap()
    }

    fn run(before: &Value, context: &Value, command: &str) -> Value {
        step(before, context, value!({"kind": "run", "command": command}))
    }

    #[test]
    fn unavailable_work_cannot_attach_or_offer_management_commands() {
        let mut page = context("terminals");
        page["start_work"] = Value::Null;
        page["terminals"][0]["attach"] = value!(false);
        page["terminals"][0]["attachNote"] = value!("Read only");
        let view = run(&Value::Null, &page, "raw_terminal");
        assert_eq!(view["main"]["attach"], false);
        assert_eq!(view["main"]["notice"], "Read only");
        assert_eq!(view["effect"]["kind"], "none");
        assert_eq!(run(&view, &page, "start_work")["effect"]["kind"], "none");
        assert_eq!(run(&view, &page, "rename_terminal")["effect"]["kind"], "none");

        page["missing_terminal"] = value!(true);
        let missing = step(&view, &page, value!({"kind": "present"}));
        assert_eq!(missing["main"]["terminal"], Value::Null);
        assert!(!page_available("workflow", false, true));
        assert!(!page_available("board", false, false));
        assert!(page_available("board", true, false));
        assert_eq!(run(&view, &page, "board")["effect"], value!({"kind": "navigate", "page": "board"}));
        assert!(!page_available("ticket", true, false));
        assert!(page_available("plan", false, false));
    }

    #[test]
    fn empty_workspace_names_where_to_open_a_terminal() {
        let mut page = context("terminals");
        page["terminals"] = value!([]);
        page["orchestrator"] = Value::Null;
        let split = run(&Value::Null, &page, "split");
        assert!(split["main"]["terminal"].is_null());
        let notice = "Billing has no terminals yet. Use Start Work to open one.";
        assert_eq!(split["main"]["notice"], notice);
        assert_eq!(split["side"]["notice"], notice);

        page["workspace_count"] = value!(1);
        let single_workspace = step(&split, &page, value!({"kind": "present"}));
        assert_eq!(single_workspace["main"]["notice"], "Build Mac has no terminals yet. Use Start Work to open one.");

        page["page"] = value!("plan");
        let plan = step(&single_workspace, &page, value!({"kind": "present"}));
        assert_eq!(plan["main"]["notice"], "");
    }

    #[test]
    fn main_pane_opens_the_first_running_terminal_or_the_first_listed() {
        let mut page = context("terminals");
        let terminals = page["terminals"].as_array_mut().unwrap();
        terminals.rotate_right(1);
        let selected = step(&Value::Null, &page, value!({"kind": "present"}));
        assert_eq!(selected["main"]["terminal"], "orch");

        for terminal in page["terminals"].as_array_mut().unwrap() {
            terminal["running"] = value!(false);
        }
        let selected = step(&Value::Null, &page, value!({"kind": "present"}));
        assert_eq!(selected["main"]["terminal"], "logs");
    }

    #[test]
    fn split_has_two_unique_leaves_and_focus_restores() {
        let page = context("terminals");
        let start = run(&Value::Null, &page, "split");
        assert_eq!(start["main"]["terminal"], "orch");
        assert!(start["side"]["terminal"].is_null());
        assert_eq!(start["focus"], "side");
        let next = step(&start, &page, value!({"kind": "open", "id": "logs"}));
        assert_eq!(next["side"]["terminal"], "logs");
        let same = run(&next, &page, "split");
        assert_eq!(same["layout"], next["layout"]);
        let focused = run(&same, &page, "focus_other");
        let restored = step(&focused, &page, value!({"kind": "present"}));
        assert_eq!(restored["layout"], focused["layout"]);
        assert_eq!(restored["focus"], "main");
        let duplicate = step(&restored, &page, value!({"kind": "open", "id": "logs"}));
        assert_eq!(duplicate["focus"], "side");
        assert_eq!(duplicate["main"]["terminal"], "orch");
        let closed = run(&duplicate, &page, "close_split");
        assert_eq!(closed["focus"], "main");
        assert!(closed["side"].is_null());
        assert_eq!(step(&closed, &page, value!({"kind": "focus", "slot": "side"}))["focus"], "main");
    }

    #[test]
    fn orchestrator_spans_pages_and_keeps_mode_when_discovery_is_pending() {
        let page = context("plan");
        let open = run(&Value::Null, &page, "orchestrator");
        let raw = run(&open, &page, "raw_terminal");
        assert_eq!(raw["side"]["mode"], "terminal");
        for page in ["ticket", "workflow", "needs", "terminals"] {
            let page = context(page);
            let mut undiscovered = page.clone();
            undiscovered["orchestrator"] = Value::Null;
            undiscovered["terminals"] = value!([]);
            let absent = step(&raw, &undiscovered, value!({"kind": "present"}));
            let restored = step(&absent, &page, value!({"kind": "present"}));
            assert_eq!(restored["side"]["terminal"], "orch");
            assert_eq!(restored["side"]["mode"], "terminal");
            if page["page"] == "terminals" { assert_eq!(restored["main"]["terminal"], "shell"); }
        }
        assert_eq!(run(&raw, &context("terminals"), "close_split")["main"]["terminal"], "orch");
    }

    #[test]
    fn shortcuts_respect_editors_and_terminals_and_reject_conflicts() {
        let page = context("ticket");
        let configured = step(&Value::Null, &page, value!({"kind": "bind", "command": "split", "key": " D "}));
        assert_eq!(configured["bindings"]["split"], "d");
        let conflict = step(&configured, &page, value!({"kind": "bind", "command": "commands", "key": "d"}));
        assert!(conflict["bindings"]["commands"].is_null());
        assert_ne!(conflict["problem"], "");
        let key = |terminal, editing, modal| value!({
            "kind": "key", "key": "D", "alt": true, "shift": true, "ctrl": false, "meta": false, "terminal": terminal, "editing": editing, "modal": modal,
        });
        for ignored in [key(true, false, false), key(false, true, false), key(false, false, true)] {
            assert_eq!(step(&configured, &page, ignored)["handled"], false);
        }
        let split = step(&configured, &page, key(false, false, false));
        assert_eq!(split["effect"]["kind"], "focus");
        assert_eq!(split["focus"], "side");
    }

    #[test]
    fn stale_panes_and_unknown_layout_versions_recover() {
        let page = context("terminals");
        let split = run(&Value::Null, &page, "split");
        let opened = step(&split, &page, value!({"kind": "open", "id": "logs"}));
        let mut changed = page.clone();
        changed["terminals"].as_array_mut().unwrap().pop();
        let repaired = step(&opened, &changed, value!({"kind": "present"}));
        assert!(repaired["side"]["terminal"].is_null());
        assert_eq!(repaired["focus"], "side");
        let mut unknown = repaired;
        unknown["layout"]["version"] = value!(2);
        let fallback = step(&unknown, &changed, value!({"kind": "present"}));
        assert!(fallback["side"].is_null());
        assert_eq!(fallback["focus"], "main");
    }

    #[test]
    fn plan_names_an_explicit_terminal_or_a_pane_id() {
        assert_eq!(orchestrator(r#"{"agents":[{"role":"orchestrator","terminal":"t","id":"pane:p"}]}"#).as_deref(), Some("t"));
        assert_eq!(orchestrator(r#"{"agents":[{"role":"agent","terminal":"wrong"},{"role":"orchestrator","id":"pane:p"}]}"#).as_deref(), Some("p"));
        assert_eq!(orchestrator(r#"{"agents":[{"role":"orchestrator","id":"native:123"}]}"#), None);
    }
}
