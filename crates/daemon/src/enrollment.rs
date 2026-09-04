//! Which devices may log in to this runner, and how they reach it:
//! `client.list`, `client.enroll`, `client.revoke`, `client.set_node_key`.
//!
//! The policy layer above `fence`, which owns the bytes. Free functions taking
//! the service rather than methods on it, the shape `review_ops` already uses:
//! the dispatch table stays a table, and this stays testable without a wire
//! frame around it.
//!
//! **The file is the authority.** Nothing here caches what is enrolled, and
//! nothing writes an enrollment anywhere else. A record in a database that
//! disagreed with `authorized_keys` would be a screen telling somebody their
//! phone has access when sshd says otherwise, and the file is the one of the
//! two that decides.
//!
//! Every call here runs the file work on the blocking pool. `fence::update`
//! takes an advisory lock, `fsync`s twice and can sleep in a bounded retry, and
//! a runtime worker is not the thread to do that on. It also holds that lock
//! across the decision it hands back here, which is what makes two enrollments
//! landing together safe — see `fence::update` — and one more reason none of this
//! belongs on a runtime worker.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{
    ClientEnroll, ClientEnrollResult, ClientList, ClientRevoke, ClientSetNodeKey,
    ClientSetNodeKeyResult, Scope,
};
use farcooler_transport::Peer;

use farcooler_fence::{self as fence, Entry, FenceError, Rejected};
use crate::service::Service;
use crate::wire;

/// Every line in this runner's fence, ours and otherwise.
pub async fn list(svc: &Service) -> Result<ClientList> {
    let path = svc.authorized_keys().to_path_buf();
    blocking(move || Ok(listing(&read(&path)?))).await
}

/// Add a device's key, or report the grant it already has.
///
/// One call writes ONE line. A Mac calls twice — once for Key A, the restricted
/// line the app and the CLI use, and once for Key B, the plain line Zed, git and
/// Terminal use — with the same client id both times, which is what ties them
/// together for `list` and `revoke`. See `fence::Grant`.
///
/// **Those two calls may land in either order, or in the same instant.** The
/// whole read-modify-write happens inside `fence::update`'s lock hold, so neither
/// can rebuild the block from a snapshot that predates the other. It used to be
/// the caller's job not to overlap them — a rule stated in a comment here and
/// obeyed in a comment in `apps/macos`, which is not a rule that is enforced.
pub async fn enroll(svc: &Service, request: &ClientEnroll) -> Result<ClientEnrollResult> {
    let path = svc.authorized_keys().to_path_buf();
    // Which SHAPE, and nothing else about the bytes: `fence::render` builds both
    // from the same decoded key material, so this chooses between two lines this
    // daemon writes rather than between writing and being written to.
    let grant = if request.shell_access { fence::Grant::Shell } else { fence::Grant::FarCooler };
    // Rendered before the file is opened, so a request that could never produce
    // a line does not create a `.ssh` directory or a backup on its way to being
    // refused.
    let line = fence::render(
        &request.public_key,
        &request.label,
        &request.client_id,
        Scope::try_from(request.scope).unwrap_or(Scope::Unspecified),
        grant,
        // Task 9's enrollment ceremony carries a node key; nothing does yet.
        None,
    )
    .map_err(refused)?;
    // Read the line just rendered with the parser that will read it back out of
    // the file, rather than parsing the key a second way here. Two readers of
    // one line is two answers about what is enrolled.
    let mut mine = read_back(&line).ok_or_else(|| {
        tracing::error!("a rendered line did not parse back as an entry");
        DomainError::OperationFailed
    })?;
    mine.account = local_account();

    let now = now_millis();
    blocking(move || {
        // Read and write under ONE lock hold. Two enrollments landing in the same
        // instant used to each rebuild the block from a snapshot taken before the
        // other's write, and the loser's key was silently gone — a device
        // Settings says is enrolled and that cannot connect. A Mac is two
        // enrollments of one client id, so the window was reachable from the
        // product's ordinary path, not only in theory. See `fence::update`.
        fence::update(&path, fence::AUTHORIZED_KEYS, fence::Placement::Last, |entries| {
            let entries = attributed(entries);
            // Either identity already being present is "already enrolled".
            //
            // The FINGERPRINT, whatever shape either line has, because a key
            // already in the file can already log in — including as a foreign
            // line somebody added by hand, where sshd matches their unrestricted
            // line first and ours would be dead weight granting nothing while the
            // device got a shell. **One key, one line, for the same reason:** sshd
            // takes the first line whose key matches, so the same key written both
            // plain and restricted would make "does this device get a shell" a
            // question about line order in a text file. A Mac's two keys are two
            // different keys.
            //
            // The CLIENT ID only within the same shape, because two lines naming
            // one device in one shape leave the daemon unable to say which of them
            // a session arrived on and every audit answer after that is a guess —
            // while two lines naming one device in DIFFERENT shapes is exactly
            // what a Mac is, and refusing that (as an earlier version of this
            // check did) is refusing the second key it needs.
            //
            // Under the lock now, so a device that raced past this check a
            // microsecond ago is a device this check can see.
            if let Some(existing) = entries.iter().find(|e| {
                (!e.fingerprint.is_empty() && e.fingerprint == mine.fingerprint)
                    || (!e.client_id.is_empty()
                        && e.client_id == mine.client_id
                        && e.shell_access == mine.shell_access)
            }) {
                // The grant it HAS, not the one that was asked for, and nothing is
                // written — `Change::Leave` rather than a rewrite of what was just
                // read, which would take a fresh backup over the one copy of the
                // file from before the last real change. Answering with the
                // requested scope would tell a person their phone is read-only
                // while the file still says control, and rewriting the line to
                // match would let a second ceremony silently widen an existing
                // device's access.
                return Ok((
                    fence::Change::Leave,
                    ClientEnrollResult {
                        client: Some(wire::enrolled_client(existing, 0)),
                        already_enrolled: true,
                    },
                ));
            }

            let (mut ours, foreign) = sorted(&entries);
            ours.push(line);
            Ok((
                fence::Change::Write { entries: ours, foreign },
                ClientEnrollResult {
                    // The only moment this runner can honestly stamp a time: the
                    // file records none, so every later read of this entry
                    // reports 0.
                    client: Some(wire::enrolled_client(&mine, now)),
                    already_enrolled: false,
                },
            ))
        })
    })
    .await
}

