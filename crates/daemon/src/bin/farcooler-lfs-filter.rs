//! `farcooler-lfs-filter`: the Git LFS filter the daemon's own gits use, in
//! place of git-lfs.
//!
//! The daemon's gits run inside an exec allowlist (`git_sandbox` in the
//! daemon) with no shell on it, and git-lfs can't go on it either: git starts
//! `git-lfs filter-process` through `sh -c`, and git-lfs itself execs, with
//! arguments, whatever the repository's own config names
//! (`lfs.extension.<name>.smudge`, `lfs.customtransfer.<name>.path`). With
//! git on the list, `lfs.extension.x.smudge = git config --global …` would
//! be a write to the user's global config. So the daemon pins
//! `filter.lfs.process` to this program instead (`git_lfs` in the daemon),
//! by a bare name with no arguments, which git execs directly.
//!
//! It speaks git's long-running filter protocol (`gitattributes(5)`, "Long
//! Running Filter Process"), one process per git, and:
//!
//! - **smudge** (checkout): an LFS pointer whose object is already in this
//!   repository's local store (`<common dir>/lfs/objects/aa/bb/<oid>`, where
//!   git-lfs keeps it) becomes the object's content, once the content's
//!   SHA-256 is checked against the pointer's oid. Anything else (an object
//!   that isn't here, a pointer naming extensions, content that isn't a
//!   pointer) is refused per file, which leaves git to write the blob as it
//!   is: the pointer.
//! - **clean** (status and diff): content becomes its pointer, so a hydrated
//!   file compares equal to the pointer the index holds. A pointer stays a
//!   pointer. Nothing is written to the store: the daemon never commits.
//!
//! It execs nothing, reads no config, and never touches the network. The
//! store's location comes from `GIT_DIR`, which git sets for every filter, and
//! the repository's `commondir` file. Both are the agent's to write, so a
//! store can be anything, symlinks included; the hash check is what makes
//! that harmless. A file reaches the worktree only if its SHA-256 is the oid
//! the pointer named, so a pointer can't read a file whose content the agent
//! doesn't already know.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The largest payload one pkt-line carries: 65520 bytes, less the 4-byte
/// length.
const MAX_DATA: usize = 65516;

/// git-lfs's own cutoff: a file this size or larger is never a pointer.
const POINTER_MAX: usize = 1024;

const VERSION: &str = "https://git-lfs.github.com/spec/v1";
/// The pre-1.0 spelling, which git-lfs still reads.
const VERSION_HAWSER: &str = "https://hawser.github.com/spec/v1";

fn main() {
    let stdin = io::stdin().lock();
    let stdout = io::stdout().lock();
    let code = match serve(stdin, stdout, &Store::from_env()) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("farcooler-lfs-filter: {e}");
            1
        }
    };
    std::process::exit(code);
}

/// One pkt-line read: data, or a flush.
#[derive(Debug, PartialEq, Eq)]
enum Pkt {
    Data(Vec<u8>),
    Flush,
}

fn read_pkt(r: &mut impl Read) -> io::Result<Option<Pkt>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = std::str::from_utf8(&len)
        .ok()
        .and_then(|s| usize::from_str_radix(s, 16).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "a pkt-line length that isn't hex"))?;
    match len {
        0 => Ok(Some(Pkt::Flush)),
        1..=4 => Err(io::Error::new(io::ErrorKind::InvalidData, "a pkt-line length under 4")),
        _ => {
            let mut data = vec![0u8; len - 4];
            r.read_exact(&mut data)?;
            Ok(Some(Pkt::Data(data)))
        }
    }
}

fn write_pkt(w: &mut impl Write, data: &[u8]) -> io::Result<()> {
    for chunk in data.chunks(MAX_DATA) {
        write!(w, "{:04x}", chunk.len() + 4)?;
        w.write_all(chunk)?;
    }
    Ok(())
}

fn flush(w: &mut impl Write) -> io::Result<()> {
    w.write_all(b"0000")
}

fn write_text(w: &mut impl Write, line: &str) -> io::Result<()> {
    write_pkt(w, format!("{line}\n").as_bytes())
}

/// Text packets up to the next flush, each without its newline. `None` at a
/// clean end of input.
fn read_list(r: &mut impl Read) -> io::Result<Option<Vec<String>>> {
    let mut lines = Vec::new();
    loop {
        match read_pkt(r)? {
            None if lines.is_empty() => return Ok(None),
            None => return Err(io::ErrorKind::UnexpectedEof.into()),
            Some(Pkt::Flush) => return Ok(Some(lines)),
            Some(Pkt::Data(d)) => {
                let text = String::from_utf8_lossy(&d);
                lines.push(text.strip_suffix('\n').unwrap_or(&text).to_string());
            }
        }
    }
}

