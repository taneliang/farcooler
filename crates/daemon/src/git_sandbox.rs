//! The exec allowlist every git and gh the daemon starts runs inside: the
//! process, and everything it starts, may execute only the programs named.
//!
//! `crate::git_guard` turns off, key by key, what a repository's config can
//! make git run. That depends on knowing every key, and on reading the config
//! before git does; an agent rewriting its config in a loop lands a hook in
//! the gap between the guard's `git config` listing and the call it guards
//! (measured: about one guarded status in five). This doesn't read config at
//! all. Whatever a config names, git can only start it by `exec`, and the
//! kernel refuses any `exec` of a program not on the list.
//!
//! **What is on the list**, by exact path, never a directory (a `bin`
//! directory holds interpreters, and an interpreter runs whatever it's fed):
//!
//! - git itself: the binary the daemon starts, which is `<git --exec-path>/git`
//!   resolved through its symlinks. git starts a child git for some of its own
//!   work (`worktree add` runs `update-ref`; a dashed builtin re-execs), and
//!   always through `<exec-path>/git`, which git puts first on its children's
//!   `PATH`. Everything else in the exec-path is a helper the daemon never
//!   asks for: every subcommand it runs is a builtin in git 2.54, and the
//!   transports (`git-remote-https`, `ssh`) are off (`protocol.allow=never`).
//!   The `git` the daemon found (on a Mac, `/usr/bin/git`, which only starts
//!   the developer directory's git) is on the list too, as a fallback for an
//!   install with no `git` in its exec-path.
//! - For gh only: gh, the git and ssh gh finds on the `PATH` it is handed (it
//!   runs `git remote -v` and `git config` to find the repository, and
//!   `ssh -G <host>` to resolve an ssh host alias in a remote's URL), and on
//!   a Mac `/usr/bin/security`, which gh runs to read its token from the login
//!   keychain (measured: without it, `gh pr list` answers HTTP 401). The
//!   daemon's own git calls never get these.
//! - On Linux, the ELF interpreter (`ld-linux…`) each of those names, since
//!   the kernel opens it for execution as well.
//!
//! **No interpreter, ever**: no `sh`, `bash`, `perl`, `python`. No daemon git
//! needs one. git runs every hook and every filter, textconv, external diff
//! and fsmonitor command with arguments through `sh -c`, and a script's `#!`
//! line through its interpreter, so with no shell on the list none of them
//! can start, whatever the config says. That includes git-lfs: its filter is
//! `git-lfs filter-process`, which git starts through `sh -c` (see
//! `crate::git_guard` for why LFS is turned off rather than allowed).
//!
//! **How**: on a Mac, a Seatbelt profile (`(deny process-exec*)` and an
//! `(allow process-exec (literal …))` per path), compiled once in the daemon
//! and applied in the child between `fork` and `exec` (`sandbox_apply`, from
//! libsandbox, found at run time). On Linux, a Landlock ruleset that handles
//! `EXECUTE` and grants it on each path's inode, applied in the child the same
//! way (`landlock_restrict_self`, after `PR_SET_NO_NEW_PRIVS`). Either way the
//! daemon itself is untouched, and the restriction is inherited by everything
//! git starts and can't be lifted.
//!
//! **When it's unavailable**, git runs anyway, under `crate::git_guard`'s pins
//! alone, and the daemon says so once in its log: libsandbox missing or
//! refusing to compile, or a kernel without Landlock (older than 5.13, or
//! with it left out of the boot LSM list). An agent sandbox uses the same
//! mechanism on each OS (codex: Seatbelt; Landlock and seccomp), so on a host
//! where the daemon can't confine git the agent isn't confined either, and
//! the daemon was never its boundary. Refusing to run git there would break
//! review for no gain. A sandbox that compiled but fails to apply in the
//! child is a different thing: that git doesn't start.
//!
//! **What it doesn't do**: it stops programs, not writes. git itself can
//! still be steered to read or write where its config says (`core.worktree`,
//! pinned in `crate::git_guard`), and those reads are what the pins are for.

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The programs one kind of process may execute, ready to apply to a child.
pub struct Sandbox {
    allowed: Vec<PathBuf>,
    #[cfg(target_os = "macos")]
    compiled: Arc<imp::Compiled>,
}

impl std::fmt::Debug for Sandbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sandbox").field("allowed", &self.allowed).finish()
    }
}

