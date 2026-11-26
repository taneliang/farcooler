//! Process evidence for effects admitted before Git writer locks were recorded.
use std::path::{Path, PathBuf};
use std::{collections::HashSet, ffi::OsStr, os::unix::ffi::OsStrExt};

/// Look for worktree add, push and receive-pack by their path arguments or
/// working directory. Checkout/reset and the local receiver run in the tree
/// or bare repository. A failed inventory is not evidence of quiescence.
pub(crate) async fn legacy_git_writers(paths: &[&Path]) -> Option<bool> {
    let paths = paths.iter().flat_map(|path| {
        let canonical = path.canonicalize().ok().filter(|resolved| resolved != *path);
        std::iter::once(path.to_path_buf()).chain(canonical)
    }).collect::<Vec<_>>();
    crate::process_reads::OPEN_FILES.read_async(move || writers(&paths)).await.flatten()
}

fn writers(paths: &[PathBuf]) -> Option<bool> {
    // These commands, including a local receive-pack, run as the runner user.
    let me = std::process::id() as i32;
    let mut checked = HashSet::from([me]);
    let mut unreadable = HashSet::new();
    for _ in 0..4 {
        let pids = own_pids()?;
        if !pids.contains(&me) { return None; }
        unreadable.retain(|pid| pids.contains(pid));
        let mut changed = false;
        for pid in pids {
            let new = checked.insert(pid);
            if !new && !unreadable.remove(&pid) { continue; }
            changed |= new;
            match could_write(pid, paths) {
                Some(true) => return Some(true),
                Some(false) => {}
                None => { unreadable.insert(pid); }
            }
        }
        // A command that exited during our reads may have forked a receiver
        // or checkout helper since the first inventory.
        if !changed && unreadable.is_empty() { return Some(false); }
    }
    None
}

fn could_write(pid: i32, paths: &[PathBuf]) -> Option<bool> {
    let cwd = super::process_cwd(pid);
    if let Some(cwd) = &cwd {
        let raw = cwd.as_os_str().as_bytes();
        let cwd = Path::new(OsStr::from_bytes(raw.strip_suffix(b" (deleted)").unwrap_or(raw)));
        if paths.iter().any(|path| cwd.starts_with(path)) { return Some(true); }
    }
    let exe = super::read_path_of(pid);
    let git = exe.as_deref().and_then(|exe| Path::new(exe).file_name())
        .is_some_and(|name| name.to_string_lossy().contains("git"));
    if !git && cwd.is_some() { return Some(false); }
    match super::read_command_of(pid) {
        Some((_, args)) if paths.iter().any(|path| args.contains(path.to_string_lossy().as_ref())) =>
            Some(true),
        Some(_) if !git || cwd.is_some() => Some(false),
        _ if super::read_state(pid) == super::ProcessState::Gone => Some(false),
        _ if exe.is_some() && !git => Some(false),
        // A writer might be in exec. Wait for a clear answer.
        _ => None,
    }
}

#[cfg(target_os = "macos")]
fn own_pids() -> Option<Vec<i32>> {
    // From <sys/proc_info.h>. proc_listpids returns bytes, not PID count.
    const PROC_UID_ONLY: u32 = 4;
    let uid = unsafe { libc::geteuid() };
    let bytes = unsafe { libc::proc_listpids(PROC_UID_ONLY, uid, std::ptr::null_mut(), 0) };
    let mut pids = vec![0i32; usize::try_from(bytes).ok()? / size_of::<i32>() + 64];
    let capacity = i32::try_from(size_of_val(pids.as_slice())).ok()?;
    let filled = unsafe { libc::proc_listpids(PROC_UID_ONLY, uid, pids.as_mut_ptr().cast(), capacity) };
    let filled = usize::try_from(filled).ok()?;
    if filled >= capacity as usize || filled % size_of::<i32>() != 0 { return None; }
    pids.truncate(filled / size_of::<i32>());
    pids.retain(|&pid| pid > 0);
    Some(pids)
}

#[cfg(target_os = "linux")]
fn own_pids() -> Option<Vec<i32>> {
    use std::os::unix::fs::MetadataExt;
    let uid = unsafe { libc::geteuid() };
    let mut pids = Vec::new();
    for entry in std::fs::read_dir("/proc").ok()? {
        let entry = entry.ok()?;
        let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse().ok()) else { continue };
        match entry.metadata() {
            Ok(meta) if meta.uid() == uid => pids.push(pid),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
    }
    Some(pids)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn own_pids() -> Option<Vec<i32>> { None }