/// Admit a device's node key to the tunnel, on the line it already holds.
///
/// **The whole safety argument for skipping the ceremony.** The line written is
/// the one this runner's own `authorized_keys` gave the CONNECTION — never the
/// device the request names, which is ignored and kept only so a log shows what
/// the caller believed. A device therefore cannot register a key for another
/// device. And registering one for itself grants nothing: it adds a route to
/// access the caller is demonstrably already holding, because it is using that
/// access to make this call. That is why this sits at `read` rather than
/// `host_admin` — see `rpc::required_scope`.
///
/// **What it is for.** A fleet enrolled before the tunnel existed has lines
/// with no node key at all, which `allowlist::from_entries` reads as a runner
/// that admits nobody — the only safe reading, since tailcat treats an empty
/// allowlist as "admit everyone". Without this call every one of those devices
/// would have to be re-enrolled by hand.
///
/// **Adding is not revoking, and the two use different mechanisms on purpose.**
/// This admits the key to the LIVE server through `allow_add`, which mutates
/// the running server's allowed set. Withdrawing a key cannot work that way —
/// tailcat copies the allowlist at `Start` and consults it only when a client
/// first registers, so a revoked device that is already peered would keep its
/// route — which is why revocation rebuilds the server and drops every live
/// tunnel with it. Adding must not pay that price: the caller is holding a
/// session on this runner right now, and every other device is holding one too.
///
/// Nothing here touches `allowlist::tunnel_plan` or `start_tunnel`. Those
/// decide whether a booting runner may serve at all, and their refusal to serve
/// an empty allowlist is the guard that keeps sshd from being opened to
/// everyone. This function only ever makes an allowlist longer.
pub async fn set_node_key(
    svc: &Service,
    peer: &Peer,
    request: &ClientSetNodeKey,
) -> Result<ClientSetNodeKeyResult> {
    // A caller that named no device has no line here. That is every local
    // socket client — the owner's own Mac app talking to its own daemon — and
    // it is not an error worth guessing around: there is nothing to write onto,
    // and picking a line for such a caller is precisely the thing this call
    // must never do. Named as `client_id` because that is the field of the
    // request a caller would think supplied it, and the answer is that nothing
    // in the request ever can.
    let Some(client_id) = peer.client_id.as_deref() else {
        return Err(DomainError::InvalidArgument { what: "client_id" });
    };
    let client_id = client_id.to_string();
    let node_key = request.node_key.clone();
    let path = svc.authorized_keys().to_path_buf();

    // Read and write under ONE lock hold, the same as `enroll` and `revoke`:
    // this rebuilds the whole block from what it read, so a snapshot taken
    // before a concurrent enrollment would put the file back without that
    // device's key. See `fence::update`.
    let rewriting = node_key.clone();
    blocking(move || {
        fence::update(&path, fence::AUTHORIZED_KEYS, fence::Placement::Last, |entries| {
            Ok((rerender_with_node_key(entries, &client_id, &rewriting)?, ()))
        })
    })
    .await?;

    // The allowlist is a projection of that file, so it is now one entry
    // longer. `allow_add` and `conn_blob` are synchronous and hold the Go
    // side's package-wide mutex — `serve` on the same runner can hold it for
    // 30-45 seconds — so neither may run on a runtime worker, and this RPC
    // must not be the thing that waits behind one inline.
    let admitting = node_key;
    let conn_blob = match tokio::task::spawn_blocking(move || {
        // Best effort, and the line is already written either way. A runner
        // serving no tunnel yet answers `ENOTCONN` here: the first device to
        // register on a migrating runner is exactly that case, and what it has
        // done is write the line that lets the next boot start a server at all.
        if let Err(error) = farcooler_tailcat::allow_add(&admitting) {
            tracing::warn!(
                code = error.code(),
                "the running tunnel did not admit a newly registered node key"
            );
        }
        farcooler_tailcat::conn_blob()
    })
    .await
    {
        Ok(Ok(blob)) => blob,
        Ok(Err(error)) => {
            tracing::debug!(code = error.code(), "this runner has no tunnel token to hand back");
            String::new()
        }
        Err(error) => {
            tracing::warn!(error = %error, "the tunnel task did not finish");
            String::new()
        }
    };

    tracing::info!(client = %peer.client_id.as_deref().unwrap_or("-"), "registered a node key");
    Ok(ClientSetNodeKeyResult { conn_blob })
}