impl Sandbox {
    /// A sandbox for `program` allowing exactly `allowed`, or `None` when
    /// this host can't confine a process (logged, once per reason).
    ///
    /// Also `None` when `program` is itself a script: it can't start without
    /// its `#!` interpreter, and an interpreter on the list would undo it. A
    /// test's stand-in gh is one; so is a package manager's shell wrapper.
    pub fn new(program: &Path, allowed: Vec<PathBuf>) -> Option<Arc<Sandbox>> {
        if is_script(program) {
            unavailable(&format!("{} is a script, and no interpreter is ever allowed", program.display()));
            return None;
        }
        match imp::check(&allowed) {
            Ok(()) => {}
            Err(why) => {
                unavailable(&why);
                return None;
            }
        }
        #[cfg(target_os = "macos")]
        {
            match imp::compile(&allowed) {
                Ok(compiled) => Some(Arc::new(Sandbox { allowed, compiled: Arc::new(compiled) })),
                Err(why) => {
                    unavailable(&why);
                    None
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        Some(Arc::new(Sandbox { allowed }))
    }

    /// The paths this sandbox lets a process execute.
    pub fn allowed(&self) -> &[PathBuf] {
        &self.allowed
    }

    /// Make `cmd`'s child, and everything it starts, unable to execute
    /// anything but [`Sandbox::allowed`].
    ///
    /// On Linux the ruleset is built here, per spawn, because a Landlock rule
    /// holds the inode a path named when it was made: a package upgrade that
    /// replaces `/usr/bin/ssh` would leave a cached rule allowing the old file.
    /// Building one is a few syscalls. A ruleset that can't be built here
    /// (the kernel said yes at [`Sandbox::new`]) fails the spawn.
    pub fn confine(&self, cmd: &mut std::process::Command) -> io::Result<()> {
        use std::os::unix::process::CommandExt;
        #[cfg(target_os = "macos")]
        let compiled = self.compiled.clone();
        #[cfg(not(target_os = "macos"))]
        let compiled = imp::compile(&self.allowed).map_err(io::Error::other)?;
        // SAFETY: the closure runs in the child between fork and exec, and
        // does only what is safe there: one or two syscalls on memory and
        // descriptors the parent prepared, and no allocation.
        unsafe {
            cmd.pre_exec(move || imp::apply(&compiled));
        }
        Ok(())
    }
}

/// Say once per reason, for the life of the daemon, that git or gh runs
/// unconfined.
fn unavailable(why: &str) {
    static SAID: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut said = SAID.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if said.iter().any(|s| s == why) {
        return;
    }
    said.push(why.to_string());
    tracing::warn!(
        reason = why,
        "git or gh runs without the exec allowlist on this host; \
         only the config pins guard what a repository can make it run"
    );
}

/// Whether `path` starts with `#!`.
pub(crate) fn is_script(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 2];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && head == *b"#!"
}

/// `programs`, each also by the path its symlinks resolve to, and on Linux
/// with the ELF interpreter each one names; in order, without repeats, and
/// without a path that isn't there.
pub fn allowlist<P: AsRef<Path>>(programs: &[P]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if p.is_absolute() && !out.contains(&p) {
            out.push(p);
        }
    };
    for program in programs {
        let program = program.as_ref();
        if !program.exists() {
            continue;
        }
        push(program.to_path_buf());
        if let Ok(real) = program.canonicalize() {
            #[cfg(target_os = "linux")]
            if let Some(interp) = elf_interpreter(&real) {
                push(interp.clone());
                if let Ok(real) = interp.canonicalize() {
                    push(real);
                }
            }
            push(real);
        }
    }
    out
}

/// Paths a resolve read, each with the file it resolved to then, so a cached
/// allowlist can tell it's stale: a Homebrew upgrade repoints
/// `/opt/homebrew/bin/git` at a new Cellar directory, and the old one may
/// stay behind, so "the old path still exists" isn't enough.
#[derive(Debug, Default)]
pub struct Resolved(Vec<(PathBuf, Option<PathBuf>)>);

impl Resolved {
    pub fn of(paths: &[PathBuf]) -> Resolved {
        Resolved(paths.iter().map(|p| (p.clone(), p.canonicalize().ok())).collect())
    }

