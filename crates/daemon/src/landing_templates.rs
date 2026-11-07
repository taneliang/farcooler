//! Landing templates on a host, and the one-time import of a charter's `## Landing` section (ov-959).
//!
//! The templates themselves are `farcooler_workflow::templates`; this is what pins them to a runner's
//! configured adapter and what turns a charter into a draft bundle. Nothing here publishes: a person
//! reads the draft and publishes it, as for any definition.
use farcooler_core::charter_landing::{self, Route};
use farcooler_protocol::workflow_config::{AdapterConfiguration, CodingCommand, CodingConfig};
use farcooler_workflow::templates::{self, Gate, Pins, Plan, Template};
use farcooler_workflow::{Definition, OperationSpec};
use crate::workflow_runtime::CodingConfigExt;

/// The environment a charter's gate runs in. The old landing job passed the whole of the runner's;
/// a definition pins its own, so the import names the few variables a gate needs.
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
        }
    }
}
impl std::error::Error for Error {}

/// A gate beyond the charter's commands. The charter can only say commands; an agent or a person is chosen here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Extra { Agent, Person(String) }

fn command(text: &str) -> CodingCommand {
    CodingCommand { argv: vec!["/bin/sh".into(), "-c".into(), text.into()],
        environment: [("PATH".to_string(), GATE_PATH.to_string())].into(), timeout_ms: GATE_TIMEOUT_MS }
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
pub fn import_charter(charter: &str, config: &CodingConfig, published: Option<&str>, extra: &[Extra]) -> Result<(Bundle, Vec<String>), Error> {
    let rules = charter_landing::parse(charter).map_err(|e| Error::Charter(e.to_string()))?.ok_or(Error::NoSection)?;
    if let Some(source) = published.filter(|s| s.starts_with("landing/")) { return Err(Error::AlreadyImported(source.into())); }
    let configured = config.landing_ref.strip_prefix("refs/heads/").unwrap_or(&config.landing_ref);
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
    let template = match route { Route::PullRequest => Template::TicketPr, Route::Push | Route::Ask => Template::BatchedPush };
    let mut commands: Vec<String> = rules.gate.clone();
    if template.pushes() && commands.is_empty() { notes.push("`Gate: none` is written as a gate that always passes.".into()); commands.push(NO_GATE.into()); }
    if rules.base != "main" || rules.remote != "origin" { notes.push(format!("Lands on {} through the remote the runner is set up with; the charter's remote ({}) isn't a setting here.", rules.base, rules.remote)); }
    let mut gates = Vec::new();
    for text in &commands {
        if config.checks.len() >= 8 { return Err(Error::TooManyGates); }
        gates.push(Gate::Command(config.checks.len()));
        config.checks.push(if text == NO_GATE { CodingCommand { argv: vec![NO_GATE.into()], ..command("") } } else { command(text) });
    }
    if route == Route::Ask { gates.push(Gate::Person { principal: "owner".into() }); }
    for e in extra {
        gates.push(match e {
            Extra::Agent => Gate::Agent { argument: config.reviewer.as_ref().map(|r| r.identity.clone()).unwrap_or_else(|| "reviewer".into()) },
            Extra::Person(p) => Gate::Person { principal: p.clone() },
        });
    }
    let mut plan = Plan::new(template, gates);
    plan.ci = rules.ci;
    plan.green_base = rules.ci;
    let definition = templates::build(&plan, &Host(&config)).map_err(|_| Error::Unsound)?;
    Ok((Bundle { definition, adapter: AdapterConfiguration::Coding(config) }, notes))
}

/// One of the four templates on `config`, with `commands` as its command gates (in order, after any
/// already configured) and `extra` as its agent and person gates.
pub fn template(name: &str, config: &CodingConfig, commands: &[String], extra: &[Extra], batch: Option<u8>) -> Result<Bundle, Error> {
    let template = Template::parse(name).ok_or_else(|| Error::NoSuchTemplate(name.into()))?;
    let mut config = config.clone();
    let mut gates = Vec::new();
    for text in commands {
        if config.checks.len() >= 8 { return Err(Error::TooManyGates); }
        gates.push(Gate::Command(config.checks.len()));
        config.checks.push(command(text));
    }
    for e in extra {
        gates.push(match e {
            Extra::Agent => Gate::Agent { argument: config.reviewer.as_ref().ok_or(Error::Unsound)?.identity.clone() },
            Extra::Person(p) => Gate::Person { principal: p.clone() },
        });
    }
    let mut plan = Plan::new(template, gates);
    if let Some(batch) = batch { plan.batch = batch; }
    let definition = templates::build(&plan, &Host(&config)).map_err(|_| Error::Unsound)?;
    Ok(Bundle { definition, adapter: AdapterConfiguration::Coding(config) })
}
