//! Every function the apps can call is guarded, and stays that way.
//!
//! The guard only protects the functions that call it, and nothing in the
//! compiler notices a new export that forgot to. So this reads the source of
//! every crate in the workspace and finds every exported function — anything
//! `#[no_mangle]`, and any `extern "C"` or `extern "system"` function with a
//! body — and fails unless that function's WHOLE body is one call to a
//! `guarded…` function. Not a guard somewhere inside it: a line before the
//! guard runs unguarded, and so does one after it.
//!
//! A source scan rather than a symbol scan, because the symbols say a function
//! exists and nothing about what it does, and because it runs on every
//! platform `cargo test` does without a built archive to read.
//!
//! The rule is by name: the guard is `guarded`, or a helper whose name starts
//! with `guarded` and is itself built on `farcooler_ffi_guard::caught` (the
//! terminal's `guarded_handle`, which repairs the emulator after a panic). Name
//! a helper that way only if it really is one.

use std::path::{Path, PathBuf};

/// One exported function, and what is wrong with it if anything.
#[derive(Debug)]
struct Export {
    file: PathBuf,
    name: String,
    problem: Option<&'static str>,
}

/// The source with every comment, string and char literal blanked to spaces.
///
/// Lengths are kept, so an index into this is an index into the original. The
/// quotes themselves stay, which is what lets `extern "C" fn` still be found:
/// it reads `extern " " fn` here. Without this, a `{` inside a string or a
/// `fn` in a doc comment would be read as code.
fn code_only(source: &str) -> Vec<char> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = chars.clone();
    let mut i = 0;
    let blank = |out: &mut Vec<char>, from: usize, to: usize| {
        for c in out.iter_mut().take(to).skip(from) {
            if *c != '\n' {
                *c = ' ';
            }
        }
    };
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            let end = chars[i..].iter().position(|&c| c == '\n').map_or(chars.len(), |p| i + p);
            blank(&mut out, i, end);
            i = end;
        } else if c == '/' && next == Some('*') {
            let mut depth = 0;
            let mut j = i;
            while j < chars.len() {
                if chars[j] == '/' && chars.get(j + 1) == Some(&'*') {
                    depth += 1;
                    j += 2;
                } else if chars[j] == '*' && chars.get(j + 1) == Some(&'/') {
                    depth -= 1;
                    j += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    j += 1;
                }
            }
            blank(&mut out, i, j);
            i = j;
        } else if c == 'r'
            && (i == 0 || !ident(chars[i - 1]) || chars[i - 1] == 'b')
            && matches!(next, Some('"') | Some('#'))
        {
            // A raw string: r"…", r#"…"#, and so on.
            let hashes = chars[i + 1..].iter().take_while(|&&c| c == '#').count();
            let open = i + 1 + hashes;
            if chars.get(open) != Some(&'"') {
                i += 1;
                continue;
            }
            let closer: String = std::iter::once('"').chain(std::iter::repeat_n('#', hashes)).collect();
            let rest: String = chars[open + 1..].iter().collect();
            let len = rest.find(&closer).map_or(rest.len(), |p| rest[..p].chars().count());
            blank(&mut out, open + 1, open + 1 + len);
            i = open + 1 + len + closer.chars().count();
        } else if c == '"' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '"' {
                j += if chars[j] == '\\' { 2 } else { 1 };
            }
            blank(&mut out, i + 1, j);
            i = j + 1;
        } else if c == '\'' {
            // A char literal, or a lifetime. `'x'` and `'\…'` are literals;
            // `'a` with no closing quote two along is a lifetime.
            if next == Some('\\') {
                let j = chars[i + 2..].iter().position(|&c| c == '\'').map_or(chars.len(), |p| i + 2 + p);
                blank(&mut out, i + 1, j);
                i = j + 1;
            } else if chars.get(i + 2) == Some(&'\'') {
                blank(&mut out, i + 1, i + 2);
                i += 3;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

/// The index just past the bracket that closes the one at `open`.
fn matching(code: &[char], open: usize) -> Option<usize> {
    let (o, c) = (code[open], match code[open] {
        '(' => ')',
        '{' => '}',
        _ => return None,
    });
    let mut depth = 0;
    for (k, &ch) in code.iter().enumerate().skip(open) {
        if ch == o {
            depth += 1;
        } else if ch == c {
            depth -= 1;
            if depth == 0 {
                return Some(k + 1);
            }
        }
    }
    None
}

fn find(code: &[char], from: usize, needle: &str) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    (from..code.len().saturating_sub(needle.len() - 1)).find(|&k| code[k..k + needle.len()] == needle[..])
}

/// Every exported function in one file's source.
fn exports_in(file: &Path, source: &str) -> Vec<Export> {
    let code = code_only(source);
    let ident = |c: char| c.is_alphanumeric() || c == '_';

    // Where each exported function's `fn` keyword sits.
    let mut starts = Vec::new();
    let mut k = 0;
    while let Some(at) = find(&code, k, "no_mangle") {
        if let Some(f) = find(&code, at, "fn ") {
            starts.push(f);
        }
        k = at + 1;
    }
    let mut k = 0;
    while let Some(at) = find(&code, k, "extern \"") {
        // `extern "C" fn` with the ABI blanked to spaces. An `extern "C" {`
        // block declares imports, which are not ours to guard.
        let close = find(&code, at + 8, "\"").unwrap_or(code.len());
        let after: String = code[close + 1..].iter().take(4).collect();
        if after == " fn " {
            starts.push(close + 2);
        }
        k = at + 1;
    }
    starts.sort();
    starts.dedup();

    let mut found = Vec::new();
    for f in starts {
        let name: String = code[f + 3..].iter().skip_while(|c| c.is_whitespace()).take_while(|&&c| ident(c)).collect();
        let Some(paren) = find(&code, f, "(") else { continue };
        let Some(after_params) = matching(&code, paren) else { continue };
        // The body is the first `{` after the parameters; a `;` first means a
        // declaration with no body.
        let Some(brace) = (after_params..code.len()).find(|&k| code[k] == '{' || code[k] == ';') else { continue };
        if code[brace] == ';' {
            continue;
        }
        let Some(end) = matching(&code, brace) else { continue };
        let body: String = code[brace + 1..end - 1].iter().collect();
        found.push(Export { file: file.to_path_buf(), name, problem: problem_with(&body) });
    }
    found
}

/// Why a body is not one guarded call, or `None` when it is.
fn problem_with(body: &str) -> Option<&'static str> {
    let body = body.trim();
    let chars: Vec<char> = body.chars().collect();
    // The callee: a path like `farcooler_ffi_guard::guarded` or `guarded_handle`.
    let path: String = chars.iter().take_while(|&&c| c.is_alphanumeric() || c == '_' || c == ':').collect();
    let callee = path.rsplit("::").next().unwrap_or("");
    if !callee.starts_with("guarded") {
        return Some("its body does not start with a guarded call");
    }
    if chars.get(path.chars().count()) != Some(&'(') {
        return Some("its body does not start with a guarded call");
    }
    let Some(end) = matching(&chars, path.chars().count()) else {
        return Some("the guarded call is not closed");
    };
    let rest: String = chars[end..].iter().collect();
    match rest.trim() {
        "" | ";" => None,
        _ => Some("code after the guarded call runs unguarded"),
    }
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every exported function in every crate's `src`.
///
/// `src` only: a test or an example can export what it likes, because no app
/// links it.
fn workspace_exports() -> Vec<Export> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&crates).expect("the crates directory").flatten() {
        rust_files(&entry.path().join("src"), &mut files);
    }
    files.sort();
    let mut all = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(&file).expect("readable source");
        all.extend(exports_in(&file, &source));
    }
    all
}