    /// Whether every path still resolves where it did.
    pub fn unchanged(&self) -> bool {
        self.0.iter().all(|(path, then)| path.canonicalize().ok() == *then)
    }
}

/// The real git behind `found`: `<found --exec-path>/git` with its symlinks
/// resolved, or `found` itself when that can't be had.
///
/// On a Mac `found` is usually `/usr/bin/git`, a shim that looks up the
/// developer directory and starts the git there, which costs about 5 ms a
/// call. Starting that git directly skips it; the configuration it reads is
/// the same (measured: `--exec-path`, the system config and the environment
/// git hands its children all match, apart from SDK variables the shim adds
/// for compilers).
///
/// Run once per resolve, in `/`, with every `GIT_` variable removed so
/// `GIT_EXEC_PATH` can't steer it. `--exec-path` reads no config.
pub fn real_git(found: &Path) -> PathBuf {
    let mut cmd = std::process::Command::new(found);
    for (key, _) in std::env::vars_os() {
        if key.as_bytes().starts_with(b"GIT_") {
            cmd.env_remove(key);
        }
    }
    let out = cmd
        .arg("--exec-path")
        .current_dir("/")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    let Ok(out) = out else { return found.to_path_buf() };
    if !out.status.success() {
        return found.to_path_buf();
    }
    let dir = OsStr::from_bytes(out.stdout.trim_ascii_end());
    let candidate = Path::new(dir).join("git");
    match candidate.canonicalize() {
        Ok(real) if dir.as_bytes().starts_with(b"/") && is_executable(&real) => real,
        _ => found.to_path_buf(),
    }
}

/// `PATH`, with every entry that isn't an absolute directory dropped: `.`,
/// the empty entry a stray `:` makes, and any relative one. Each of those is
/// looked up from the child's own working directory, which for git is the
/// agent's worktree.
pub fn absolute_path_entries(path: &OsStr) -> OsString {
    let kept: Vec<&[u8]> = path.as_bytes().split(|b| *b == b':').filter(|e| e.starts_with(b"/")).collect();
    OsStr::from_bytes(&kept.join(&b':')).to_os_string()
}

/// The first `name` on `path` that is an executable file, the way `execvp`
/// (and Go's `exec.LookPath`, which gh uses) would find it.
pub fn on_path(name: &str, path: &OsStr) -> Option<PathBuf> {
    path.as_bytes()
        .split(|b| *b == b':')
        .filter(|e| e.starts_with(b"/"))
        .map(|dir| Path::new(OsStr::from_bytes(dir)).join(name))
        .find(|p| is_executable(p))
}

pub(crate) fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The `PT_INTERP` path of the ELF file at `path`: the dynamic loader the
/// kernel opens to run it. `None` for a static binary, a script, or a file
/// that isn't ELF.
#[cfg(any(target_os = "linux", test))]
pub fn elf_interpreter(path: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::FileExt;
    let file = std::fs::File::open(path).ok()?;
    let mut ident = [0u8; 64];
    let n = file.read_at(&mut ident, 0).ok()?;
    let header = elf_header(&ident[..n])?;
    let mut table = vec![0u8; header.entry_size * header.count];
    file.read_exact_at(&mut table, header.offset).ok()?;
    let (offset, size) = interp_segment(&header, &table)?;
    if size == 0 || size > 4096 {
        return None;
    }
    let mut bytes = vec![0u8; size];
    file.read_exact_at(&mut bytes, offset).ok()?;
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    let interp = PathBuf::from(OsStr::from_bytes(&bytes[..end]));
    interp.is_absolute().then_some(interp)
}

/// Where an ELF file's program headers are, and how to read them.
#[cfg(any(target_os = "linux", test))]
struct ElfHeader {
    wide: bool,
    little: bool,
    offset: u64,
    entry_size: usize,
    count: usize,
}

#[cfg(any(target_os = "linux", test))]
fn elf_header(bytes: &[u8]) -> Option<ElfHeader> {
    if bytes.len() < 52 || &bytes[..4] != b"\x7fELF" {
        return None;
    }
    let wide = match bytes[4] {
        1 => false,
        2 => true,
        _ => return None,
    };
    let little = match bytes[5] {
        1 => true,
        2 => false,
        _ => return None,
    };
    let (offset, entry_size, count) = if wide {
        if bytes.len() < 64 {
            return None;
        }
        (read(bytes, 32, 8, little)?, read(bytes, 54, 2, little)?, read(bytes, 56, 2, little)?)
    } else {
        (read(bytes, 28, 4, little)?, read(bytes, 42, 2, little)?, read(bytes, 44, 2, little)?)
    };
    let least = if wide { 56 } else { 32 };
    let (entry_size, count) = (usize::try_from(entry_size).ok()?, usize::try_from(count).ok()?);
    if entry_size < least || count == 0 || count > 256 {
        return None;
    }
    Some(ElfHeader { wide, little, offset, entry_size, count })
}

/// The file offset and size of the `PT_INTERP` segment, if there is one.
#[cfg(any(target_os = "linux", test))]
fn interp_segment(header: &ElfHeader, table: &[u8]) -> Option<(u64, usize)> {
    const PT_INTERP: u64 = 3;
    for entry in table.chunks_exact(header.entry_size) {
        if read(entry, 0, 4, header.little)? != PT_INTERP {
            continue;
        }
        let (offset, size) = if header.wide {
            (read(entry, 8, 8, header.little)?, read(entry, 32, 8, header.little)?)
        } else {
            (read(entry, 4, 4, header.little)?, read(entry, 16, 4, header.little)?)
        };
        return Some((offset, usize::try_from(size).ok()?));
    }
    None
}

/// The `width`-byte unsigned integer at `at`, in the file's byte order.
#[cfg(any(target_os = "linux", test))]
fn read(bytes: &[u8], at: usize, width: usize, little: bool) -> Option<u64> {
    let field = bytes.get(at..at + width)?;
    let mut value = 0u64;
    for i in 0..width {
        let byte = if little { field[width - 1 - i] } else { field[i] };
        value = (value << 8) | u64::from(byte);
    }
    Some(value)
}

/// Seatbelt, through libsandbox: the profile is compiled in the daemon, and
/// only applying it (one call) happens in the child.
///
/// `sandbox_compile_string` and `sandbox_apply` aren't in a public header.
/// They are what `sandbox-exec` and Chromium's sandbox use, and they're looked
/// up when the daemon first needs them rather than linked, so a macOS that
/// drops them gets a daemon that runs git unconfined and says so, not one
/// that fails to start.
#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::io;
    use std::path::PathBuf;
    use std::sync::OnceLock;

