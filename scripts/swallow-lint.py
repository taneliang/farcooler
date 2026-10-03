#!/usr/bin/env python3
"""No new swallowed client call (ov-167).

Root cause 3 of the ov-102 review: failures are swallowed. A send, an answer
or a refresh that the host refused looks exactly like one it accepted, because
the call that carried it threw into a `try?` and nobody heard. This scans the
apps' own sources for the shapes that swallow a call to the host:

  Swift   `_ = try? ...` and `try? await ...` on a call to the core, daemon
          or CLI client: `core.call(`, `rpc(`, `runCLI(`, `call(`, `post(`,
          `send…(`, or any method on `core.`, `client.`, `daemon.`,
          `connection.` or `stream.`. Searched in apps/macos/Sources,
          apps/ios and apps/shared/AgentKit/Sources.
  Kotlin  a `runCatching { … }` or `attempt { … }` (net/Cancellation.kt's
          cancellation-safe runCatching) used as a statement, with its result
          ignored or only `.getOrNull()`ed; and a `catch (e: Exception)` whose
          body is empty once comments and `e.rethrowIfCancellation()` are
          taken out. Searched in all of the Android app's Kotlin.

A `Task.sleep`, a decode or a notification post is not a call to the host and
isn't matched. A call split so that `try?` and the callee sit on different
lines isn't seen.

Every hit that exists on purpose is listed in scripts/swallow-lint-allow.txt,
one per line, with the reason it's fine to swallow:

  path :: function :: statement :: reason

`statement` is the hit with its whitespace collapsed (the scan prints it ready
to paste), so a line number moving doesn't break the entry, and the same
statement twice in one function needs two entries. An entry without a reason,
and an entry that no longer matches anything, fail the scan too: the list only
shrinks by fixing a call, never by forgetting one.

  ./scripts/swallow-lint.py              scan the tree; exit 1 on a new hit
  ./scripts/swallow-lint.py --self-test  check the patterns against known cases
"""

import collections
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
ALLOW = ROOT / "scripts" / "swallow-lint-allow.txt"

SWIFT_ROOTS = ["apps/macos/Sources", "apps/ios", "apps/shared/AgentKit/Sources"]
# All of the app's Kotlin, not just net/ and model/: a screen that calls the
# connection itself (ui/Sheets.kt's new terminal) swallows just as well.
KOTLIN_ROOTS = ["apps/android/app/src/main/java/com/farcooler"]
SKIP_PARTS = {"Tests", "FarCoolerUITests", "test", "androidTest", ".build", "build"}

# `_ = try?` (awaited or not) or `try? await`, then a call to the host.
SWIFT_CALL = re.compile(
    r"(?:_\s*=\s*try\?\s*(?:await\s+)?|try\?\s*await\s+)"
    r"(?:self\.)?"
    r"(?:(?:core|client|daemon|connection|stream)\.\w+|rpc|runCLI|call|post|send\w*)\s*\("
)
KOTLIN_CATCHING = re.compile(r"^\s*(?:runCatching|attempt)\s*\{")
KOTLIN_CATCH = re.compile(r"catch\s*\(\s*(\w+)\s*:\s*\w*(?:Exception|Throwable)\s*\)\s*\{")
# What a statement-level runCatching may be followed by and still have its
# failure looked at.
HANDLED = re.compile(r"^\s*\??\.\s*(?:onFailure|getOrElse|getOrThrow|getOrDefault|fold|exceptionOrNull|isFailure|isSuccess|recover\w*)\b")
SWIFT_FUNC = re.compile(r"\b(?:func\s+(\w+)|(init)\s*[(<?!]|(deinit)\b|var\s+(\w+)\s*:[^=]*\{\s*$)")
KOTLIN_FUNC = re.compile(r"\bfun\s+(?:<[^>]*>\s*)?(?:[\w.]+\.)?(\w+)\s*\(")


def collapse(text: str) -> str:
    return " ".join(text.split())


def is_comment(line: str) -> bool:
    return line.lstrip().startswith(("//", "*", "/*"))


def balanced_end(text: str, start: int, open_ch: str, close_ch: str) -> int:
    """Index just past the bracket that closes the first `open_ch` at or after
    `start`, skipping string literals and comments; len(text) if none."""
    depth = 0
    i = start
    seen = False
    while i < len(text):
        c = text[i]
        if text.startswith("//", i):
            nl = text.find("\n", i)
            i = len(text) if nl < 0 else nl
            continue
        if text.startswith("/*", i):
            end = text.find("*/", i + 2)
            i = len(text) if end < 0 else end + 2
            continue
        if c == '"':
            i += 1
            while i < len(text) and text[i] != '"' and text[i] != "\n":
                i += 2 if text[i] == "\\" else 1
            i += 1
            continue
        if c == open_ch:
            depth += 1
            seen = True
        elif c == close_ch and seen:
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    return len(text)


def enclosing(lines: list[str], index: int, pattern: re.Pattern) -> str:
    for line in reversed(lines[: index + 1]):
        if is_comment(line):
            continue
        m = pattern.search(line)
        if m:
            return next(g for g in m.groups() if g)
    return "-"