/// The handshake, then one answer per file until git closes the pipe.
fn serve(mut r: impl Read, mut w: impl Write, store: &Store) -> io::Result<()> {
    let hello = read_list(&mut r)?.unwrap_or_default();
    if !hello.iter().any(|l| l == "git-filter-client") || !hello.iter().any(|l| l == "version=2") {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not git's filter protocol, version 2"));
    }
    write_text(&mut w, "git-filter-server")?;
    write_text(&mut w, "version=2")?;
    flush(&mut w)?;
    w.flush()?;
    let offered = read_list(&mut r)?.unwrap_or_default();
    for cap in ["capability=clean", "capability=smudge"] {
        if offered.iter().any(|l| l == cap) {
            write_text(&mut w, cap)?;
        }
    }
    flush(&mut w)?;
    w.flush()?;

    while let Some(request) = read_list(&mut r)? {
        let command = request.iter().find_map(|l| l.strip_prefix("command=")).unwrap_or("").to_string();
        match command.as_str() {
            "clean" => clean(&mut r, &mut w)?,
            "smudge" => smudge(&mut r, &mut w, store)?,
            _ => {
                drain(&mut r)?;
                refuse(&mut w)?;
            }
        }
        w.flush()?;
    }
    Ok(())
}

/// Read the content packets of one file, giving each to `each`.
fn content(r: &mut impl Read, mut each: impl FnMut(&[u8])) -> io::Result<()> {
    loop {
        match read_pkt(r)? {
            Some(Pkt::Data(d)) => each(&d),
            Some(Pkt::Flush) => return Ok(()),
            None => return Err(io::ErrorKind::UnexpectedEof.into()),
        }
    }
}

fn drain(r: &mut impl Read) -> io::Result<()> {
    content(r, |_| {})
}

/// This file isn't converted: git keeps what it had, without a word.
fn refuse(w: &mut impl Write) -> io::Result<()> {
    write_text(w, "status=error")?;
    flush(w)
}

/// The first [`POINTER_MAX`] bytes of a file, and whether there was more.
#[derive(Default)]
struct Head {
    bytes: Vec<u8>,
    longer: bool,
}

impl Head {
    fn take(&mut self, chunk: &[u8]) {
        let room = POINTER_MAX.saturating_sub(self.bytes.len());
        self.bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
        self.longer |= chunk.len() > room;
    }

    /// The pointer this file is, if it is one.
    fn pointer(&self) -> Option<Pointer> {
        if self.longer { None } else { Pointer::parse(&self.bytes) }
    }
}

fn clean(r: &mut impl Read, w: &mut impl Write) -> io::Result<()> {
    let mut head = Head::default();
    let mut hash = Sha256::new();
    let mut size = 0u64;
    content(r, |chunk| {
        head.take(chunk);
        hash.update(chunk);
        size += chunk.len() as u64;
    })?;
    // Already a pointer (a file nobody hydrated), or empty: as it is, which
    // is what git-lfs answers for both.
    if size == 0 || head.pointer().is_some() {
        return refuse(w);
    }
    let pointer = Pointer { oid: hex(&hash.finalize()), size, extensions: false };
    write_text(w, "status=success")?;
    flush(w)?;
    write_pkt(w, pointer.text().as_bytes())?;
    flush(w)?;
    flush(w)
}

fn smudge(r: &mut impl Read, w: &mut impl Write, store: &Store) -> io::Result<()> {
    let mut head = Head::default();
    content(r, |chunk| head.take(chunk))?;
    let Some(pointer) = head.pointer().filter(|p| !p.extensions) else { return refuse(w) };
    let Some(mut object) = store.open(&pointer) else { return refuse(w) };

    write_text(w, "status=success")?;
    flush(w)?;
    let mut hash = Sha256::new();
    let mut sent = 0u64;
    let mut buf = vec![0u8; MAX_DATA];
    loop {
        let n = match object.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        hash.update(&buf[..n]);
        sent += n as u64;
        write_pkt(w, &buf[..n])?;
    }
    flush(w)?;
    // The verdict after the content: git keeps what it streamed only on an
    // empty list, and on `status=error` discards it and writes the blob as it
    // was. Checked here rather than in a pass before, so a large object is
    // read once.
    if sent == pointer.size && hex(&hash.finalize()) == pointer.oid {
        flush(w)
    } else {
        refuse(w)
    }
}

/// An LFS pointer: `version`, then `oid sha256:<64 hex>` and `size <n>`, and
/// any `ext-<n>-<name>` lines, each `key value\n` in key order.
#[derive(Debug, PartialEq, Eq)]
struct Pointer {
    oid: String,
    size: u64,
    /// Names extensions, whose smudge is a program from config: never
    /// hydrated.
    extensions: bool,
}

