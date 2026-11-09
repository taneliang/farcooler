#!/usr/bin/env python3
"""Feature logic that reads runner data lives in crates/client and reaches the apps as a view
model (ruling R-112, ov-1015). This keeps it there.

A native app owns layout, gestures, platform services, text input and local view state. It does
not own a string about runner data, a mapping from a wire word to copy, or a second copy of a
screen's logic. Four rules, each run against the user-facing sources of the Mac and iPhone
(apps/shared/AgentKit, apps/macos, apps/ios), Android (apps/android/.../main) and the web page
(services/web/src/client):

  1. rust-copy    A string the Rust client or the shared usage words produce (three words or
                  more) is written again in an app. The app should show what Rust sends.
  2. wire-word    An app switches on a word a Rust `*_word` function returns (`case "running"`,
                  `"running" ->`, `=== 'running'`). The apps map wire words to copy no more:
                  ov-924 removed those maps for workflow runs.
  3. twin         A new file in Android's model/ or notify/ folder or in the web client shares
                  its base name with a logic file in AgentKit, or says in its header that it is a
                  twin, port, mirror or transcription of AgentKit.
  4. ratchet      The count of user-facing strings in a logic file (AgentKit files that import no
                  UI framework, Android model/ and notify/, the web client minus its views) may
                  fall but not rise. A new file starts at zero.

Today's violations are recorded in scripts/shared-core-baseline.json, so only new ones fail.
The baseline may shrink: run `--update-baseline` after removing duplication, and review the
diff; a run that would grow it is refused unless `--allow-growth` says it is deliberate.

A line that is copy about the platform itself, not about runner data, says
`platform copy: <reason>` in a comment on the line; a bare `platform copy` does not exempt.

  ./scripts/shared-core-lint.py                    scan the tree; exit 1 on any new violation
  ./scripts/shared-core-lint.py --self-test        plant each violation and watch it go red
  ./scripts/shared-core-lint.py --update-baseline  record today's violations (shrinking only)
"""

import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
BASELINE = ROOT / "scripts" / "shared-core-baseline.json"

RUST_COPY_DIRS = ["crates/client/src"]
RUST_COPY_FILES = ["crates/core/src/usage_words.rs", "crates/core/src/phrase.rs"]
RUST_WORD_DIRS = ["crates/client/src"]
NATIVE_ROOTS = ["apps/shared/AgentKit/Sources", "apps/macos/Sources", "apps/ios",
                "apps/android/app/src/main", "services/web/src/client"]
NATIVE_SUFFIXES = {".swift", ".kt", ".ts"}
SKIP_PARTS = {"Tests", "FarCoolerUITests", "test", "androidTest", ".build", "build", "node_modules", "testdata"}
UI_IMPORTS = re.compile(r"^\s*import\s+(SwiftUI|AppKit|UIKit)\b|^\s*import\s+androidx\.compose|^\s*import\s+android\.(view|widget)", re.M)
WEB_VIEW = re.compile(r"-(view|column|list|tab)\.ts$")

_PLATFORM_OK = re.compile(r"platform copy:\s*\S")
_LITERAL = re.compile(r'"((?:[^"\\\n]|\\.)*)"' + r"|'((?:[^'\\\n]|\\.)*)'")
_RUST_LITERAL = re.compile(r'"((?:[^"\\\n]|\\.)*)"')
_TWIN_HEADER = re.compile(r"\b(twin|port|mirror|mirrors|transcription|transcribes|transcribed)\b[^\n]{0,60}\bAgentKit\b"
                          r"|\bAgentKit\b[^\n]{0,60}\b(twin|port|mirror|mirrors|transcription|transcribes)\b", re.I)


def is_comment(line: str) -> bool:
    return line.lstrip().startswith(("//", "*", "/*", "///", "<!--"))


def words(text: str) -> list[str]:
    return [w for w in text.split() if any(c.isalpha() for c in w)]


# ---- what Rust says --------------------------------------------------------------------------

def rust_literals(text: str) -> set[str]:
    """Strings of three words or more in the non-test code of a Rust file."""
    cut = text.find("#[cfg(test)]")
    if cut >= 0:
        text = text[:cut]
    found = set()
    for line in text.splitlines():
        if is_comment(line) or line.lstrip().startswith("#["):
            continue
        for match in _RUST_LITERAL.finditer(line):
            literal = match.group(1)
            if "{" in literal or "\\" in literal or len(words(literal)) < 3:
                continue
            found.add(literal)
    return found


