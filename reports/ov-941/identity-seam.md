# ov-941: CLI durable retry identity seam — phase 1

Disposition: source contract/options ready for scoped security review. No existing
CLI identity getter supports durable retry. A documented OpenSSH callback is a
bounded investigation candidate, not an approved identity source or implementation.
Durable CLI retry remains OPEN and unwaived. The accepted R-94 refusal stays intact.

## Authority and verified anchors

The assigned isolated branch is `ov-941-cli-identity-source`; initial HEAD is exactly
approved `d43e8f99cc00d81ec5f2ed4e606483e9352fe3a3`. This report is its direct child;
only `reports/ov-941` is allocated. No product source, accepted evidence or other
tree is changed. No implementation, candidate import, build, test, SSH/config
resolution, network, daemon, provider, UI, device, database, cache/target probe,
secret-store action, process signal or cleanup is part of this phase.

Read the absolute manager anchor:
`/Users/e-liang/Dev/overnight/.claude/agent/reports/ov-939/independent-review.md`.
Its actual SHA-256 is
`987d29d8396779d9df9f89ce2ebd351298777982ea3874c2253a775709fea2c2`, as required.
Its original blocker, SAME identity clarification and final refusal-only PASS
remain distinct. The final addendum approves d43e8f99's unsafe-send closure and
explicitly leaves ov-941 recovery open. It attributes the original gap inspection
to fixer `8b30fcf2` and the chosen refusal to R-94. `git show 8b30fcf2` cannot
resolve that object here; no independent source comparison to it is claimed.
The tracked `reports/ov-939-cli-fix/plan.md` and `handoff.md` identify manager answer
`d8b34fd7` and earlier addendum hash
`8512335e424ba28a7fc35c5cf5a64dbfbce06a149e3c105b9aea4869080b0f58` as the
refusal authorization. These corroborate the disposition, but are not the literal
R-94 decision record. That record and unavailable fixer object remain provenance
limits for the manager to reconcile, not grounds to invent authority or reopen
the accepted closure. No missing anchor has been silently declared verified.

## Observed source contract

All repository references below refer to the approved immutable base.

| Surface | What it actually supplies | Authority and limit |
| --- | --- | --- |
| `crates/cli/src/daemon_link.rs:30–82`, `Link` | Protocol Client, optional owned SSH Child, optional local peer PID; daemon version/capabilities and mutable client access | No authenticated host/account identity accessor. Version/capability metadata is not recipient proof. |
| `daemon_link.rs:85–124` | `Some(target)` calls `remote::connect(target, None)`; local dial splits a Unix stream and negotiates protocol; local connect can auto-start | Route selection is not durable identity. No silent substitution of device credentials. |
| `daemon_link.rs:127–162` | macOS LOCAL_PEERPID or Linux peer credential PID | Kernel provenance for the current socket/process only. PID reuse, restart, account/store replacement and other OS absence defeat durable binding. |
| `crates/cli/src/remote.rs:43–49, 324–366` | External `ssh`, inherited stderr, piped protocol stdin/stdout, owned kill-on-drop Child | Existing OpenSSH authenticates under its existing configuration; the CLI does not receive its negotiated host key or selected account/principal. Child ownership is lifetime management, not an identity assertion. |
| `remote.rs:77–145, 154–195, 254–301` | BatchMode, optional named key or ordinary agent/config auth, channel-specific multiplex route, ssh -G route classification | Alias, resolved destination, path, `%C`, local user and process IDs are routing/current-process context. They do not certify the original receipt namespace. No ssh -G was executed for this report. |
| `crates/client/src/ssh.rs:119–165, 179–184, 212–218, 224–316` | Separate native russh verifier records actual SHA256 host-key fingerprint; explicit Destination user/private key and policy; successful public-key authentication precedes Session return | Real in-process producer exists in a different transport. Its host-key provenance does not magically extend to external OpenSSH Link or prove unique database/account continuity. |

Native `Session.host_fingerprint` is reusable only for callers already using that
Session under its policy. Replacing CLI transport would also replace config,
authentication, multiplexing and prompting semantics. It is outside this seam and
is not recommended. No shared/native/Web closure is reopened.

A durable binding needs the original authenticated recipient AND remote account /
receipt principal context, channel and workspace, plus unchanged inner command
bytes. Host key alone identifies a key holder, not a unique database installation;
remote username alone is a label, not authenticated account continuity. Existing
actor roles must remain unchanged. Neither a caller-supplied principal nor the
cloneable host.get install UUID can supply this proof. A copied workflow snapshot
without the original receipt is the controlling wrong-recipient counterexample.