/// The block as it should read once the caller's own line carries a node key.
///
/// Exactly one line changes. Every other line in the block goes back byte for
/// byte, including this device's OWN plain line — a Mac's Key B has no forced
/// command to hold the flag — every other device's lines, and every foreign
/// line, which is somebody's hand-added key that this daemon must never rewrite.
///
/// A caller with no Key A line is `NotFound` rather than a cheerful success. It
/// cannot happen over SSH, where the client id came from the very line being
/// looked for, but answering "registered" to a caller whose key was revoked a
/// microsecond ago would be telling a device it has a route it does not have.
fn rerender_with_node_key(
    entries: &[Entry],
    client_id: &str,
    node_key: &str,
) -> std::result::Result<fence::Change, Refusal> {
    // `!shell_access`, because a plain line carries no forced command; a
    // non-empty client id, because a foreign line has none and must never be
    // matched by one.
    let at = entries
        .iter()
        .position(|e| !e.shell_access && !e.client_id.is_empty() && e.client_id == client_id)
        .ok_or(Refusal(DomainError::NotFound))?;
    // `Rejected::NodeKey` is carried out to the caller as a refusal rather than
    // swallowed: a device told its key was registered when it was not would
    // wait forever for a tunnel that never admits it.
    let rewritten = fence::with_node_key(&entries[at], node_key).map_err(|r| Refusal(refused(r)))?;

    let mut ours: Vec<String> = Vec::new();
    let mut foreign: Vec<String> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let line = if index == at { rewritten.clone() } else { entry.line.clone() };
        if entry.client_id.is_empty() {
            foreign.push(line);
        } else {
            ours.push(line);
        }
    }
    Ok(fence::Change::Write { entries: ours, foreign })
}