def rust_wire_words(text: str) -> set[str]:
    """What each `fn …_word` returns, from its body."""
    found = set()
    cut = text.find("#[cfg(test)]")
    if cut >= 0:
        text = text[:cut]
    for match in re.finditer(r"\bfn\s+\w+_word\s*\([^)]*\)[^{]*\{", text):
        depth, i = 1, match.end()
        while i < len(text) and depth:
            depth += {"{": 1, "}": -1}.get(text[i], 0)
            i += 1
        for literal in _RUST_LITERAL.finditer(text[match.end():i]):
            word = literal.group(1)
            if re.fullmatch(r"[a-z][a-z0-9_\-]*", word):
                found.add(word)
    return found


# ---- what the apps say -----------------------------------------------------------------------

def literals(line: str) -> list[str]:
    return [m.group(1) if m.group(1) is not None else m.group(2) for m in _LITERAL.finditer(line)]


def user_facing(literal: str) -> bool:
    """A string with two words or more, sentence-like: not an identifier, a path or a format key."""
    if "/" in literal and " " not in literal:
        return False
    parts = words(literal)
    return len(parts) >= 2 and " " in literal and bool(re.match(r"[A-Za-z‘’\"'(]", literal)) \
        and bool(re.search(r"[a-z]{2}", literal))


def rule_rust_copy(rust: set[str], native: dict[str, str]) -> set[str]:
    out = set()
    for path, text in native.items():
        for line in text.splitlines():
            if is_comment(line) or _PLATFORM_OK.search(line):
                continue
            for literal in literals(line):
                if literal in rust:
                    out.add(f"{path}::{literal}")
    return out


_SWITCH = [
    re.compile(r'\bcase\s+"([a-z][a-z0-9_\-]*)"'),                      # Swift
    re.compile(r'(?<![\w.])"([a-z][a-z0-9_\-]*)"\s*(?:,\s*"[^"]*"\s*)*->'),  # Kotlin when
    re.compile(r"===?\s*'([a-z][a-z0-9_\-]*)'|'([a-z][a-z0-9_\-]*)'\s*===?"),  # TypeScript
    re.compile(r"\bcase\s+'([a-z][a-z0-9_\-]*)'"),
]


def rule_wire_word(word_set: set[str], native: dict[str, str]) -> set[str]:
    out = set()
    for path, text in native.items():
        for line in text.splitlines():
            if is_comment(line) or _PLATFORM_OK.search(line):
                continue
            for pattern in _SWITCH:
                for match in pattern.finditer(line):
                    word = next(g for g in match.groups() if g)
                    if word in word_set:
                        out.add(f"{path}::{word}")
    return out


def logic_files(native: dict[str, str]) -> dict[str, str]:
    """The files the twin rule and the ratchet look at."""
    out = {}
    for path, text in native.items():
        if path.startswith("apps/shared/AgentKit/Sources") and path.endswith(".swift"):
            if not UI_IMPORTS.search(text):
                out[path] = text
        elif path.startswith("apps/android/app/src/main") and path.endswith(".kt"):
            if "/model/" in path or "/notify/" in path:
                out[path] = text
        elif path.startswith("services/web/src/client/") and path.endswith(".ts"):
            if not WEB_VIEW.search(path):
                out[path] = text
    return out


def base(path: str) -> str:
    return re.sub(r"\.(swift|kt|ts)$", "", path.rsplit("/", 1)[-1]).replace("-", "").lower()


def rule_twin(native: dict[str, str]) -> set[str]:
    logic = logic_files(native)
    agentkit = {base(p) for p in logic if p.startswith("apps/shared/AgentKit")}
    out = set()
    for path, text in logic.items():
        if path.startswith("apps/shared/AgentKit"):
            continue
        if base(path) in agentkit or _TWIN_HEADER.search("\n".join(text.splitlines()[:25])):
            out.add(path)
    return out


def counts(native: dict[str, str]) -> dict[str, int]:
    out = {}
    for path, text in logic_files(native).items():
        n = 0
        for line in text.splitlines():
            if is_comment(line) or _PLATFORM_OK.search(line):
                continue
            n += sum(1 for literal in literals(line) if user_facing(literal))
        if n:
            out[path] = n
    return out


# ---- the tree --------------------------------------------------------------------------------

def read_native() -> dict[str, str]:
    out = {}
    for root in NATIVE_ROOTS:
        for path in sorted((ROOT / root).rglob("*")):
            rel = path.relative_to(ROOT)
            if path.suffix in NATIVE_SUFFIXES and not SKIP_PARTS & set(rel.parts) and not path.name.endswith(".d.ts"):
                out[rel.as_posix()] = path.read_text(encoding="utf-8", errors="replace")
    return out


