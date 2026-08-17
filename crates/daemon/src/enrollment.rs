//! Which devices may log in to this runner: `client.list`, `client.enroll`,
//! `client.revoke`.
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
//! Every call here runs the file work on the blocking pool. `fence::write`
//! takes an advisory lock, `fsync`s twice and can sleep in a bounded retry, and
//! a runtime worker is not the thread to do that on.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{
    ClientEnroll, ClientEnrollResult, ClientList, ClientRevoke, Scope,
};

use crate::fence::{self, Entry, FenceError, Rejected};
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
        let entries = read(&path)?;
        // Either identity already being present is "already enrolled".
        //
        // The FINGERPRINT, whatever shape either line has, because a key already
        // in the file can already log in — including as a foreign line somebody
        // added by hand, where sshd matches their unrestricted line first and
        // ours would be dead weight granting nothing while the device got a
        // shell. **One key, one line, for the same reason:** sshd takes the
        // first line whose key matches, so the same key written both plain and
        // restricted would make "does this device get a shell" a question about
        // line order in a text file. A Mac's two keys are two different keys.
        //
        // The CLIENT ID only within the same shape, because two lines naming one
        // device in one shape leave the daemon unable to say which of them a
        // session arrived on and every audit answer after that is a guess —
        // while two lines naming one device in DIFFERENT shapes is exactly what
        // a Mac is, and refusing that (as an earlier version of this check did)
        // is refusing the second key it needs.
        if let Some(existing) = entries.iter().find(|e| {
            (!e.fingerprint.is_empty() && e.fingerprint == mine.fingerprint)
                || (!e.client_id.is_empty()
                    && e.client_id == mine.client_id
                    && e.shell_access == mine.shell_access)
        }) {
            // The grant it HAS, not the one that was asked for, and nothing is
            // written. Answering with the requested scope would tell a person
            // their phone is read-only while the file still says control, and
            // rewriting the line to match would let a second ceremony silently
            // widen an existing device's access.
            return Ok(ClientEnrollResult {
                client: Some(wire::enrolled_client(existing, 0)),
                already_enrolled: true,
            });
        }

        let (mut ours, foreign) = sorted(&entries);
        ours.push(line);
        fence::write(&path, fence::AUTHORIZED_KEYS, &ours, &foreign, fence::Placement::Last)?;
        Ok(ClientEnrollResult {
            // The only moment this runner can honestly stamp a time: the file
            // records none, so every later read of this entry reports 0.
            client: Some(wire::enrolled_client(&mine, now)),
            already_enrolled: false,
        })
    })
    .await
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
        let entries = read(&path)?;
        // NOT_FOUND rather than a cheerful success. "Revoked" from a runner that
        // revoked nothing is the one answer a person must never be given about
        // a device they are trying to cut off.
        if !entries.iter().any(|e| e.client_id == client_id) {
            return Err(DomainError::NotFound.into());
        }
        let remaining: Vec<Entry> =
            entries.into_iter().filter(|e| e.client_id != client_id).collect();
        let (ours, foreign) = sorted(&remaining);
        fence::write(&path, fence::AUTHORIZED_KEYS, &ours, &foreign, fence::Placement::Last)?;
        // Read back rather than answering with what was just computed, the same
        // way a settings write does: what the file now says is the only claim
        // worth making about who may log in.
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

/// This runner's fence, with each line attributed to the account whose file it
/// was read from.
///
/// The attribution is done HERE because `fence::parse` cannot do it: nothing in
/// an `authorized_keys` line names the account it grants — the file's location
/// is what does — so the caller that opened the file is the only one that knows.
///
/// Read-modify-write, and the read is not inside the writer's lock: two
/// enrollments landing in the same instant can each rebuild the block from a
/// snapshot taken before the other's write, and the loser's key is lost. Bounded
/// by `client.enroll` being a `host_admin` ceremony a person drives, and the
/// pre-write backup is what recovers it. Closing it properly means the writer
/// growing a read-modify-write shape, which is a change to a routine whose
/// failure mode is losing SSH access and not one to make in passing.
fn read(path: &std::path::Path) -> std::result::Result<Vec<Entry>, FenceError> {
    let account = local_account();
    Ok(fence::read(path, fence::AUTHORIZED_KEYS)?
        .into_iter()
        .map(|entry| Entry { account: account.clone(), ..entry })
        .collect())
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
        ] {
            let DomainError::InvalidArgument { what } = refused(rejected) else {
                panic!("{rejected:?} did not map to an invalid argument");
            };
            assert!(
                ["public_key", "public_key algorithm", "client_id", "scope"].contains(&what),
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
        let line = fence::render(key, "iPhone", "c1", Scope::Control, fence::Grant::FarCooler)
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
        let line = fence::render(key, "MacBook Air", "mac-1", Scope::HostAdmin, fence::Grant::Shell)
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
