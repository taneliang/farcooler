# ov-937 stable startup handoff to ov-936

Source pin e1203d8c6bbeb4bf7eaecb51a08fd0ab2c6884e2; product bytes remain unchanged while validation evidence is added. This is a source contract, not permission to integrate or execute main. Read sibling ov-936/reports/ov-936/contract.md only; no cutover guard implemented here.

## Held API

`paths::selection() -> Result<&'static RuntimeSelection>` captures validated provenance/root. `RuntimeSelection::root() -> &Path` computes a DB filename via `.join("farcooler.db")` without ensure/mkdir. `ensure_dir()` and `socket_path()` do mkdir/chmod. `Service::open()` at service.rs:2097 delegates to `Service::open_selected(paths::selection()?)`. `Service::open_selected` at runtime_selection.rs:6 ensures the selected dir then calls `open_in`; it does not currently accept cutover status or an already opened Store.

`Service::open_in` at service.rs:2117 calls load_or_create_install_id, Store::open(root/farcooler.db), constructs install-specific tmux/inventory and refreshes it, then loads registry and persisted Sessions. `Store::open` at store.rs:57 migrates/normalizes via its existing startup path (including waits::sweep_unfitting at106/120). A check after Service::open_selected is therefore too late for a requirement to preflight before Store startup normalization. Scope for Store guards belongs to936.

## Normal daemon order, main.rs

1. main:37 obtains selection before kept_log (:47) and run (:54). kept_log may create a retained log. No malformed explicit root reaches it.
2. run dispatches version/stdio/stream/fanout modes before normal daemon setup. Normal path :169 selected.socket_path ensures root, :180 probes existing socket, :206 acquires daemon lock.
3. :220 offer_passkey_keys runs the existing bounded sshd version child; :225/:228 read existing stand-in/test switch values. :230 Service::open_selected performs Store/inventory/config/session setup.
4. :249 start_workflows starts configured workflow service. :256 recover_check_ins_after_exclusive_start writes recovery state; :264 backfill_pane_tags, :266 resume_agent_listeners, :269 refresh_claude_key_exports; :275 release sweep, :279 access migration, :282 chat migration; :296 workspace homes task.
5. :325 rendezvous config, :329 tunnel start task, :339 Watcher::new and watcher run, :376 socket bind. Notice/helper region remains933/938-owned.

For the normal-daemon **preflight before Store/child effects** requirement, the stable insertion point is after acquiring `_lock` and before offer_passkey_keys (:220), using the held selected.root without re-reading FARCOOLER_HOME. If936 requires even logs/directory/socket-probe effects withheld, main must instead preflight immediately after selection and before kept_log; that is a stronger ordering contract than simply guarding start_workflows. A fresh-incarnation/authority admission check must also precede :249; checking only before Watcher::new is too late.

## Other startup callers — counterexamples to a normal-run-only guard

- --stream: main.rs:483 stream_terminal calls Service::open at:488 then inventory refresh and stream. It never reaches the normal-daemon lock/guard site.
- Standalone stdio: main.rs:612 serve_stdio_session first tries the socket/relay path, then opens Service at:662 and starts Watcher at:680 if serving itself. It also bypasses the normal-daemon lock/guard site. Existing relay-only path does not create Service.
- All three Service startup routes share the cached selection and same explicit/default binding, but **a normal run guard alone cannot cover them**.936 must account for shared Store/Service admission and the relevant mode paths under explicit reserved-region handoff. No authority bypass is supplied by this delta.

The real-home read-only guard remains unchanged. Mandatory literal absolute private FARCOOLER_CONFIG and CLAUDE_CONFIG_DIR still stand; unconditional Registry watch uses ambient fallback without CLAUDE_CONFIG_DIR. log_join is terminal-driven; no-terminal/provider constraints remain. Parent owns reviewed composition/rebuild/native links/platform/cutover evidence. No actual main, app, provider or owner-startup call was made for this handoff.
