# ov-937 source contract (before executable checks)

Base: frozen fae3f30d0c3d1c475f9dc4463243f293fd88b007. Branch ov-937-explicit-runtime-root. This delta does not approve or integrate 933.

## API and callers

- `paths::RuntimeSelection::from_override(Channel, Option<OsString>) -> Result<RuntimeSelection>` computes only paths. Holds root and explicit/default provenance. Empty, relative, non-Unicode, control/NUL, dot/parent, redundant separator, root-only and overlong socket paths refuse. No filesystem operation or silent default fallback. Lexical normalization is checked; canonical physical path/ownership/no symlink/fresh0700 remain launcher preconditions, not adversarial isolation guarantees.
- `paths::selection() -> Result<&'static RuntimeSelection>` captures runtime input once per process, including failure. `runtime_dir`, `ensure_runtime_dir`, and dependent socket/database/install/worktree callers use that capture. `runtime_dir_for(channel)` uses the same selector for channel-specific queries.
- daemon `main` obtains selection before `kept_log`; `run` gets that held selection, socket setup uses it, then `Service::open_selected` uses it. stdio's existing `Service::open` delegates to the capture. CLI `main` validates through the identical capture before `run`; daemon_link's existing paths callers share it. No alternate entry point/identity.
- `daemon_log::log_dir_for(&RuntimeSelection)` uses explicit root/logs, retains installed platform log defaults; legacy `log_dir` delegates.
- `Service::open_selected` calls existing private `open_in`, keeps read_only_folders::load and its real HOME guard, then binds installed paths only for default provenance. Explicit branch keeps scratch_authorized_keys(root), root/claude/.claude.json, root/home, root/home/.claude; both normal branches resolve real current_exe sibling CLI. open_in's fixture CLI remains unchanged.
- `Service::log_roots` excludes ambient HOME/CODEX_HOME for explicit/private construction; default calls existing log_watch::roots with unchanged CLAUDE/CODEX precedence. Watcher::build consumes those roots. `Service::log_home` supplies worker follower creation and therefore its reset-on-join-failure fallback. No follower algorithm change.

## Effects and limits

Validation precedes retained log creation, mkdir/chmod, socket probe/lock, Store, and startup config watch effects. Existing successful-root ensure chmod behavior is unchanged. Service startup still inventories its install-specific tmux namespace and loads configured registry/read-only folder config; private executable fixtures must supply literal private FARCOOLER_CONFIG. No OS HOME/CODEX_HOME reassignment or auth/security bypass.

Global claude_registry::global in watch.sample remains unconditional and uses CLAUDE_CONFIG_DIR else ambient HOME. Future launch MUST supply nonempty absolute private CLAUDE_CONFIG_DIR; FARCOOLER_HOME alone is not confinement proof. log_join fallback remains terminal-driven, so no-terminal/provider bootstrap limits remain mandatory. Do not launch actual main/app/providers as regression tests.

## Planned private checks

Selector tests and normal explicit Service binding test; default binding computes names on a private service without opening installed paths. Existing pure roots/precedence tests and a new private direct-fence migration/revoke/access-ceiling/allowlist fixture, CLI/daemon compile and clippy, file-size check. Existing migration suites were excluded after source review found tunnel-route actions. The direct-fence fixture never enrolls or constructs a tunnel identity. Safe intended-red mutation restores old unconditional binding only (assert paths before reading any); selector mutation accepts malformed input only in pure tests. Restore bytes and record failures/green results. No hazardous old normal startup.

Heavy checks only after normal FIFO admission, original923 target holder/provenance checks, native Python command-local PATH, profiles000/jobs6/offline, load5<20, 10GiB reserve, 1800s foreground supervisor/cancel/reap. Failed historical Python sandbox setup remains unknown, not capability evidence. Parent still owes reviewed composition, rebuilt binaries/native links, full private launch contract, platform/cutover proof.

See `callers.md` for exact file:line API/caller/effect anchors. `compile.log/json` preserves the first E0425 edit failure; `compile-fixed.log/json` preserves its correction. `selected.log/json` is initial green evidence before the added account/effect/CLI tests. `red-binding` and `red-selector` logs/json record expected assertion failures; negative JSON records exact restoration. `restored.log/json` records pre-build refusal at occupied target, with holder diagnosis in `holder-diagnostic.json`. It is not a restored green claim.

## Validation harness correction

The initial harness wrongly exported TMPDIR=/private/tmp before scripts/heavy. heavy_lock.py:179 resolves its machine-wide lock using tempfile.gettempdir(), so this forked the lock namespace. The later occupied-target refusal exposed the error. Prior compile/test/mutation outputs are preserved historical execution evidence, **not credited as normal exclusive FIFO evidence or sandbox capability proof**. No foreign process was killed and no cache was cleaned. The corrected harness preserves native TMPDIR; socket fixtures use explicit canonical short TempDirs under /tmp. Orchestrator authorization is pending before any corrected pass. No automatic retry.