    type Compile = unsafe extern "C" fn(*const c_char, *mut c_void, *mut *mut c_char) -> *mut c_void;
    type Apply = unsafe extern "C" fn(*mut c_void) -> c_int;
    type Free = unsafe extern "C" fn(*mut c_void);

    struct Lib {
        compile: Compile,
        apply: Apply,
        free: Free,
    }

    fn lib() -> Result<&'static Lib, String> {
        static LIB: OnceLock<Result<Lib, String>> = OnceLock::new();
        LIB.get_or_init(|| {
            // SAFETY: dlopen and dlsym with NUL-terminated names; each symbol
            // is checked for null before it is turned into a function pointer
            // of the type libsandbox has exported since macOS 10.x.
            unsafe {
                let handle = libc::dlopen(c"/usr/lib/libsandbox.1.dylib".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
                if handle.is_null() {
                    return Err("libsandbox can't be loaded".to_string());
                }
                let find = |name: &CStr| {
                    let sym = libc::dlsym(handle, name.as_ptr());
                    (!sym.is_null()).then_some(sym)
                };
                let (Some(compile), Some(apply), Some(free)) =
                    (find(c"sandbox_compile_string"), find(c"sandbox_apply"), find(c"sandbox_free_profile"))
                else {
                    return Err("libsandbox has no sandbox_compile_string or sandbox_apply".to_string());
                };
                Ok(Lib {
                    compile: std::mem::transmute::<*mut c_void, Compile>(compile),
                    apply: std::mem::transmute::<*mut c_void, Apply>(apply),
                    free: std::mem::transmute::<*mut c_void, Free>(free),
                })
            }
        })
        .as_ref()
        .map_err(Clone::clone)
    }

    /// A compiled profile, owned.
    pub struct Compiled {
        profile: *mut c_void,
        apply: Apply,
        free: Free,
    }

    // SAFETY: the profile is immutable once compiled; it is only read, by
    // `sandbox_apply` in a forked child, and freed once, on drop.
    unsafe impl Send for Compiled {}
    unsafe impl Sync for Compiled {}

    impl Drop for Compiled {
        fn drop(&mut self) {
            // SAFETY: `profile` came from `sandbox_compile_string` and is
            // freed exactly once.
            unsafe { (self.free)(self.profile) }
        }
    }

    /// Every path has to be spelled in the profile, so one that isn't
    /// UTF-8 can't be allowed.
    pub fn check(allowed: &[PathBuf]) -> Result<(), String> {
        lib()?;
        match allowed.iter().find(|p| p.to_str().is_none()) {
            Some(p) => Err(format!("{} isn't UTF-8, so a Seatbelt profile can't name it", p.display())),
            None => Ok(()),
        }
    }

    /// The profile's text: everything allowed but `exec`, and `exec` only of
    /// each path, literally.
    pub fn profile(allowed: &[PathBuf]) -> String {
        let mut text = String::from("(version 1)\n(allow default)\n(deny process-exec*)\n(allow process-exec");
        for path in allowed {
            let path = path.to_string_lossy();
            text.push_str("\n  (literal \"");
            for c in path.chars() {
                if c == '"' || c == '\\' {
                    text.push('\\');
                }
                text.push(c);
            }
            text.push_str("\")");
        }
        text.push_str(")\n");
        text
    }

    pub fn compile(allowed: &[PathBuf]) -> Result<Compiled, String> {
        let lib = lib()?;
        let text = CString::new(profile(allowed)).map_err(|_| "a path holds a NUL".to_string())?;
        let mut error: *mut c_char = std::ptr::null_mut();
        // SAFETY: `text` is NUL-terminated and outlives the call; params may
        // be null; `error` is set to a malloc'd string only on failure.
        let profile = unsafe { (lib.compile)(text.as_ptr(), std::ptr::null_mut(), &mut error) };
        if profile.is_null() {
            let why = if error.is_null() {
                "no reason given".to_string()
            } else {
                // SAFETY: a NUL-terminated string libsandbox allocated with
                // malloc, read once and freed once.
                let why = unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned();
                unsafe { libc::free(error.cast()) };
                why
            };
            return Err(format!("the Seatbelt profile didn't compile: {why}"));
        }
        Ok(Compiled { profile, apply: lib.apply, free: lib.free })
    }

