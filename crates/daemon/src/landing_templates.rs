//! Landing templates on a host, and the one-time import of a charter's `## Landing` section (ov-959).
//!
//! The templates themselves are `farcooler_workflow::templates`; this is what pins them to a runner's
//! configured adapter and what turns a charter into a draft bundle. Nothing here publishes: a person
//! reads the draft and publishes it, as for any definition.
use farcooler_core::charter_landing::{self, Route};
use farcooler_protocol::workflow_config::{AdapterConfiguration, CodingCommand, CodingConfig};
pub use farcooler_workflow::templates::Template;
pub use farcooler_workflow::{Reviewer, StageKind};
use farcooler_workflow::templates::{self, Gate, Pins, Plan};
use farcooler_workflow::{Definition, OperationSpec};
use crate::workflow_runtime::CodingConfigExt;

/// The environment a gate runs in. The old landing job passed the whole of the runner's; a definition pins
/// its own: `PATH` and `HOME` unless the draft says otherwise, plus whatever `--gate-env` adds.
pub const GATE_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
/// How long a gate may run: the landing job's own allowance.
pub const GATE_TIMEOUT_MS: u64 = 3 * 3_600_000;
/// The command `Gate: none` becomes: a gate that always passes, written into the definition so the
/// absence of a check is something a person reads and publishes, not something a validator skips.
pub const NO_GATE: &str = "/usr/bin/true";

/// A runner's configured adapter, answering for the templates.
pub struct Host<'a>(pub &'a CodingConfig);
impl Pins for Host<'_> {
    fn operation(&self, version: &str, argument: &str) -> OperationSpec { self.0.operation(version, argument) }
    fn wait(&self, version: &str, argument: &str, every_ms: u64, patience_ms: u64) -> OperationSpec {
        self.0.wait_operation(version, argument, every_ms, patience_ms)
    }
    fn landing_resource(&self) -> String {
        self.0.landing_resource().unwrap_or_else(|| format!("ref:{}", self.0.landing_ref))
    }
    fn landing_target(&self) -> String {
        self.0.landing.as_ref().map_or_else(|| self.0.landing_ref.clone(), |l| l.target.clone())
    }
    fn ci_subject(&self) -> String { self.0.ci.repository.clone() }
}

/// A definition and the adapter settings it is pinned to, ready for `workspace set-workflow --file`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle { pub definition: Definition, pub adapter: AdapterConfiguration }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The charter's `## Landing` section didn't read.
    Charter(String),
    NoSection,
    /// A landing definition is already published here: the import is for the first one only.
    AlreadyImported(String),
    BaseDiffers { charter: String, configured: String },
    TooManyGates,
    NoSuchTemplate(String),
    /// The plan can't be built into a sound definition.
    Unsound,
    /// The runner has no adapter to pin to.
    NoAdapter,
    /// A `--gate-env` that isn't `NAME=value`.
    BadEnvironment(String),
    /// A person gate can only name someone who can decide.
    NotADecider(String),
    /// An agent gate needs the reviewer the runner is set up with.
    NoReviewer,
    BatchTooLarge,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Charter(why) => write!(f, "The charter's Landing section doesn't read: {why}"),
            Self::NoSection => f.write_str("This charter has no Landing section to import. Add one, or draft a template with `workflow template`."),
            Self::AlreadyImported(source) => write!(f, "A landing definition is already published here ({source}). The charter imports once; change landing by publishing a new definition."),
            Self::BaseDiffers { charter, configured } => write!(f, "The charter lands on {charter}, but this runner is set up to land on {configured}. Change one of them and try again."),
            Self::TooManyGates => f.write_str("That is more gates than a runner can pin (8 in all)."),
            Self::NoSuchTemplate(name) => write!(f, "There's no template called {name}. The four are batched-push, ticket-push, ticket-pr and batched-pr."),
            Self::Unsound => f.write_str("Those choices don't make a sound definition. A push needs at least one command gate to stand behind it."),
            Self::NoAdapter => f.write_str("This workspace has no adapter settings yet, so there is nothing to pin a definition to."),
            Self::BadEnvironment(text) => write!(f, "`{text}` isn't a gate setting. Write it as NAME=value."),
            Self::NotADecider(who) => write!(f, "{who} can't decide a gate. Only the owner can, so name owner."),
            Self::NoReviewer => f.write_str("An agent gate needs a reviewer, and this runner isn't set up with one."),
            Self::BatchTooLarge => write!(f, "A batch holds at most {MAX_BATCH} tickets, so a failing one can always be found by splitting."),
        }
    }
}
impl std::error::Error for Error {}