## Installed producer documentation and bounded callback option

Only public installed man sources were read, not owner SSH files or executable
behavior. `/usr/share/man/man5/ssh_config.5` has Mdocdate March 23 2026;
`/usr/share/man/man1/ssh.1` has Mdocdate December 22 2025. Their byte hashes are
in `source-hashes.txt`. These documents do not establish the version or behavior
of whichever PATH-selected `ssh` a future CLI process launches.

Exact documentation anchors in ssh_config.5:

- Lines 1367–1386: LocalCommand runs locally after successful server connection,
  synchronously through the user's shell, without access to the spawning SSH
  session; ignored unless PermitLocalCommand is enabled (1535 onward).
- Lines 2336–2449: runtime token expansion has no shell escaping/filtering.
  `%f` is the server host-key fingerprint, `%K` the base64 host key, `%t` its
  type, `%r` remote username; LocalCommand accepts all tokens. `%i` and `%u`
  identify the local user; `%h/%n/%k` name destinations/aliases. `%C` hashes
  `%l%h%p%r%j` in this installed manual, rather than authenticating a host.
  This also counters remote.rs's older `%C` comment omitting `%j`; neither
  spelling is an authority source.
- Lines 661–742: ControlMaster can reuse a prior connection and ControlPersist
  can keep it alive after the initial client closes. A new Child or channel is
  not proof of a fresh host-key exchange or renewed authentication/revocation.
- Lines 1342–1366: KnownHostsCommand supplies candidate known-host keys and can
  run for algorithm ordering, hostname and address lookup. It is a trust-input
  callback, not evidence that its presented key was accepted/authenticated.

Smallest candidate: a CLI-local, invocation-bound observation of OpenSSH's
documented LocalCommand host-key/account tokens, carried separately from the
protocol byte stream and exposed through RemoteLink/Link only after verification.
This is a proposed producer contract, NOT an existing callback implementation,
getter, file format, capability, field allocation or permission to enable it.
No debug/stderr parsing, known_hosts scraping, ssh-G inference, fake field or
generic SSH/authentication platform is acceptable.

The installed docs alone do NOT prove:

1. Whether the callback runs on a multiplexed slave; whether its key/account
   tokens are populated from the authenticated master or merely current config;
   whether it can identify the specific protocol channel. A fresh handshake is
   not promised. Disabling/changing multiplex or authentication to make a test
   pass is a compatibility change requiring separate disposition, not a fix here.
2. Exact ordering relative to authenticated account acceptance, remote command
   launch and protocol hello, callback failure behavior, or callback completion
   before application send. The callback cannot access the session itself.
3. Stable certificate/CA/host-key rotation semantics, canonical account identity,
   authorized-key restrictions/forced commands, selected login credential or
   resulting daemon receipt principal. `%r` documents username, not a signed
   durable account identifier or Unix UID/receipt namespace.
4. Authenticity of an arbitrary callback message. A future private receiver
   needs a bounded invocation lifetime and association to the actual SSH Child
   and channel, no stale/duplicate/cross-invocation input, bounded framing and
   timeouts, and rejection of injected/partial records. A nonce/path alone does
   not protect against a malicious same-user process. Local-command shell/token
   injection and executable/helper substitution must be reasoned explicitly.
5. Coexistence with an existing configured LocalCommand/PermitLocalCommand,
   Match/ProxyJump/ProxyCommand/config behavior and platform/version support.
   Replacing a user's callback or enabling arbitrary configured execution is
   not a transparent extension. Child drop is not callback/master containment.

Thus this candidate is smaller in surface than replacing the transport, but is
not yet supportable as durable authority. Preserve all existing SSH policy,
known-host/config behavior, CLI actor roles and native/Web paths. If a bounded
producer observation cannot prove all required dimensions without altering those
contracts, report the genuine gap; do not ship a host-fingerprint-only binding.
Local Unix retry has no restart-safe identity producer in this allocation and
remains Unknown/unsupported. Host/account key cloning, account deletion/recreation,
same-key multiple stores, certificate ambiguity, unsupported versions and
unproved multiplex inheritance remain explicit unsupported cases unless a later
review proves continuity or the owner literally waives the capability limit.

## Exact future boundaries — proposal only

Before code, obtain a scoped security disposition and literal allocation for:

- `crates/cli/src/remote.rs`: only the reasoned retry connection's producer
  observation and Child/channel lifetime association, preserving ordinary SSH
  argv/config behavior. A private helper or test file must be named in that
  allocation; this report allocates none.