impl Pointer {
    fn parse(bytes: &[u8]) -> Option<Pointer> {
        let text = std::str::from_utf8(bytes).ok()?;
        let text = text.strip_suffix('\n')?;
        let mut lines = text.split('\n');
        let version = lines.next()?.strip_prefix("version ")?;
        if version != VERSION && version != VERSION_HAWSER {
            return None;
        }
        let (mut oid, mut size, mut extensions, mut last) = (None, None, false, "");
        for line in lines {
            let (key, value) = line.split_once(' ')?;
            if key <= last {
                return None;
            }
            last = key;
            match key {
                "oid" => {
                    let hex = value.strip_prefix("sha256:")?;
                    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
                        return None;
                    }
                    oid = Some(hex.to_string());
                }
                "size" => {
                    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                        return None;
                    }
                    size = Some(value.parse().ok()?);
                }
                k if k.starts_with("ext-") => extensions = true,
                _ => return None,
            }
        }
        Some(Pointer { oid: oid?, size: size?, extensions })
    }

    fn text(&self) -> String {
        format!("version {VERSION}\noid sha256:{}\nsize {}\n", self.oid, self.size)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The local LFS object store: `<common dir>/lfs/objects`.
struct Store {
    objects: Option<PathBuf>,
}

impl Store {
    /// From `GIT_DIR`, which git sets for the filters it starts, and the
    /// `commondir` file a linked worktree's git directory holds.
    fn from_env() -> Store {
        let objects = std::env::var_os("GIT_DIR").map(|dir| {
            let dir = PathBuf::from(dir);
            let dir = if dir.is_absolute() {
                dir
            } else {
                std::env::current_dir().map(|cwd| cwd.join(&dir)).unwrap_or(dir)
            };
            common_dir(&dir).join("lfs").join("objects")
        });
        Store { objects }
    }

    /// The object `pointer` names: a regular file of exactly its size, opened
    /// once, so what is hashed is what is sent. Never blocks on a FIFO.
    fn open(&self, pointer: &Pointer) -> Option<std::fs::File> {
        use std::os::unix::fs::OpenOptionsExt;
        let path = self.objects.as_ref()?.join(&pointer.oid[0..2]).join(&pointer.oid[2..4]).join(&pointer.oid);
        let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path).ok()?;
        let meta = file.metadata().ok()?;
        (meta.is_file() && meta.len() == pointer.size).then_some(file)
    }
}