/// A gate beyond the charter's commands. The charter can only say commands; an agent or a person is chosen here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Extra { Agent, Person(String) }

/// The most tickets a batch may hold: the split limit halves it down to single tickets.
pub const MAX_BATCH: u8 = 8;

/// The environment gates run in: `PATH` and the draft host's `HOME`, then each `NAME=value` given, which wins.
pub fn gate_environment(given: &[String]) -> Result<std::collections::BTreeMap<String, String>, Error> {
    let mut environment = std::collections::BTreeMap::from([("PATH".to_string(), GATE_PATH.to_string())]);
    if let Some(home) = std::env::var_os("HOME").and_then(|h| h.into_string().ok()).filter(|h| h.starts_with('/')) { environment.insert("HOME".into(), home); }
    for text in given {
        let (name, value) = text.split_once('=').filter(|(n, _)| !n.is_empty() && !n.contains(char::is_whitespace)).ok_or_else(|| Error::BadEnvironment(text.clone()))?;
        environment.insert(name.into(), value.into());
    }
    Ok(environment)
}

fn command(text: &str, environment: &std::collections::BTreeMap<String, String>) -> CodingCommand {
    CodingCommand { argv: vec!["/bin/sh".into(), "-c".into(), text.into()], environment: environment.clone(), timeout_ms: GATE_TIMEOUT_MS }
}

fn person(principal: &str) -> Result<Gate, Error> {
    if principal != "owner" { return Err(Error::NotADecider(principal.into())); }
    Ok(Gate::Person { principal: principal.into() })
}