def read_rust(dirs, files=()) -> dict[str, str]:
    out = {}
    for d in dirs:
        for path in sorted((ROOT / d).rglob("*.rs")):
            rel = path.relative_to(ROOT)
            if SKIP_PARTS & set(rel.parts) or "test" in path.name or "tests" in rel.parts:
                continue
            out[rel.as_posix()] = path.read_text(encoding="utf-8")
    for f in files:
        out[f] = (ROOT / f).read_text(encoding="utf-8")
    return out


def measure(rust: dict[str, str], native: dict[str, str]) -> dict:
    lits, wset = set(), set()
    for text in rust.values():
        lits |= rust_literals(text)
        wset |= rust_wire_words(text)
    return {
        "rust-copy": sorted(rule_rust_copy(lits, native)),
        "wire-word": sorted(rule_wire_word(wset, native)),
        "twin": sorted(rule_twin(native)),
        "ratchet": dict(sorted(counts(native).items())),
    }


def compare(now: dict, baseline: dict) -> list[str]:
    """Each way `now` is worse than the baseline."""
    out = []
    for rule in ("rust-copy", "wire-word", "twin"):
        for item in sorted(set(now[rule]) - set(baseline.get(rule, []))):
            out.append(f"{rule}: {item}")
    for path, n in now["ratchet"].items():
        was = baseline.get("ratchet", {}).get(path, 0)
        if n > was:
            out.append(f"ratchet: {path} has {n} user-facing strings, the baseline allows {was}")
    return out


HELP = {
    "rust-copy": "this string is also written in Rust. Show what the shared client sends instead of writing it again.",
    "wire-word": "this switches on a word a Rust `*_word` function returns. Let Rust send the copy or the decision.",
    "twin": "this is a new native copy of a logic file. Put the logic in crates/client and render its view model.",
    "ratchet": "feature copy grew in a logic file. Move it into crates/client, or say `platform copy: <reason>` on a line about the platform itself.",
}


def scan() -> int:
    now = measure(read_rust(RUST_COPY_DIRS, RUST_COPY_FILES) | read_rust(RUST_WORD_DIRS), read_native())
    baseline = json.loads(BASELINE.read_text(encoding="utf-8")) if BASELINE.exists() else {}
    if not baseline:
        print(f"{BASELINE.relative_to(ROOT)} is missing: record today's violations with --update-baseline")
        return 1
    new = compare(now, baseline)
    for line in new:
        print(line)
    if new:
        print("\n" + "\n".join(f"{rule}: {text}" for rule, text in HELP.items()
                               if any(n.startswith(rule + ":") for n in new)))
        print("\nRule R-112: feature logic that reads runner data lives in crates/client as a view model.")
        return 1
    fewer = sum(len(set(baseline.get(r, [])) - set(now[r])) for r in ("rust-copy", "wire-word", "twin")) \
        + sum(1 for p, n in baseline.get("ratchet", {}).items() if now["ratchet"].get(p, 0) < n)
    print("shared-core-lint: nothing new" + (f" ({fewer} baseline entries can shrink: --update-baseline)" if fewer else ""))
    return 0