/// Remove a device's lines, close the sessions it was holding, and answer with
/// what is left.
///
/// **Every line under that client id**, which for a Mac is both of its keys —
/// the restricted one and the plain one Zed, git and Terminal use — in one write.
/// That is the whole reason a plain line carries the id in its comment: two
/// calls could half fail and leave a device that is gone from the app and still
/// holds a shell. It is also why the removal copy says the Mac loses SSH access
/// to this runner entirely rather than only its access to Far Cooler.
///
/// The plain line's removal contains NOTHING that is already running. Nothing
/// arrives on it that this daemon can see, so there is no session of ours to
/// close, and a shell that is already open stays open — and can put both lines
/// back. That is stated rather than hidden: revoke, then audit the runner.
///
/// **In that order, and the order is the point.** sshd reads `authorized_keys`
/// at authentication and never again, so deleting the line stops the next login
/// and does nothing at all to a session the device already holds. Answering
/// before those are closed would report a containment that had not happened,
/// to somebody who is going to act on the answer.
///
/// **What this contains:** every connection this daemon is serving that carries
/// the revoked client id — the device's control connection, and any relayed
/// stdio session piped into this daemon, since those arrive on this socket like
/// any other. When the daemon closes one, the relay's copy loop reaches EOF and
/// the `farcoolerd --stdio` process that sshd launched exits with it, which ends
/// the ssh session too.
///
/// **What it does not:** anything this daemon is not serving. A daemon in
/// another `FARCOOLER_HOME` has its own registry, and a multiplexed ssh master
/// is a client-side object on the device's own machine that no runner can
/// reach — it can keep opening channels for `ControlPersist` seconds, and each
/// new channel runs the forced command again with the same `--client`. That is
/// why `crates/cli/src/remote.rs` disables `ControlMaster` when an identity is
/// named: the containment a runner can offer is per connection, so the
/// connections must not be shared.
pub async fn revoke(svc: &Service, request: &ClientRevoke) -> Result<ClientList> {
    let path = svc.authorized_keys().to_path_buf();
    let client_id = request.client_id.clone();
    // A foreign line has no client id, so an empty one must not match all of
    // them and delete keys Far Cooler never wrote.
    if client_id.is_empty() {
        return Err(DomainError::InvalidArgument { what: "client_id" });
    }
    let closing = client_id.clone();
    let remaining = blocking(move || {
        // Read and write under one lock hold, so that a revocation cannot rebuild
        // the block from a snapshot taken before a concurrent enrollment landed
        // and put the enrolled key back. See `fence::update`.
        fence::update(&path, fence::AUTHORIZED_KEYS, fence::Placement::Last, |entries| {
            let entries = attributed(entries);
            // NOT_FOUND rather than a cheerful success. "Revoked" from a runner
            // that revoked nothing is the one answer a person must never be given
            // about a device they are trying to cut off. Decided in here, and
            // `Refusal` is what carries it out — the file is not touched.
            if !entries.iter().any(|e| e.client_id == client_id) {
                return Err(Refusal(DomainError::NotFound));
            }
            let remaining: Vec<Entry> =
                entries.into_iter().filter(|e| e.client_id != client_id).collect();
            let (ours, foreign) = sorted(&remaining);
            Ok((fence::Change::Write { entries: ours, foreign }, ()))
        })?;
        // Read back rather than answering with what was just computed, the same
        // way a settings write does: what the file now says is the only claim
        // worth making about who may log in.
        //
        // Outside the lock on purpose, and not the hazard the lock is there for:
        // this is a report, not a decision. Nothing is rebuilt from it, so an
        // enrollment that lands between the write and this read shows up in the
        // answer — which is the file being the authority, not a lost update.
        Ok(listing(&read(&path)?))
    })
    .await?;

    // Then the sessions, and only then the answer.
    //
    // After the write rather than before it, so a close that raced a failed
    // write could not have cut a device off while leaving its key in place —
    // the file is the authority, and it is made to say so first.
    //
    // Synchronous and complete when it returns: closing a session is a flag the
    // connection's next poll reads, not a request to something that might get
    // round to it.
    let closed = svc.sessions().close(&closing);
    tracing::info!(client = %closing, closed, "revoked a device and closed its live sessions");

    Ok(remaining)
}