/// What a charter's landing section says, as a draft bundle for `config`.
///
/// - The most careful route in the section wins for the whole workspace: ask over pull request over push.
///   That is what a ticket touching every rule's files would have got, so it is never less careful. The
///   note says which rules it folded.
/// - Each `Gate:` becomes one command gate, run with `sh -c` and a pinned environment. `Gate: none` is
///   a gate that always passes, written out.
/// - `Ask` adds a gate a person decides before the ticket lands.
/// - `published` is the source of the definition already published, if any: the import is once only.
pub fn import_charter(charter: &str, config: &CodingConfig, published: Option<&str>, extra: &[Extra], environment: &[String]) -> Result<(Bundle, Vec<String>), Error> {
    let rules = charter_landing::parse(charter).map_err(|e| Error::Charter(e.to_string()))?.ok_or(Error::NoSection)?;
    if let Some(source) = published.filter(|s| s.starts_with("landing/")) { return Err(Error::AlreadyImported(source.into())); }
    // The push goes to the landing target when the runner has one, so that is the branch the charter's must match.
    let configured = config.landing.as_ref().map_or(config.landing_ref.as_str(), |l| l.target.as_str());
    let configured = configured.strip_prefix("refs/heads/").unwrap_or(configured);
    let environment = gate_environment(environment)?;
    if rules.base != configured { return Err(Error::BaseDiffers { charter: rules.base.clone(), configured: configured.into() }); }
    let mut notes = Vec::new();
    let all = std::iter::once(rules.default_route).chain(rules.rules.iter().map(|(_, r)| *r));
    let careful = |r: &Route| match r { Route::Push => 0, Route::PullRequest => 1, Route::Ask => 2 };
    let route = all.max_by_key(careful).unwrap_or(Route::Push);
    let differs = rules.default_route != route || rules.rules.iter().any(|(_, r)| *r != route);
    if differs {
        notes.push(format!("Every ticket takes the {} route, the most careful the charter names. Path rules ({}) aren't a step yet.",
            match route { Route::Push => "push", Route::PullRequest => "pull request", Route::Ask => "ask" },
            rules.rules.iter().map(|(g, _)| format!("`{g}`")).collect::<Vec<_>>().join(", ")));
    } else if !rules.rules.is_empty() {
        notes.push("The charter's path rules all agree with its default route, so none is written.".into());
    }
    let mut config = config.clone();
    if !config.checks.is_empty() { notes.push(format!("The runner's {} configured check(s) aren't part of this definition: its gates are the charter's own.", config.checks.len())); }
    config.checks.clear();
    let template = match route { Route::PullRequest => Template::TicketPr, Route::Push | Route::Ask => Template::BatchedPush };
    let mut commands: Vec<String> = rules.gate.clone();
    if template.pushes() && commands.is_empty() { notes.push("`Gate: none` is written as a gate that always passes.".into()); commands.push(NO_GATE.into()); }
    if rules.base != "main" || rules.remote != "origin" { notes.push(format!("Lands on {} through the remote the runner is set up with; the charter's remote ({}) isn't a setting here.", rules.base, rules.remote)); }
    let mut gates = Vec::new();
    for text in &commands {
        if config.checks.len() >= 8 { return Err(Error::TooManyGates); }
        gates.push(Gate::Command(config.checks.len()));
        config.checks.push(if text == NO_GATE { CodingCommand { argv: vec![NO_GATE.into()], ..command("", &environment) } } else { command(text, &environment) });
    }
    if route == Route::Ask { gates.push(person("owner")?); }
    for e in extra {
        gates.push(match e {
            Extra::Agent => Gate::Agent { argument: config.reviewer.as_ref().ok_or(Error::NoReviewer)?.identity.clone() },
            Extra::Person(p) => person(p)?,
        });
    }
    if template == Template::TicketPr { notes.push("A pull request route can't be published yet: the steps that open and merge pull requests aren't built in.".into()); }
    let mut plan = Plan::new(template, gates);
    plan.ci = rules.ci;
    plan.green_base = rules.ci;
    let definition = templates::build(&plan, &Host(&config)).map_err(|_| Error::Unsound)?;
    Ok((Bundle { definition, adapter: AdapterConfiguration::Coding(config) }, notes))
}

/// One of the four templates on `config`, with `commands` as the whole set of its command gates (in order)
/// and `extra` as its agent and person gates. The draft replaces the runner's configured checks, so it
/// always lists every gate it should have; tickets already in flight keep the bundle they started with.
pub fn template(name: &str, config: &CodingConfig, commands: &[String], extra: &[Extra], batch: Option<u8>, environment: &[String]) -> Result<Bundle, Error> {
    let template = Template::parse(name).ok_or_else(|| Error::NoSuchTemplate(name.into()))?;
    if batch.is_some_and(|b| b > MAX_BATCH) { return Err(Error::BatchTooLarge); }
    let environment = gate_environment(environment)?;
    let mut config = config.clone();
    config.checks.clear();
    let mut gates = Vec::new();
    for text in commands {
        if config.checks.len() >= 8 { return Err(Error::TooManyGates); }
        gates.push(Gate::Command(config.checks.len()));
        config.checks.push(command(text, &environment));
    }
    for e in extra {
        gates.push(match e {
            Extra::Agent => Gate::Agent { argument: config.reviewer.as_ref().ok_or(Error::NoReviewer)?.identity.clone() },
            Extra::Person(p) => person(p)?,
        });
    }
    let mut plan = Plan::new(template, gates);
    if let Some(batch) = batch { plan.batch = batch; }
    let definition = templates::build(&plan, &Host(&config)).map_err(|_| Error::Unsound)?;
    Ok(Bundle { definition, adapter: AdapterConfiguration::Coding(config) })
}