#[test]
fn every_exported_function_is_one_guarded_call() {
    let exports = workspace_exports();

    // A scan that silently stopped finding anything would pass forever. These
    // are floors, not counts, so adding an export never breaks this — only
    // losing the ability to see them does.
    let from = |suffix: &str| {
        exports.iter().filter(|e| e.file.ends_with(suffix)).count()
    };
    assert!(from("vt/src/ffi.rs") >= 23, "the terminal's exports went missing");
    assert!(from("review/src/ffi.rs") >= 3, "review's exports went missing");
    assert!(from("client/src/ffi.rs") >= 22, "the client's exports went missing");
    assert!(from("android/src/lib.rs") >= 39, "the JNI shim's exports went missing");

    let unguarded: Vec<String> = exports
        .iter()
        .filter_map(|e| e.problem.map(|p| format!("{} in {}: {p}", e.name, e.file.display())))
        .collect();
    assert!(unguarded.is_empty(), "unguarded exports:\n{}", unguarded.join("\n"));
}

/// The scanner itself, against exports it must refuse and ones it must accept.
#[test]
fn the_scan_catches_each_way_an_export_can_be_unguarded() {
    let check = |source: &str| -> Vec<(String, Option<&'static str>)> {
        exports_in(Path::new("x.rs"), source).into_iter().map(|e| (e.name, e.problem)).collect()
    };

    let bare = check("#[unsafe(no_mangle)]\npub extern \"C\" fn a() -> u8 { 1 }");
    assert_eq!(bare.len(), 1);
    assert!(bare[0].1.is_some(), "a bare body must be refused");

    let before = check("#[no_mangle]\npub unsafe extern \"C\" fn b(p: *mut u8) { let x = 1; guarded((), || {}) }");
    assert!(before[0].1.is_some(), "a statement before the guard must be refused");

    let after = check("#[unsafe(no_mangle)]\npub extern \"C\" fn c() { guarded((), || {}); other() }");
    assert_eq!(after[0].1, Some("code after the guarded call runs unguarded"));

    // `extern "system"` with no attribute at all is still something C can call
    // through a pointer.
    let jni = check("pub extern \"system\" fn Java_x(env: JNIEnv) -> jint { 0 }");
    assert_eq!(jni.len(), 1, "an extern fn is an export even without no_mangle");
    assert!(jni[0].1.is_some());

    let fine = check(
        "#[unsafe(no_mangle)]\n// a comment with fn and { in it\npub unsafe extern \"C\" fn d<'a>(p: *const u8) -> bool {\n    farcooler_ffi_guard::guarded(false, || { let s = \"}\"; let c = '{'; s.len() > 0 && c == '{' })\n}",
    );
    assert_eq!(fine.len(), 1);
    assert_eq!(fine[0].1, None, "a body that is one guarded call is accepted");

    let helper = check("#[unsafe(no_mangle)]\npub unsafe extern \"C\" fn e(h: *mut u8) -> u64 { unsafe { guarded_handle(h, 0, |h| h.n) } }");
    assert!(helper[0].1.is_some(), "an `unsafe {{` wrapper hides what is inside it; write the guard first");

    // An import block is not an export.
    assert!(check("unsafe extern \"C\" {\n    fn imported(x: i32) -> i32;\n}").is_empty());
}