/// This runner's fence, for the calls that only report it.
///
/// `list`, and `revoke`'s read-back — never the read half of a read-modify-write.
/// That one goes through `fence::update`, which does its reading inside the
/// writer's lock: reading here and writing afterwards is what used to let two
/// enrollments in the same instant each rebuild the block from a snapshot taken
/// before the other's write, losing the loser's key and with it a device's access.
fn read(path: &std::path::Path) -> std::result::Result<Vec<Entry>, FenceError> {
    Ok(attributed(&fence::read(path, fence::AUTHORIZED_KEYS)?))
}

/// Each line marked with the account whose file it was read from.
///
/// Done HERE because `fence` cannot do it: nothing in an `authorized_keys` line
/// names the account it grants — the file's location is what does — so the caller
/// that opened the file is the only one that knows. Which is also why the entries
/// `fence::update` hands a closure arrive without it, and every closure that
/// reports one puts it on first.
fn attributed(entries: &[Entry]) -> Vec<Entry> {
    let account = local_account();
    entries
        .iter()
        .map(|entry| Entry { account: account.clone(), ..entry.clone() })
        .collect()
}

fn listing(entries: &[Entry]) -> ClientList {
    ClientList { items: entries.iter().map(|e| wire::enrolled_client(e, 0)).collect() }
}

/// The local account this daemon runs as, which is whose file it writes.
///
/// From the environment rather than `getpwuid`, because the answer is only ever
/// shown beside a key in a list — an empty one costs a person nothing, and the
/// unsafe call to avoid it would be the only one in this module.
fn local_account() -> Option<String> {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .ok()
        .filter(|name| !name.is_empty())
}

/// Our lines and the ones we did not write, as `fence::write` wants them.
///
/// Foreign lines are carried through verbatim rather than dropped: dropping one
/// would delete a key somebody added by hand inside our block.
fn sorted(entries: &[Entry]) -> (Vec<String>, Vec<String>) {
    let (ours, foreign): (Vec<&Entry>, Vec<&Entry>) =
        entries.iter().partition(|e| !e.client_id.is_empty());
    (
        ours.into_iter().map(|e| e.line.clone()).collect(),
        foreign.into_iter().map(|e| e.line.clone()).collect(),
    )
}

/// One rendered line, as the parser reads it.
fn read_back(line: &str) -> Option<Entry> {
    let text = format!("{}\n{line}\n{}\n", fence::BEGIN, fence::END);
    fence::parse(&text).ok()?.into_iter().next()
}

/// Why a key was not enrolled, as a code a client already knows.
///
/// `Rejected` carries no payload by design, and this keeps it that way: the
/// `what` is a constant from this build, never a word from the request. The
/// parser's own message — built from bytes off the wire — went to the daemon
/// log inside `render` and stops there, because this string is rendered in
/// Settings.
fn refused(rejected: Rejected) -> DomainError {
    tracing::debug!(%rejected, "a key was refused enrollment");
    DomainError::InvalidArgument {
        what: match rejected {
            Rejected::MultiLine | Rejected::Unparseable => "public_key",
            Rejected::Algorithm => "public_key algorithm",
            Rejected::ClientId => "client_id",
            // For `ShellScope` the field that disagrees with `shell_access` is
            // `scope`, and naming it is what says which of the two to change.
            Rejected::Unscoped | Rejected::ShellScope => "scope",
            Rejected::NodeKey => "node_key",
        },
    }
}

/// A refusal on its way out of the blocking closure.
///
/// A newtype only so `?` can carry both a `FenceError` and a `DomainError`.
/// `DomainError` lives in `core`, which knows nothing about `authorized_keys`
/// and should not: an error type with a variant per file this daemon touches
/// would be a domain vocabulary defined by its plumbing.
struct Refusal(DomainError);

impl From<DomainError> for Refusal {
    fn from(error: DomainError) -> Self {
        Refusal(error)
    }
}

/// A fence failure, as something a screen can say.
///
/// Never the underlying `io::Error`, and never the parser's account of the
/// damage. "Permission denied (os error 13)" tells a person nothing about which
/// file or what to do; the one thing they need to know is that nothing was
/// changed, and every one of these leaves the file exactly as it was.
impl From<FenceError> for Refusal {
    fn from(error: FenceError) -> Self {
        match &error {
            FenceError::Damaged(why) => {
                tracing::warn!(%why, "the fence in authorized_keys is damaged");
            }
            e => tracing::warn!(error = %e, "the fence could not be read or written"),
        }
        Refusal(DomainError::OperationFailed)
    }
}

