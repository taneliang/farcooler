//! Read the ticket format shared by scripts/heavy and lane-gate.py.
use crate::proc_table::{ProcessState, process_state};
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) const NAME: &str = "lane-gate.lock.queue";
const REPORT_AFTER: Duration = Duration::from_secs(300);

pub(super) fn machine() -> PathBuf { Path::new("/tmp").join(NAME) }

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

pub(super) fn waiting(tickets: &[Ticket], candidate_groups: &HashSet<i32>) -> HashMap<i32, String> {
    let Some(cutoff) = SystemTime::now().duration_since(UNIX_EPOCH).ok()
        .and_then(|now| now.checked_sub(REPORT_AFTER)).map(|t| t.as_nanos()) else { return Default::default() };
    let mut live = tickets.iter().filter_map(|t| {
        let group = unsafe { libc::getpgid(t.pid) };
        let status = process_state(t.pid);
        (group > 0 && status != ProcessState::Gone).then_some((t, group, status))
    });
    let Some((holder, holder_group, status)) = live.next() else { return Default::default() };
    let mut output = HashMap::new();
    for (waiter, group, _) in live {
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