- `crates/cli/src/daemon_link.rs`: carry only proved identity/context or explicit
  Unknown for that connection. No invented accessor over absent state, no local
  auto-start in unsupported retry and no global authentication conversion.
- `crates/cli/src/workflow.rs`: CLI-local outer retained binding and pre-send
  comparison only after producer proof. Retain exact inner bytes and accepted
  history; reject old unbound artifacts rather than migrate/reprepare them.
- A specifically allocated private producer test/report boundary, with no owner
  config/keys, live runner or secret-store writes. Product schema/wire tags,
  shared PreparedWorkflowAction, protocol, daemon, Store, client SSH/auth,
  native/Web adapters and global SSH defaults remain outside this proposal.

No future source/test work is authorized by listing these paths. If account or
local continuity requires another producer, it needs a new bounded allocation;
this task does not allocate wire/schema/capability or security-default changes.

## Finite later proof plan and decisive controls

All execution below is deferred. A mock identity object is insufficient evidence
for the producer seam. Review the exact producer/executable/version and fixture
resource boundaries before running a finite private SSH fixture with disposable
keys/config/accounts/state. No owner SSH configuration, key, known-host, live
daemon or provider is borrowed. Use the existing CLI auth/actor route.

1. Observe one real fresh private OpenSSH handshake and its LocalCommand producer
   through the actual connection-to-Link path, associating host key, account,
   protocol channel and receipt principal. Test callback ordering, shell metachar
   inputs, missing/late/partial/duplicate/spoofed/cross-child records, helper or
   executable substitution and bounded termination. No application mutation
   before complete proof. A protocol hello alone must not create authority.
2. Exercise fresh and reused multiplex connections separately, including master
   persistence after Child exit and identity/config change while a master lives.
   Either prove inherited host/account/principal/channel semantics or refuse that
   case. Do not count a new child as fresh authentication. Cover existing callback
   coexistence, proxy/jump routing, rejected/unknown host keys, certificate versus
   raw keys and unsupported producer behavior. No trust bypass flags.
3. Persist bound original preparation before first mutation using unchanged inner
   bytes and existing file safety. Discard only the reply after private fixture
   acceptance; restart the client/recipient with preserved identity and receipt
   store; explicit retry returns the same receipt with one accepted mutation.
   Show byte equality, account/principal namespace and receipt equality rather
   than only an exit code or stale-state refusal.
4. Change/omit runner/default, retarget alias, change account, key, channel or
   workspace, clone install ID and matching workflow snapshot without receipt,
   replace store under same key/account, and try old/unbound/malformed files.
   Each unproved/mismatched case has zero mutation sends and zero automatic
   reprepare. Unknown local retry must retain no-connect/no-auto-start behavior.
   Drift discovered by connecting may require a handshake, but never mutation;
   statically unsupported paths must refuse before connection. Do not relabel
   all mismatch checks as no-connect checks.
5. Three finite red/restored controls: bypass recipient/account comparison and
   make the matching-snapshot wrong-recipient test fail; accept stale/spoofed
   producer input and make its no-send test fail; reserialize/reprepare retained
   bytes and make exact-byte/same-receipt assertions fail. Each red must actually
   compile and fail the intended assertion, preserve raw evidence/source hashes,
   restore exact bytes, then pass affected checks. Add a multiplex-specific red
   only if multiplex support is claimed. Setup failures are not intended reds.
6. Independent scoped security review reconciles producer trust, spoof/lifetime
   semantics, actual sender counts, duplicate receipt, controls, exact diff and
   behavior outside retry. Reuse prior accepted source/evidence. Do not rerun
   accepted native/Web/shared suites or infer whole-app/release completion.

## Retained capability limits and next disposition

Acceptance 1 has a source-backed contract and conditional option; exact producer
and account/namespace proof is still absent. Acceptance 2 is not implemented or
proven. Acceptance 3 has a finite plan only; no producer test or independent
security review has run in this phase. R-94 closes unsafe unbound send, not
durable recovery. Historical accepted bytes/history and other port closures stand.

Manager next action: reconcile unavailable 8b30fcf2/literal R-94 provenance and
route this exact signed report to scoped security review. That review must choose
whether the conditional callback investigation has a finite acceptable producer
contract and literal file/resource allocation, or record the genuine remaining
gap. No owner questionnaire, broad architecture restart, implementation or runtime
probe is initiated here. Stop after the normal signed phase-1 commit.