def offsets(text: str) -> list[int]:
    out, pos = [], 0
    for line in text.splitlines(keepends=True):
        out.append(pos)
        pos += len(line)
    return out


def swift_hits(text: str) -> list[tuple[int, str, str]]:
    lines = text.splitlines()
    starts = offsets(text)
    out = []
    for index, line in enumerate(lines):
        if is_comment(line):
            continue
        code = line.split("//")[0] if "//" in line and '"' not in line else line
        m = SWIFT_CALL.search(code)
        if not m:
            continue
        begin = starts[index] + m.start()
        end = balanced_end(text, starts[index] + m.end() - 1, "(", ")")
        statement = collapse(re.sub(r"(?m)//[^\n\"]*$", "", text[begin:end]))
        out.append((index + 1, enclosing(lines, index, SWIFT_FUNC), statement))
    return out


# A line ending in one of these hands its value to the next line, so a
# runCatching there is an expression, not a statement.
VALUE_BEFORE = re.compile(
    r"(?:=|\(|,|->|\?:|\b(?:let|run|map|mapNotNull|also|apply|takeIf|with)\s*\{"
    r"|\bwithContext\([^()]*\)\s*\{)\s*$")


def in_value_position(lines: list[str], index: int) -> bool:
    for line in reversed(lines[:index]):
        code = re.sub(r"//.*", "", line).strip()
        if not code or is_comment(line):
            continue
        return bool(VALUE_BEFORE.search(code))
    return False


def kotlin_hits(text: str) -> list[tuple[int, str, str]]:
    lines = text.splitlines()
    starts = offsets(text)
    out = []
    for index, line in enumerate(lines):
        if is_comment(line):
            continue
        if KOTLIN_CATCHING.match(line):
            begin = starts[index] + len(line) - len(line.lstrip())
            end = balanced_end(text, begin, "{", "}")
            after = text[end:]
            # `.onSuccess { … }` looks at the value, not the failure: step over
            # it and judge what follows.
            while (step := re.match(r"^\s*\??\.\s*onSuccess\s*\{", after)):
                end = balanced_end(text, end + step.end() - 1, "{", "}")
                after = text[end:]
            # `.getOrNull()` as a statement throws the failure away all the same.
            ignored = re.match(r"^\s*\??\.\s*getOrNull\s*\(\s*\)", after)
            if ignored:
                end += ignored.end()
                after = text[end:]
                # ...unless the value goes on: `?.let`, `?: return`.
                if re.match(r"^\s*(?:\??\.|\?:)", after):
                    continue
            if not HANDLED.match(after) and not in_value_position(lines, index):
                out.append((index + 1, enclosing(lines, index, KOTLIN_FUNC),
                            collapse(text[begin:end])))
        for m in KOTLIN_CATCH.finditer(line):
            if is_comment(line[: m.start()] + "x"):
                continue
            name = m.group(1)
            open_at = starts[index] + m.end() - 1
            end = balanced_end(text, open_at, "{", "}")
            body = text[open_at + 1 : end - 1]
            body = re.sub(r"/\*.*?\*/", "", body, flags=re.S)
            body = re.sub(r"//[^\n]*", "", body)
            body = body.replace(f"{name}.rethrowIfCancellation()", "")
            if body.strip() == "":
                whole = re.sub(r"//[^\n]*", "", text[starts[index] + m.start() : end])
                out.append((index + 1, enclosing(lines, index, KOTLIN_FUNC), collapse(whole)))
    return out


def files():
    for roots, suffix, kind in ((SWIFT_ROOTS, ".swift", "swift"), (KOTLIN_ROOTS, ".kt", "kotlin")):
        for root in roots:
            for path in sorted((ROOT / root).rglob("*" + suffix)):
                if SKIP_PARTS & set(path.relative_to(ROOT).parts):
                    continue
                yield path, kind


def hits(text: str, kind: str) -> list[tuple[int, str, str]]:
    return swift_hits(text) if kind == "swift" else kotlin_hits(text)


