//! Read the ticket format shared by scripts/heavy and lane-gate.py.
pub(super) use crate::proc_table::topology as snapshot;
use crate::proc_table::{ProcessState, process_state, stopped_or_gone};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) const NAME: &str = "lane-gate.lock.queue";
const REPORT_AFTER: Duration = Duration::from_secs(300);

pub(super) fn machine() -> PathBuf { Path::new("/tmp").join(NAME) }

pub(super) fn group_stopped(group: i32) -> bool {
    snapshot().is_some_and(|raw| crate::foreground::parse(&raw).in_group(group).all(stopped_or_gone))
}

/// Hold each discovered legacy latch alongside the canonical one until the
/// pause has been checked. The second read can discover more queues.
#[derive(Default)]
pub(super) struct LegacyLatches(BTreeMap<PathBuf, File>);

impl LegacyLatches {
    pub(super) fn ticket_groups(&mut self, queue: &Path, snapshot: impl FnOnce() -> Option<String>,
        groups: impl Iterator<Item = i32>) -> io::Result<HashSet<i32>> {
        let raw = snapshot().ok_or_else(|| io::Error::other("process table unavailable"))?;
        let table = crate::foreground::parse(&raw);
        for old in legacy_queues(queue, groups.flat_map(|group| table.in_group(group)))? {
            if let std::collections::btree_map::Entry::Vacant(slot) = self.0.entry(old) {
                let latch = try_enter(slot.key())?;
                slot.insert(latch);
            }
        }
        let mut tickets = live_tickets(queue)?;
        for old in self.0.keys() {
            tickets.extend(live_tickets(old)?);
        }
        if tickets.is_empty() { return Ok(HashSet::new()); }
        crate::process_reads::DETAILS.read(move || {
            tickets.into_iter().map(|ticket| unsafe { libc::getpgid(ticket.pid) }).collect()
        }).ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "heavy-job ticket process groups unavailable"))
    }
}

pub(super) fn legacy_queues(queue: &Path, pids: impl IntoIterator<Item = i32>) -> io::Result<BTreeSet<PathBuf>> {
    let canonical = queue.canonicalize()?;
    let mut roots = vec![PathBuf::from("/var/tmp"), PathBuf::from("/usr/tmp")];
    for pid in pids {
        roots.extend(crate::proc_table::python_temp_roots(pid)
            .ok_or_else(|| io::Error::other("could not inspect legacy heavy-job queue owner"))?);
    }
    Ok(roots.into_iter().filter_map(|root| root.join(NAME).canonicalize().ok())
        .filter(|path| path != &canonical && path.is_dir())
        // Fixtures have private machine and legacy queues under the same test root.
        .filter(|path| !cfg!(test) || canonical.parent().is_some_and(|root| path.starts_with(root)))
        .collect())
}

/// Hold the same admission latch the Python queue takes before publishing a
/// ticket. Keep it until the kernel confirms the stop, so a process cannot
/// publish a ticket (or get stopped holding the latch) in between.
pub(super) fn try_enter(queue: &Path) -> io::Result<File> {
    std::fs::create_dir_all(queue)?;
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(queue.join(".enter"))?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(file)
}

pub(super) struct Ticket {
    pub pid: i32,
    arrived: u128,
    path: PathBuf,
}

pub(super) fn live_tickets(queue: &Path) -> io::Result<Vec<Ticket>> {
    let mut tickets = Vec::new();
    for entry in std::fs::read_dir(queue)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some((arrived, pid)) = name.to_str().and_then(|s| s.strip_suffix(".ticket"))
            .and_then(|s| s.split_once('-')) else { continue };
        let (Ok(arrived), Ok(pid)) = (arrived.parse::<u128>(), pid.parse::<i32>()) else { continue };
        if pid <= 1 || process_state(pid) == ProcessState::Gone { continue; }
        match entry.file_type() {
            Ok(kind) if kind.is_file() => tickets.push(Ticket { pid, arrived, path: entry.path() }),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    tickets.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    Ok(tickets)
}

pub(super) fn waiting(queue: &Path, tickets: &[Ticket], candidate_groups: &HashSet<i32>) -> HashMap<i32, String> {
    let Some(cutoff) = SystemTime::now().duration_since(UNIX_EPOCH).ok()
        .and_then(|now| now.checked_sub(REPORT_AFTER)).map(|t| t.as_nanos()) else { return Default::default() };
    let locate = |t: &Ticket| {
        let pid = t.pid;
        let group = crate::process_reads::DETAILS.read(move || unsafe { libc::getpgid(pid) })?;
        let status = process_state(t.pid);
        (group > 0 && status != ProcessState::Gone).then_some((group, status))
    };
    let waiters: Vec<_> = tickets.iter().filter_map(|t| locate(t).map(|(group, status)| (t, group, status))).collect();
    if !waiters.iter().any(|(t, group, _)| t.arrived < cutoff && candidate_groups.contains(group)) {
        return Default::default();
    }
    // An older queue can block the canonical head itself during the transition.
    let legacy = snapshot().and_then(|raw| {
        let pids = raw.lines().filter_map(|row| row.split_whitespace().next()?.parse().ok());
        legacy_queues(queue, pids).ok()?.into_iter()
            .filter_map(|old| live_tickets(&old).ok()?.into_iter().next())
            .min_by_key(|ticket| ticket.arrived)
    });
    let holder = legacy.as_ref().and_then(|t| locate(t).map(|(group, status)| (t, group, status)))
        .or_else(|| waiters.first().copied());
    let Some((holder, holder_group, status)) = holder else { return Default::default() };
    let mut output = HashMap::new();
    for (waiter, group, _) in waiters {
        if waiter.arrived >= cutoff || group == holder_group || !candidate_groups.contains(&group) { continue; }
        output.entry(group).or_insert_with(|| holder.diagnostic(status == ProcessState::Stopped));
    }
    output
}

impl Ticket {
    fn diagnostic(&self, paused: bool) -> String {
        let worktree = (|| {
            let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&self.path).ok()?;
            if !file.metadata().ok()?.is_file() { return None; }
            let mut note = String::new();
            file.take(4096).read_to_string(&mut note).ok()?;
            let (path, _) = note.strip_prefix("worktree ")?.split_once(&format!(", PID {},", self.pid))?;
            Some(path.chars().filter(|c| !c.is_control()).take(512).collect::<String>())
        })();
        let holder = match worktree {
            Some(path) => format!("PID {} ({path})", self.pid),
            None => format!("PID {}", self.pid),
        };
        let paused = if paused { " and is paused" } else { "" };
        format!("Waiting for the machine-wide heavy-job lock. {holder} holds it{paused}.")
    }
}