def update_baseline(allow_growth: bool) -> int:
    now = measure(read_rust(RUST_COPY_DIRS, RUST_COPY_FILES) | read_rust(RUST_WORD_DIRS), read_native())
    if BASELINE.exists() and not allow_growth:
        grew = compare(now, json.loads(BASELINE.read_text(encoding="utf-8")))
        if grew:
            print("refusing to record new violations in the baseline (pass --allow-growth if that is deliberate):")
            print("\n".join(grew))
            return 1
    BASELINE.write_text(json.dumps(now, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"recorded {len(now['rust-copy'])} rust-copy, {len(now['wire-word'])} wire-word, "
          f"{len(now['twin'])} twin, {len(now['ratchet'])} ratchet files")
    return 0


def self_test() -> int:
    """Plant one violation per rule in an in-memory tree: each must be caught, and the clean tree must pass."""
    rust = {"crates/client/src/x.rs": (
        'pub fn status_word(raw: i32) -> &\'static str { match raw { 1 => "awaiting_review", _ => "idle" } }\n'
        'fn copy() -> &\'static str { "Nothing needs you right now" }\n'
        '#[cfg(test)]\nmod tests { fn t() { let _ = "Only a test says this"; } }\n')}
    clean = {
        "apps/shared/AgentKit/Sources/AgentKit/Card.swift": 'import Foundation\nlet title = "Open a ticket"  // one string\n',
        "apps/android/app/src/main/java/com/farcooler/ui/Screen.kt": 'import androidx.compose.ui.Modifier\nfun f() = "Nothing needs you right now"  // platform copy: the sample in the preview\n',
        "services/web/src/client/other.ts": "export const x = 1\n",
    }
    planted = {
        "rust-copy": ("apps/android/app/src/main/java/com/farcooler/ui/Screen.kt",
                      'import androidx.compose.ui.Modifier\nfun f() = "Nothing needs you right now"\n'),
        "wire-word-swift": ("apps/macos/Sources/Farcooler/Row.swift", 'switch s { case "awaiting_review": break\ndefault: break }\n'),
        "wire-word-kotlin": ("apps/android/app/src/main/java/com/farcooler/ui/Row.kt", 'fun f(s: String) = when (s) { "awaiting_review" -> 1\nelse -> 0 }\n'),
        "wire-word-ts": ("services/web/src/client/row.ts", "export const f = (s: string) => s === 'awaiting_review'\n"),
        "twin-name": ("apps/android/app/src/main/java/com/farcooler/model/Card.kt", "package x\n"),
        "twin-header": ("services/web/src/client/thing.ts", "// The web port of AgentKit's Thing.swift\nexport const a = 1\n"),
        "ratchet-android": ("apps/android/app/src/main/java/com/farcooler/model/Words.kt", 'val a = "Something went wrong here"\n'),
        "ratchet-web": ("services/web/src/client/words.ts", "export const a = 'Something went wrong here'\n"),
        "ratchet-agentkit": ("apps/shared/AgentKit/Sources/AgentKit/Words.swift", 'let a = "Something went wrong here"\n'),
    }
    expect = {"rust-copy": "rust-copy", "wire-word-swift": "wire-word", "wire-word-kotlin": "wire-word", "wire-word-ts": "wire-word",
              "twin-name": "twin", "twin-header": "twin", "ratchet-android": "ratchet", "ratchet-web": "ratchet",
              "ratchet-agentkit": "ratchet"}
    failed = 0
    base_state = measure(rust, clean)
    if compare(base_state, base_state):
        print("self-test: the clean tree is not clean against itself")
        failed += 1
    # The clean tree has no hit of its own: its one Rust-copy line says `platform copy`.
    if base_state["rust-copy"] or base_state["wire-word"] or base_state["twin"]:
        print(f"self-test: the clean tree has hits: {base_state}")
        failed += 1
    for name, (path, text) in planted.items():
        tree = dict(clean); tree[path] = text
        hit = compare(measure(rust, tree), base_state)
        if not any(h.startswith(expect[name] + ":") for h in hit):
            print(f"self-test: missed {name}: {hit}")
            failed += 1
    # A bare `platform copy` does not exempt; one with a reason does (already used in the clean tree).
    bare = dict(clean); bare["apps/android/app/src/main/java/com/farcooler/ui/Screen.kt"] = \
        'fun f() = "Nothing needs you right now"  // platform copy\n'
    if not compare(measure(rust, bare), base_state):
        print("self-test: a bare `platform copy` exempted a line")
        failed += 1
    # A string only a Rust test says, and a word no `*_word` function returns, are not violations.
    quiet = dict(clean)
    quiet["apps/macos/Sources/Farcooler/Q.swift"] = 'let a = "Only a test says this"\nswitch s { case "idle_not_returned": break\ndefault: break }\n'
    if compare(measure(rust, quiet), base_state):
        print(f"self-test: a non-violation was caught: {compare(measure(rust, quiet), base_state)}")
        failed += 1
    # The ratchet lets a count fall and holds it where it is.
    grown = dict(clean); grown[planted["ratchet-web"][0]] = planted["ratchet-web"][1]
    recorded = measure(rust, grown)
    if compare(recorded, recorded):
        print("self-test: the ratchet failed at its own baseline")
        failed += 1
    if compare(measure(rust, clean), recorded):
        print("self-test: the ratchet failed on a count that fell")
        failed += 1
    print("shared-core-lint self-test: " + ("ok" if not failed else f"{failed} failed"))
    return 1 if failed else 0


if __name__ == "__main__":
    if "--self-test" in sys.argv[1:]:
        sys.exit(self_test())
    if "--update-baseline" in sys.argv[1:]:
        sys.exit(update_baseline("--allow-growth" in sys.argv[1:]))
    sys.exit(scan())