def load_allow() -> tuple[collections.Counter, list[str]]:
    allowed, problems = collections.Counter(), []
    if not ALLOW.exists():
        return allowed, problems
    for number, raw in enumerate(ALLOW.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split(" :: ")
        if len(parts) < 4:
            problems.append(f"{ALLOW.name}:{number}: not `path :: function :: statement :: reason`: {line}")
            continue
        path, function, reason = parts[0], parts[1], parts[-1].strip()
        statement = " :: ".join(parts[2:-1])
        if not reason:
            problems.append(f"{ALLOW.name}:{number}: no reason given: {line}")
            continue
        allowed[(path, function, collapse(statement))] += 1
    return allowed, problems


def scan() -> int:
    allowed, problems = load_allow()
    remaining = collections.Counter(allowed)
    new = []
    total = 0
    for path, kind in files():
        rel = str(path.relative_to(ROOT))
        for number, function, statement in hits(path.read_text(encoding="utf-8"), kind):
            total += 1
            key = (rel, function, statement)
            if remaining[key] > 0:
                remaining[key] -= 1
            else:
                new.append(f"{rel}:{number}: swallowed call in {function}: {statement}\n"
                           f"    allow as: {rel} :: {function} :: {statement} :: <reason>")
    stale = [f"{ALLOW.name}: matches nothing (fixed? remove it): {p} :: {f} :: {s}"
             for (p, f, s), n in remaining.items() for _ in range(n)]
    for line in problems + new + stale:
        print(line)
    if problems or new or stale:
        print(f"\n{len(new)} new swallowed call(s), {len(stale)} stale and {len(problems)} "
              "malformed allowlist entr(ies). Show the failure (a banner, a send that "
              "stays in the composer, a thrown error), or, if it's truly ignorable, add "
              f"it to scripts/{ALLOW.name} with the reason.")
        return 1
    print(f"swallow-lint: no new swallowed calls ({total} allowlisted)")
    return 0


def self_test() -> int:
    cases = [
        # Each must be caught.
        ("swift", '_ = try? await core.call("terminal.seen", ["terminal": id])', True),
        ("swift", '        _ = try? await core.call(\n            "x", [:])', True),
        ("swift", 'guard let data = try? await core.call("host") else { return }', True),
        ("swift", '_ = try? await runCLI(["terminal", "agent-cancel", t])', True),
        ("swift", 'if let data = try? await rpc("task.list", args) {}', True),
        ("swift", '_ = try? await self.client.refresh()', True),
        ("swift", '_ = try? client.sendInput(bytes)', True),
        ("swift", '_ = try? await stream.send(frame)', True),
        ("swift", '_ = try? await post("/v1/auth/logout", body)', True),
        ("kotlin", 'fun f() {\n    attempt { core.call("x") }\n}', True),
        ("kotlin", 'fun f() {\n    runCatching {\n        core.call("x")\n    }\n}', True),
        ("kotlin", 'fun f() {\n    runCatching { core.call("x") }.getOrNull()\n}', True),
        ("kotlin", 'fun f() {\n    try { g() } catch (e: Exception) {}\n}', True),
        ("kotlin", 'fun f() {\n    try { g() } catch (e: Exception) {\n        // fine\n    }\n}', True),
        ("kotlin", 'fun f() {\n    try { g() } catch (e: Exception) {\n        e.rethrowIfCancellation()\n    }\n}', True),
        # Each must pass.
        ("swift", 'try? await Task.sleep(for: .seconds(1))', False),
        ("swift", '_ = try? list.decode(Skipped.self)', False),
        ("swift", 'let data = try await core.call("host")', False),
        ("swift", '// _ = try? await core.call("x")', False),
        ("swift", 'try? await UNUserNotificationCenter.current().add(request)', False),
        ("swift", 'let x = try? JSONDecoder().decode(T.self, from: data)', False),
        ("kotlin", 'fun f() {\n    val x = runCatching { g() }.getOrNull()\n}', False),
        ("kotlin", 'fun f() = runCatching { g() }', False),
        ("kotlin", 'val x =\n    runCatching { g() }\n        .getOrNull()?.items ?: emptyList()', False),
        ("kotlin", 'read = { w ->\n    attempt { g(w) }\n        .getOrNull()\n}', False),
        ("kotlin", 'val y = raw?.let {\n    runCatching { g(it) }.getOrNull()\n}', False),
        ("kotlin", 'val z = withContext(Dispatchers.Default) {\n    runCatching { g() }.getOrNull()\n}', False),
        ("kotlin", 'scope.launch {\n    attempt { core.call("x") }\n}', True),
        ("kotlin", 'fun f() {\n    attempt { g() }\n        .onFailure { show(it) }\n}', False),
        ("kotlin", 'fun f() {\n    runCatching { g() }.getOrElse { return }\n}', False),
        ("kotlin", 'fun f() {\n    runCatching { g() }\n        .onSuccess { use(it) }\n        .onFailure { show(it) }\n}', False),
        ("kotlin", 'fun f() {\n    runCatching { g() }.onSuccess { use(it) }\n}', True),
        ("kotlin", 'fun f() {\n    try { g() } catch (e: Exception) {\n        e.rethrowIfCancellation()\n        show(e)\n    }\n}', False),
        ("kotlin", '/** caught by `catch (e: Exception) {}` */', False),
    ]
    failed = 0
    for kind, text, caught in cases:
        if bool(hits(text, kind)) != caught:
            print(f"self-test: {'missed' if caught else 'wrongly caught'} {kind}: {text!r}")
            failed += 1
    # The statement is what an allowlist entry names, so it must be stable.
    got = hits('func seen() {\n    _ = try? await core.call(\n        "x",\n        [:])\n}', "swift")
    if got != [(2, "seen", '_ = try? await core.call( "x", [:])')]:
        print(f"self-test: wrong key for a split call: {got}")
        failed += 1
    print("swallow-lint self-test: " + ("ok" if not failed else f"{failed} failed"))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(self_test() if "--self-test" in sys.argv[1:] else scan())