    /// In the child, between fork and exec.
    pub fn apply(compiled: &Compiled) -> io::Result<()> {
        // SAFETY: a compiled profile the parent owns for as long as the
        // command that carries this closure lives.
        match unsafe { (compiled.apply)(compiled.profile) } {
            0 => Ok(()),
            _ => Err(io::Error::from_raw_os_error(libc::EPERM)),
        }
    }
}

/// Landlock: a ruleset handling `EXECUTE`, with that right granted on each
/// allowed file, built in the daemon and applied in the child.
#[cfg(target_os = "linux")]
mod imp {
    use std::io;
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::path::PathBuf;

    use landlock::{AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr};

    pub struct Compiled {
        ruleset: OwnedFd,
    }

    /// Whether this kernel enforces Landlock at all, asked by building an
    /// empty ruleset.
    pub fn check(_allowed: &[PathBuf]) -> Result<(), String> {
        compile(&[]).map(|_| ())
    }

    pub fn compile(allowed: &[PathBuf]) -> Result<Compiled, String> {
        let mut ruleset = Ruleset::default()
            .handle_access(AccessFs::Execute)
            .and_then(|r| r.create())
            .map_err(|e| format!("Landlock refused a ruleset: {e}"))?;
        for path in allowed {
            // A path gone since the list was made (an upgrade mid-flight) is
            // left out, which refuses it: closed, not open.
            let Ok(fd) = PathFd::new(path) else { continue };
            ruleset = ruleset
                .add_rule(PathBeneath::new(fd, AccessFs::Execute))
                .map_err(|e| format!("Landlock refused a rule for {}: {e}", path.display()))?;
        }
        // Best effort: on a kernel without Landlock the ruleset has no
        // descriptor, and that is the "unavailable" answer.
        let fd: Option<OwnedFd> = ruleset.into();
        fd.map(|ruleset| Compiled { ruleset })
            .ok_or_else(|| "this kernel doesn't enforce Landlock".to_string())
    }

    /// In the child, between fork and exec: no new privileges (which
    /// Landlock requires of an unprivileged process), then the ruleset.
    pub fn apply(compiled: &Compiled) -> io::Result<()> {
        // SAFETY: two raw syscalls with integer arguments; nothing allocated.
        unsafe {
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::syscall(libc::SYS_landlock_restrict_self, compiled.ruleset.as_raw_fd(), 0u32) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }
}

/// Neither: there is nothing to confine git with.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod imp {
    use std::io;
    use std::path::PathBuf;

    pub struct Compiled;

    pub fn check(_allowed: &[PathBuf]) -> Result<(), String> {
        Err("no exec sandbox on this OS".to_string())
    }

    pub fn compile(_allowed: &[PathBuf]) -> Result<Compiled, String> {
        Err("no exec sandbox on this OS".to_string())
    }

    pub fn apply(_compiled: &Compiled) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_keeps_only_absolute_entries() {
        let path = OsStr::new(".:/usr/bin::bin:./x:/bin:");
        assert_eq!(absolute_path_entries(path), OsString::from("/usr/bin:/bin"));
        assert_eq!(absolute_path_entries(OsStr::new("")), OsString::new());
    }