/// `git_dir`'s common directory: where its `commondir` file points, or itself.
fn common_dir(git_dir: &Path) -> PathBuf {
    match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(text) => {
            let named = Path::new(text.trim_end_matches(['\n', '\r']));
            if named.is_absolute() { named.to_path_buf() } else { git_dir.join(named) }
        }
        Err(_) => git_dir.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OID: &str = "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393";

    fn pointer_text(oid: &str, size: u64) -> String {
        format!("version {VERSION}\noid sha256:{oid}\nsize {size}\n")
    }

    #[test]
    fn a_pointer_is_read_strictly() {
        let p = Pointer::parse(pointer_text(OID, 12345).as_bytes()).unwrap();
        assert_eq!(p, Pointer { oid: OID.into(), size: 12345, extensions: false });
        assert_eq!(p.text(), pointer_text(OID, 12345));
        let ext = format!("version {VERSION}\next-0-foo sha256:{OID}\noid sha256:{OID}\nsize 1\n");
        assert!(Pointer::parse(ext.as_bytes()).unwrap().extensions);
        let hawser = format!("version {VERSION_HAWSER}\noid sha256:{OID}\nsize 1\n");
        assert!(Pointer::parse(hawser.as_bytes()).is_some());

        for bad in [
            format!("version {VERSION}\nsize 1\noid sha256:{OID}\n"), // out of order
            format!("version {VERSION}\noid sha256:{}\nsize 1\n", &OID[1..]),
            format!("version {VERSION}\noid sha256:{}\nsize 1\n", OID.to_uppercase()),
            format!("version {VERSION}\noid sha256:{OID}\nsize -1\n"),
            format!("version {VERSION}\noid sha256:{OID}\nsize 1"), // no newline
            format!("version {VERSION}\noid sha256:{OID}\n"),
            format!("version other\noid sha256:{OID}\nsize 1\n"),
            format!("version {VERSION}\noid sha256:{OID}\nsize 1\nzzz x\n"),
            "hello\n".to_string(),
        ] {
            assert_eq!(Pointer::parse(bad.as_bytes()), None, "{bad:?}");
        }
    }

    fn pkt(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        write_pkt(&mut out, data).unwrap();
        out
    }

    /// What git sends: the handshake, then one request per `(command, content)`.
    fn session(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut s = Vec::new();
        for line in ["git-filter-client\n", "version=2\n"] {
            s.extend(pkt(line.as_bytes()));
        }
        s.extend(b"0000");
        for line in ["capability=clean\n", "capability=smudge\n", "capability=delay\n"] {
            s.extend(pkt(line.as_bytes()));
        }
        s.extend(b"0000");
        for (command, content) in files {
            s.extend(pkt(format!("command={command}\n").as_bytes()));
            s.extend(pkt(b"pathname=a.bin\n"));
            s.extend(b"0000");
            for chunk in content.chunks(MAX_DATA) {
                s.extend(pkt(chunk));
            }
            s.extend(b"0000");
        }
        s
    }

    /// Every packet the filter wrote, flushes as `None`.
    fn replies(out: &[u8]) -> Vec<Option<Vec<u8>>> {
        let mut r = out;
        let mut all = Vec::new();
        while let Some(p) = read_pkt(&mut r).unwrap() {
            all.push(match p {
                Pkt::Data(d) => Some(d),
                Pkt::Flush => None,
            });
        }
        all
    }

    fn text(s: &str) -> Option<Vec<u8>> {
        Some(s.as_bytes().to_vec())
    }

    fn handshake() -> Vec<Option<Vec<u8>>> {
        vec![
            text("git-filter-server\n"),
            text("version=2\n"),
            None,
            text("capability=clean\n"),
            text("capability=smudge\n"),
            None,
        ]
    }

    #[test]
    fn clean_turns_content_into_its_pointer_and_leaves_a_pointer_alone() {
        let content = vec![7u8; 100_000];
        let oid = hex(&Sha256::digest(&content));
        let pointer = pointer_text(OID, 3);
        let mut out = Vec::new();
        let store = Store { objects: None };
        serve(&session(&[("clean", &content), ("clean", pointer.as_bytes()), ("clean", b"")])[..], &mut out, &store)
            .unwrap();
        let mut want = handshake();
        want.extend([text("status=success\n"), None, text(&pointer_text(&oid, 100_000)), None, None]);
        want.extend([text("status=error\n"), None]);
        want.extend([text("status=error\n"), None]);
        assert_eq!(replies(&out), want);
    }

    #[test]
    fn smudge_hydrates_only_an_object_whose_hash_is_its_oid() {
        let dir = tempfile::tempdir().unwrap();
        let objects = dir.path().join("lfs/objects");
        let put = |oid: &str, content: &[u8]| {
            let at = objects.join(&oid[0..2]).join(&oid[2..4]);
            std::fs::create_dir_all(&at).unwrap();
            std::fs::write(at.join(oid), content).unwrap();
        };
        let content = vec![9u8; 70_000];
        let oid = hex(&Sha256::digest(&content));
        put(&oid, &content);
        // An object whose bytes aren't its name: the agent's store pointing at
        // a file whose content it doesn't know.
        let liar = hex(&Sha256::digest(b"what the pointer promised"));
        put(&liar, b"a secret of the same size!");
        let store = Store { objects: Some(objects.clone()) };

        let good = pointer_text(&oid, 70_000);
        let lying = pointer_text(&liar, 26);
        let missing = pointer_text(OID, 5);
        let wrong_size = pointer_text(&oid, 69_999);
        let ext = format!("version {VERSION}\next-0-x sha256:{oid}\noid sha256:{oid}\nsize 70000\n");
        let files: [(&str, &[u8]); 6] = [
            ("smudge", good.as_bytes()),
            ("smudge", lying.as_bytes()),
            ("smudge", missing.as_bytes()),
            ("smudge", wrong_size.as_bytes()),
            ("smudge", ext.as_bytes()),
            ("smudge", b"not a pointer\n"),
        ];
        let mut out = Vec::new();
        serve(&session(&files)[..], &mut out, &store).unwrap();

        let mut want = handshake();
        want.extend([text("status=success\n"), None, Some(content[..MAX_DATA].to_vec())]);
        want.extend([Some(content[MAX_DATA..].to_vec()), None, None]);
        // Streamed, then disowned: git keeps the pointer.
        want.extend([text("status=success\n"), None, text("a secret of the same size!"), None]);
        want.extend([text("status=error\n"), None]);
        for _ in 0..4 {
            want.extend([text("status=error\n"), None]);
        }
        assert_eq!(replies(&out), want);
    }

    #[test]
    fn a_linked_worktree_finds_the_store_through_commondir() {
        let dir = tempfile::tempdir().unwrap();
        let wt = dir.path().join(".git/worktrees/w");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join("commondir"), "../..\n").unwrap();
        assert_eq!(common_dir(&wt), wt.join("../.."));
        assert_eq!(common_dir(&dir.path().join(".git")), dir.path().join(".git"));
    }

    #[test]
    fn a_stranger_on_the_pipe_is_refused() {
        let mut s = pkt(b"hello\n");
        s.extend(b"0000");
        assert!(serve(&s[..], Vec::new(), &Store { objects: None }).is_err());
    }
}