/// Run the file work off the runtime.
///
/// The closure answers in `Refusal` so `?` works on both error types inside it;
/// callers all speak `DomainError`.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> std::result::Result<T, Refusal> + Send + 'static,
) -> Result<T> {
    match tokio::task::spawn_blocking(work).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(Refusal(error))) => Err(error),
        Err(e) => {
            tracing::warn!(error = %e, "the fence task did not finish");
            Err(DomainError::OperationFailed)
        }
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(client_id: &str, line: &str) -> Entry {
        Entry {
            fingerprint: "SHA256:whatever".into(),
            client_id: client_id.into(),
            scope: Scope::Read,
            label: "farcooler-phone-aaaaaaaa".into(),
            node_key: String::new(),
            account: None,
            line: line.into(),
            shell_access: false,
        }
    }

    #[test]
    fn every_rejection_names_a_field_and_nothing_else() {
        // The whole point of `Rejected` having no payload: what reaches a
        // screen is chosen here, from constants, and can never be assembled out
        // of the bytes that were rejected.
        for rejected in [
            Rejected::MultiLine,
            Rejected::Algorithm,
            Rejected::Unparseable,
            Rejected::ClientId,
            Rejected::Unscoped,
            Rejected::ShellScope,
            Rejected::NodeKey,
        ] {
            let DomainError::InvalidArgument { what } = refused(rejected) else {
                panic!("{rejected:?} did not map to an invalid argument");
            };
            assert!(
                ["public_key", "public_key algorithm", "client_id", "scope", "node_key"]
                    .contains(&what),
                "{rejected:?} named {what}, which is not a field of the request"
            );
        }
    }

    #[test]
    fn a_rendered_line_reads_back_as_the_entry_it_describes() {
        // What `enroll` relies on to know the fingerprint it just wrote. If this
        // stopped holding, an enrollment would report a device it could not
        // find again.
        let key = "ssh-ed25519 \
                   AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA x";
        let line = fence::render(key, "iPhone", "c1", Scope::Control, fence::Grant::FarCooler, None)
            .expect("render");
        let entry = read_back(&line).expect("read back");
        assert_eq!(entry.client_id, "c1");
        assert_eq!(entry.scope, Scope::Control);
        assert!(!entry.shell_access);
        assert!(entry.fingerprint.starts_with("SHA256:"));
    }

    /// Key B is MANAGED, which is the whole reason it is enrolled here at all.
    ///
    /// A plain line looks exactly like a key somebody added by hand, and those
    /// are carried through verbatim and never revoked. If this stopped holding,
    /// `client.list` would report a Mac's shell key as a stranger's and
    /// `client.revoke` would leave it behind — which is the shell key getting
    /// added by hand and belonging to nobody, the thing this replaces.
    #[test]
    fn a_plain_line_of_ours_is_managed_rather_than_mistaken_for_a_strangers() {
        let key = "ssh-ed25519 \
                   AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA x";
        let line =
            fence::render(key, "MacBook Air", "mac-1", Scope::HostAdmin, fence::Grant::Shell, None)
                .expect("render");
        let entry = read_back(&line).expect("read back");
        assert_eq!(entry.client_id, "mac-1");
        assert!(entry.shell_access);
        let (ours, foreign) = sorted(&[entry]);
        assert_eq!(ours, vec![line]);
        assert!(foreign.is_empty(), "a plain line of ours was treated as a stranger's");
    }

    #[test]
    fn a_line_we_did_not_write_is_kept_apart_from_ours() {
        let ours = entry("c1", "restrict,command=\"x\" ssh-ed25519 AAAA farcooler-phone-aaaaaaaa");
        let theirs = entry("", "ssh-ed25519 BBBB me@laptop");
        let (mine, foreign) = sorted(&[ours.clone(), theirs.clone()]);
        assert_eq!(mine, vec![ours.line]);
        // Not an ordering detail: `fence::write` deletes everything in the block
        // that it is not handed, so a foreign line that failed to arrive in this
        // half is a key somebody added by hand and lost.
        assert_eq!(foreign, vec![theirs.line]);
    }
}