    #[test]
    fn a_name_is_found_only_in_an_absolute_directory() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().canonicalize().unwrap();
        let tool = dir.join("tool");
        std::fs::write(&tool, "").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = OsString::from(format!(".:{}", dir.display()));
        assert_eq!(on_path("tool", &path), Some(tool));
        assert_eq!(on_path("absent", &path), None);
    }

    /// A 64-bit little-endian ELF with one `PT_INTERP` naming `interp`.
    fn elf64(interp: &[u8]) -> Vec<u8> {
        let mut f = vec![0u8; 64 + 56];
        f[..4].copy_from_slice(b"\x7fELF");
        f[4] = 2; // 64-bit
        f[5] = 1; // little-endian
        f[32..40].copy_from_slice(&64u64.to_le_bytes()); // e_phoff
        f[54..56].copy_from_slice(&56u16.to_le_bytes()); // e_phentsize
        f[56..58].copy_from_slice(&1u16.to_le_bytes()); // e_phnum
        let at = f.len() as u64;
        f[64..68].copy_from_slice(&3u32.to_le_bytes()); // PT_INTERP
        f[64 + 8..64 + 16].copy_from_slice(&at.to_le_bytes()); // p_offset
        f[64 + 32..64 + 40].copy_from_slice(&(interp.len() as u64 + 1).to_le_bytes()); // p_filesz
        f.extend_from_slice(interp);
        f.push(0);
        f
    }

    #[test]
    fn the_elf_interpreter_is_read_from_the_program_headers() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::write(&bin, elf64(b"/lib64/ld-linux-x86-64.so.2")).unwrap();
        assert_eq!(elf_interpreter(&bin), Some(PathBuf::from("/lib64/ld-linux-x86-64.so.2")));

        // A script, and an ELF with no interpreter (a static binary).
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        assert_eq!(elf_interpreter(&bin), None);
        let mut fixed = elf64(b"/x");
        fixed[64..68].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD, not PT_INTERP
        std::fs::write(&bin, fixed).unwrap();
        assert_eq!(elf_interpreter(&bin), None);
    }

    #[test]
    fn a_32_bit_big_endian_elf_is_read_too() {
        let mut f = vec![0u8; 52 + 32];
        f[..4].copy_from_slice(b"\x7fELF");
        f[4] = 1;
        f[5] = 2;
        f[28..32].copy_from_slice(&52u32.to_be_bytes());
        f[42..44].copy_from_slice(&32u16.to_be_bytes());
        f[44..46].copy_from_slice(&1u16.to_be_bytes());
        let at = f.len() as u32;
        f[52..56].copy_from_slice(&3u32.to_be_bytes());
        f[52 + 4..52 + 8].copy_from_slice(&at.to_be_bytes());
        f[52 + 16..52 + 20].copy_from_slice(&8u32.to_be_bytes());
        f.extend_from_slice(b"/lib/ld\0");
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::write(&bin, f).unwrap();
        assert_eq!(elf_interpreter(&bin), Some(PathBuf::from("/lib/ld")));
    }

    #[test]
    fn the_allowlist_resolves_symlinks_and_drops_what_isnt_there() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().canonicalize().unwrap();
        let real = dir.join("real");
        std::fs::write(&real, "").unwrap();
        let link = dir.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let list = allowlist(&[link.clone(), dir.join("absent"), real.clone()]);
        assert_eq!(list, [link, real]);
    }

    /// A repointed symlink is a change; the same target is not.
    #[test]
    fn a_resolve_is_stale_once_a_symlink_points_elsewhere() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().canonicalize().unwrap();
        for v in ["1", "2"] {
            std::fs::create_dir(dir.join(v)).unwrap();
            std::fs::write(dir.join(v).join("git"), "").unwrap();
        }
        let link = dir.join("git");
        std::os::unix::fs::symlink(dir.join("1/git"), &link).unwrap();
        let resolved = Resolved::of(&[link.clone()]);
        assert!(resolved.unchanged());
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(dir.join("2/git"), &link).unwrap();
        assert!(!resolved.unchanged(), "the old Cellar path is still there, and stale");
    }

    /// A script can't run without its interpreter, and no interpreter is
    /// allowed: it runs unconfined rather than not at all.
    #[test]
    fn a_program_that_is_a_script_gets_no_sandbox() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("gh");
        std::fs::write(&script, "#!/bin/sh\necho\n").unwrap();
        assert!(Sandbox::new(&script, allowlist(&[&script])).is_none());
        assert!(is_script(&script));
        assert!(!is_script(Path::new("/bin/sh")), "a binary isn't one");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_profile_names_each_path_literally_and_escapes_quotes() {
        let text = imp::profile(&[PathBuf::from("/usr/bin/git"), PathBuf::from("/a \"b\"\\c")]);
        assert!(text.contains("(deny process-exec*)"), "{text}");
        assert!(text.contains("(literal \"/usr/bin/git\")"), "{text}");
        assert!(text.contains(r#"(literal "/a \"b\"\\c")"#), "{text}");
        assert!(!text.contains("subpath"), "never a directory: {text}");
    }
}

/// The allowlist on its own: each test plants a program in a repository's
/// config, turns `crate::git_guard` off entirely (no pins, no flags, no
/// listing: `git::UNGUARDED`), and runs the daemon's own status. First with
/// the sandbox off too, where the program must run (the control), then with
/// it on, where it must not.
#[cfg(test)]
mod alone {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use crate::git::{UNGUARDED, UNSANDBOXED, git_launch};

    struct Fixture {
        dir: tempfile::TempDir,
    }

    fn git(dir: &Path, args: &[&str]) {
        let mut cmd = std::process::Command::new("git");
        for (k, _) in std::env::vars_os() {
            if k.to_string_lossy().starts_with("GIT_") {
                cmd.env_remove(k);
            }
        }
        // Without the user's own git-lfs, which may be configured and not
        // installed.
        cmd.args(["-c", "filter.lfs.process=", "-c", "filter.lfs.clean=", "-c", "filter.lfs.required=false"]);
        let out = cmd.current_dir(dir).args(["-c", "core.hooksPath=/dev/null"]).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    impl Fixture {
        /// A repository with `a.txt` under `attributes`, and a probe that
        /// leaves `ran-<first argument>`.
        fn new(attributes: &str) -> Self {
            let f = Fixture { dir: tempfile::tempdir().unwrap() };
            let probe = "#!/bin/sh\n\
                         touch \"$(dirname \"$0\")/ran-$1\"\n\
                         case \"$1\" in clean) cat ;; fsmonitor) printf '\\0' ;; esac\n\
                         exit 0\n";
            for name in ["probe", "git-lfs"] {
                std::fs::write(f.root().join(name), probe).unwrap();
                std::fs::set_permissions(f.root().join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let repo = f.repo();
            std::fs::create_dir_all(&repo).unwrap();
            git(&repo, &["init", "-q", "-b", "main"]);
            git(&repo, &["config", "user.email", "t@example.com"]);
            git(&repo, &["config", "user.name", "t"]);
            git(&repo, &["config", "commit.gpgsign", "false"]);
            std::fs::write(repo.join("a.txt"), "one\n").unwrap();
            std::fs::write(repo.join(".gitattributes"), attributes).unwrap();
            git(&repo, &["add", "-A"]);
            git(&repo, &["commit", "-q", "-m", "base"]);
            f
        }

        fn root(&self) -> PathBuf {
            self.dir.path().canonicalize().unwrap()
        }

        fn repo(&self) -> PathBuf {
            self.root().join("repo")
        }

        fn probe(&self, role: &str) -> String {
            format!("{} {role}", self.root().join("probe").display())
        }

        fn plant(&self, key: &str, value: &str) {
            git(&self.repo(), &["config", "--add", key, value]);
        }

        /// Every probe that ran, by role, and the record cleared.
        fn ran(&self) -> Vec<String> {
            let mut ran = Vec::new();
            for entry in std::fs::read_dir(self.root()).unwrap() {
                let name = entry.unwrap().file_name().into_string().unwrap();
                if let Some(role) = name.strip_prefix("ran-") {
                    ran.push(role.to_string());
                    std::fs::remove_file(self.root().join(&name)).unwrap();
                }
            }
            ran.sort();
            ran
        }

        /// Same content, new mtime: git can't trust the index's stat data,
        /// reads the file (through its filter) and rewrites the index.
        fn restat(&self) {
            std::thread::sleep(std::time::Duration::from_millis(1100));
            let path = self.repo().join("a.txt");
            let text = std::fs::read(&path).unwrap();
            std::fs::write(&path, text).unwrap();
        }

        /// The daemon's status with the guard off, sandboxed or not; what
        /// ran.
        async fn status(&self, sandboxed: bool) -> Vec<String> {
            self.restat();
            UNGUARDED.with(|u| u.set(true));
            UNSANDBOXED.with(|u| u.set(!sandboxed));
            let _ = crate::change_set::working_tree(&self.repo()).await;
            UNGUARDED.with(|u| u.set(false));
            UNSANDBOXED.with(|u| u.set(false));
            self.ran()
        }

        /// The control, then the verdict.
        async fn refused_by_the_sandbox_alone(&self) {
            assert!(
                git_launch().unwrap().sandbox.is_some(),
                "this host has no exec sandbox, so nothing here can be tested; \
                 on Linux that is a kernel without Landlock"
            );
            let without = self.status(false).await;
            assert!(!without.is_empty(), "with neither guard, nothing ran: the fixture plants nothing");
            assert_eq!(self.status(true).await, Vec::<String>::new(), "ran with the pins off, under the sandbox");
        }
    }

    #[tokio::test]
    async fn a_planted_hook_is_refused_with_the_pins_off() {
        let f = Fixture::new("*.txt filter=evil\n");
        let hooks = f.root().join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("post-index-change");
        std::fs::write(&hook, format!("#!/bin/sh\n{}\n", f.probe("hook-dir"))).unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        f.plant("core.hooksPath", hooks.to_str().unwrap());
        f.plant("hook.x.command", &f.probe("hook-config"));
        f.plant("hook.x.event", "post-index-change");
        f.refused_by_the_sandbox_alone().await;
    }

    #[tokio::test]
    async fn a_planted_filter_is_refused_with_the_pins_off() {
        let f = Fixture::new("*.txt filter=evil\n");
        f.plant("filter.evil.clean", &f.probe("clean"));
        f.plant("filter.evil.required", "true");
        f.refused_by_the_sandbox_alone().await;
    }

    #[tokio::test]
    async fn a_planted_fsmonitor_is_refused_with_the_pins_off() {
        let f = Fixture::new("*.txt filter=evil\n");
        f.plant("core.fsmonitor", &f.probe("fsmonitor"));
        f.refused_by_the_sandbox_alone().await;
    }

    /// git-lfs by absolute path, as a trusted install would name it: still
    /// refused, because git starts `git-lfs filter-process` through `sh -c`.
    #[tokio::test]
    async fn git_lfs_is_refused_with_the_pins_off_even_by_absolute_path() {
        let f = Fixture::new("*.txt filter=lfs\n");
        let lfs = f.root().join("git-lfs");
        f.plant("filter.lfs.process", &format!("{} filter-process", lfs.display()));
        f.plant("filter.lfs.required", "true");
        f.refused_by_the_sandbox_alone().await;
    }

    /// The per-call cost, printed: medians of 60 `git rev-parse HEAD` runs,
    /// one process each, then the daemon's whole guarded call (the listing
    /// and the call). Ignored: it measures, it doesn't check.
    /// `cargo test -p farcooler-daemon --lib measure_the_overhead -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "a measurement, not a check"]
    async fn measure_the_overhead() {
        use std::time::{Duration, Instant};
        let cwd = std::env::current_dir().unwrap();
        let median = |mut runs: Vec<Duration>| {
            runs.sort();
            runs[runs.len() / 2]
        };
        let once = |cmd: &mut std::process::Command| {
            let started = Instant::now();
            assert!(cmd.current_dir(&cwd).args(["rev-parse", "HEAD"]).output().unwrap().status.success());
            started.elapsed()
        };
        let launch = git_launch().unwrap();
        let found = crate::git::absolute_git().unwrap();
        type Make<'a> = Box<dyn Fn() -> std::process::Command + 'a>;
        let rows: [(&str, Make); 3] = [
            ("found git, unconfined", Box::new(|| std::process::Command::new(&found))),
            ("real git, unconfined", Box::new(|| std::process::Command::new(&launch.program))),
            ("real git, confined", Box::new(|| launch.command().unwrap())),
        ];
        for (name, make) in &rows {
            let runs: Vec<Duration> = (0..60).map(|_| once(&mut make())).collect();
            println!("{name:32} {:?}", median(runs));
        }
        // As before this change: the git found (on a Mac, the shim),
        // unconfined; then the real git, unconfined and confined.
        for (name, before, sandboxed) in
            [("found git, unconfined", true, false), ("real git, unconfined", false, false), ("real git, confined", false, true)]
        {
            let mut runs = Vec::new();
            for _ in 0..60 {
                crate::git::PROGRAM.with(|p| *p.borrow_mut() = before.then(|| found.clone()));
                UNSANDBOXED.with(|u| u.set(!sandboxed));
                let started = Instant::now();
                crate::git::git(&cwd, &["rev-parse", "HEAD"]).await.unwrap();
                runs.push(started.elapsed());
            }
            crate::git::PROGRAM.with(|p| *p.borrow_mut() = None);
            UNSANDBOXED.with(|u| u.set(false));
            println!("guarded call, {name:22} {:?}", median(runs));
        }
    }

    /// What's on the list: git and its own, never a shell or interpreter,
    /// and never a directory.
    #[test]
    fn no_interpreter_is_ever_allowed() {
        let mut lists = vec![git_launch().unwrap()];
        lists.extend(crate::git::gh_launch());
        let interpreters = [
            "sh", "bash", "zsh", "dash", "ksh", "fish", "csh", "tcsh", "env", "perl", "python", "python3", "ruby",
            "node", "osascript", "lua", "tclsh", "php", "awk", "xcrun",
        ];
        for launch in lists {
            let Some(sandbox) = &launch.sandbox else { continue };
            for path in sandbox.allowed() {
                let name = path.file_name().unwrap().to_string_lossy();
                assert!(!interpreters.contains(&&*name), "{} is on the list", path.display());
                assert!(!name.starts_with("python") && !name.starts_with("perl"), "{} is on the list", path.display());
                assert!(!path.is_dir(), "{} is a directory", path.display());
            }
        }
    }

    /// The mechanism, with nothing of git's in the way: a confined command
    /// can run what's on its list and nothing else.
    #[test]
    fn a_confined_process_runs_only_what_is_on_its_list() {
        let launch = git_launch().unwrap();
        let sandbox = launch.sandbox.as_ref().expect("an exec sandbox on this host");
        let mut git = launch.command().unwrap();
        assert!(git.arg("--version").output().unwrap().status.success(), "git itself runs");

        let mut shell = std::process::Command::new("/bin/sh");
        sandbox.confine(&mut shell).unwrap();
        let refused = shell.args(["-c", "true"]).output();
        assert!(refused.is_err() || !refused.unwrap().status.success(), "/bin/sh is refused");

        // And what git starts inherits the list: an alias is a shell command.
        let mut aliased = launch.command().unwrap();
        let out = aliased.args(["-c", "alias.x=!true", "x"]).current_dir("/").output().unwrap();
        assert!(!out.status.success(), "git's own child shell is refused too");
    }
}
