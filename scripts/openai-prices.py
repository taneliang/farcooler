#!/usr/bin/env python3
"""Keep crates/core/src/usage.rs's OPENAI price rows equal to OpenAI's pricing page.

ov-880 priced the GPT models Codex runs from developers.openai.com. Only the
Claude rows had a daily check (scripts/claude-prices.py), and GPT-5.6 Sol's
rate is promotional until 2026-11-30, so the rows would drift unseen. This
reads the page (served as Markdown) and either

    scripts/openai-prices.py            # print the Rust rows for the models usage.rs prices
    scripts/openai-prices.py --check    # fail, with a diff, if a row differs
    scripts/openai-prices.py --write    # rewrite the rates of the rows usage.rs has
    scripts/openai-prices.py --self-test

`--file PAGE.md` reads a saved copy instead of the network. A daily CI job
(.github/workflows/openai-prices.yml) runs `--check`. Exit 1 is drift; exit 2
is a page this script cannot read, which is loud on purpose: a layout change
must never read as "no drift".

Rules it applies, each one a decision about the page:
  * The page lists many models Codex never runs (astra, cyber, ...). usage.rs
    prices a chosen few, so the check goes from usage.rs's rows to the page,
    not the other way: a row must match its page row, and a row the page no
    longer lists is drift. A model on the page that usage.rs lacks is only
    noted, never a failure.
  * Retired models are skipped, as for Claude (R-106): a model the page dropped
    moves to usage.rs's RETIRED table, which this script never reads.
  * Rates come from the first "Standard pricing data" table, Short context
    columns: input, cached input, output. Long context is not modeled by
    usage.rs (Codex's window stays under 272,000 tokens).
  * Fast mode is priced by the page per model; usage.rs prices Codex as
    standard, and says so, so this checks only that the page still bills Fast
    at twice Standard for the rows it has (the assumption in usage.rs's note).
"""
import argparse
import difflib
import re
import sys
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
USAGE_RS = ROOT / "crates/core/src/usage.rs"
FIXTURE = ROOT / "crates/core/fixtures/openai-pricing.md"
URL = "https://developers.openai.com/api/docs/pricing"
MIN_MODELS = 5  # the page lists 7+ Codex models; far fewer means the layout changed
WANT = ["short context input", "short context cached input", "short context output"]


class Unreadable(Exception):
    """The page is not shaped the way this script expects."""


def fetch(url):
    last = None
    for attempt in range(3):
        try:
            req = urllib.request.Request(url, headers={"Accept": "text/markdown", "User-Agent": "farcooler-openai-prices"})
            with urllib.request.urlopen(req, timeout=30) as r:
                return r.read().decode("utf-8")
        except Exception as e:  # noqa: BLE001 - any failure is retried, then reported
            last = e
            time.sleep(2 * (attempt + 1))
    raise Unreadable(f"could not fetch {url}: {last}")


def table_after(page, heading):
    """The first Markdown table after the first line equal to `heading`."""
    lines = page.splitlines()
    start = next((i for i, l in enumerate(lines) if l.strip() == heading), None)
    if start is None:
        raise Unreadable(f"no section {heading!r}")
    rows, seen = [], False
    for line in lines[start:]:
        if line.lstrip().startswith("|"):
            seen = True
            cells = [c.strip() for c in line.strip().strip("|").split("|")]
            if all(re.fullmatch(r":?-+:?", c) for c in cells):
                continue
            rows.append(cells)
        elif seen:
            break
    if len(rows) < 2:
        raise Unreadable(f"no table under {heading!r}")
    return rows


def money(cell, what):
    m = re.fullmatch(r"\$\s*([\d,]*\.?\d+)", cell.strip())
    if not m:
        raise Unreadable(f"no price in {cell!r} ({what})")
    return float(m.group(1).replace(",", ""))


def parse_table(page, heading):
    """{id: (input, cached_input, output)} from the first table under `heading`."""
    rows = table_after(page, heading)
    head = [h.lower() for h in rows[0]]
    cols = []
    for w in WANT:
        if head.count(w) != 1:
            raise Unreadable(f"column {w!r} not found exactly once in {rows[0]}")
        cols.append(head.index(w))
    out = {}
    for cells in rows[1:]:
        model = cells[0].strip().lower()
        if model in out:
            raise Unreadable(f"{model} listed twice under {heading!r}")
        # A "-" price is a model without that rate; only a model usage.rs
        # prices needs all three, so the error waits until then.
        out[model] = tuple(cells[c] for c in cols)
    if len(out) < MIN_MODELS:
        raise Unreadable(f"only {len(out)} models under {heading!r}")
    return out


def rates_of(table, model):
    raw = table[model]
    return tuple(money(c, f"{model} {w}") for c, w in zip(raw, WANT))


OPENAI_BLOCK = re.compile(r"^const OPENAI: &\[\(&str, Rates\)\] = &\[\n(.*?)^\];", re.M | re.S)
ROW = re.compile(r'^(\s*)\("([^"]+)", openai\(([^)]*)\)\),$')


def usage_rows(src):
    """[(id, (input, cached, output))] from usage.rs's OPENAI table, in order."""
    m = OPENAI_BLOCK.search(src)
    if not m:
        raise Unreadable(f"{USAGE_RS} has no OPENAI table in the expected shape")
    rows = []
    for line in m.group(1).splitlines():
        if not line.strip():
            continue
        r = ROW.match(line)
        if not r:
            raise Unreadable(f"OPENAI row in an unexpected shape: {line!r}")
        rows.append((r.group(2), tuple(float(x) for x in r.group(3).split(","))))
    if not rows:
        raise Unreadable("usage.rs's OPENAI table is empty")
    return rows


def num(x):
    return repr(round(x, 6))


def render_row(model, r):
    return f'    ("{model}", openai({", ".join(num(v) for v in r)})),'


def page_rows(page, wanted):
    """The page's rates for each wanted id, and the drift found in the Fast table."""
    std = parse_table(page, "### Standard pricing data")
    fast = parse_table(page, "### Fast pricing data")
    have, missing, bad = [], [], []
    for model in wanted:
        if model not in std:
            missing.append(model)
            continue
        r = rates_of(std, model)
        have.append((model, r))
        if model in fast:
            f = rates_of(fast, model)
            if f != tuple(2 * v for v in r):
                bad.append(f"{model}: Fast {f} is not twice Standard {r} (input, cached, output)")
        else:
            bad.append(f"{model}: not in the page's Fast table")
    return have, missing, bad, sorted(set(std) - set(wanted))


def check(page, src):
    """(problems, notes). A problem fails the check; a note is only printed."""
    mine = usage_rows(src)
    have, missing, bad, extra = page_rows(page, [m for m, _ in mine])
    problems = []
    for model in missing:
        problems.append(f"{model} is in usage.rs's OPENAI table but not on the page. If OpenAI retired it, "
                        "move its row to RETIRED (this script never compares that table); if the page renamed it, rename the row.")
    want = [render_row(m, r) + "\n" for m, r in have]
    got = [render_row(m, r) + "\n" for m, r in mine if m not in missing]
    if want != got:
        problems.append("".join(difflib.unified_diff(got, want, "usage.rs OPENAI", "pricing page", n=1)))
    problems += bad
    return problems, extra


def write(page, src):
    mine = usage_rows(src)
    have, missing, _, _ = page_rows(page, [m for m, _ in mine])
    if missing:
        raise Unreadable(f"cannot rewrite: {', '.join(missing)} not on the page; move retired rows to RETIRED by hand")
    new = {m: render_row(m, r) for m, r in have}
    block = OPENAI_BLOCK.search(src)
    body = "".join(new[m] + "\n" for m, _ in mine)
    return src[:block.start(1)] + body + src[block.end(1):]


PAGE_FOR_TEST = """## Pricing

Standard

### Standard pricing data

| Model | Short context input | Short context output | Short context cached input | Short context 30m write | Long context input |
| --- | --- | --- | --- | --- | --- |
| gpt-a | $2.00 | $10.00 | $0.10 | $2.50 | $4.00 |
| gpt-b | $1.00 | $5.00 | $0.05 | $1.25 | $2.00 |
| gpt-c | $0.10 | $0.50 | $0.01 | $0.125 | $0.20 |
| gpt-d | $4.00 | $20.00 | $0.40 | $5.00 | $8.00 |
| gpt-e | $0.20 | $1.20 | $0.02 | $0.25 | $0.40 |
| gpt-cyber | $20.00 | $100.00 | $2.00 | - | - |

### Fast pricing data

| Model | Short context input | Short context output | Short context cached input | Short context 30m write | Long context input |
| --- | --- | --- | --- | --- | --- |
| gpt-a | $4.00 | $20.00 | $0.20 | $5.00 | $8.00 |
| gpt-b | $2.00 | $10.00 | $0.10 | $2.50 | $4.00 |
| gpt-c | $0.20 | $1.00 | $0.02 | $0.25 | $0.40 |
| gpt-d | $8.00 | $40.00 | $0.80 | $10.00 | $16.00 |
| gpt-e | $0.40 | $2.40 | $0.04 | $0.50 | $0.80 |
"""

SRC_FOR_TEST = """const OPENAI: &[(&str, Rates)] = &[
    ("gpt-a", openai(2.0, 0.1, 10.0)),
    ("gpt-b", openai(1.0, 0.05, 5.0)),
    ("gpt-c", openai(0.1, 0.01, 0.5)),
    ("gpt-d", openai(4.0, 0.4, 20.0)),
    ("gpt-e", openai(0.2, 0.02, 1.2)),
];

const RETIRED: &[(&str, Rates)] = &[
    ("gpt-old", openai(9.0, 9.0, 9.0)),
];
"""


def expect_unreadable(page, what, src=SRC_FOR_TEST):
    try:
        check(page, src)
    except Unreadable:
        return
    raise AssertionError(what)


def self_test():
    # The real page, saved: every row usage.rs has must match it today.
    real = FIXTURE.read_text(encoding="utf-8")
    std = parse_table(real, "### Standard pricing data")
    assert rates_of(std, "gpt-5.6-sol") == (4.0, 0.4, 20.0), rates_of(std, "gpt-5.6-sol")
    assert rates_of(std, "gpt-6-luna") == (0.1, 0.01, 0.5)
    assert "gpt-6-cyber" not in std, "cyber models sit in a later table, which is not read"

    problems, extra = check(PAGE_FOR_TEST, SRC_FOR_TEST)
    assert not problems, problems
    assert extra == ["gpt-cyber"], extra  # an unpriced model is a note, not drift

    # Drift: one changed row is red and the diff names it.
    drifted = SRC_FOR_TEST.replace("openai(4.0, 0.4, 20.0)", "openai(4.0, 0.4, 24.0)")
    problems, _ = check(PAGE_FOR_TEST, drifted)
    assert problems and "gpt-d" in problems[0] and "20.0" in problems[0], problems
    # ...and a changed page is red the same way.
    problems, _ = check(PAGE_FOR_TEST.replace("| gpt-e | $0.20 | $1.20 |", "| gpt-e | $0.20 | $1.50 |", 1)
                        .replace("| gpt-e | $0.40 | $2.40 |", "| gpt-e | $0.40 | $3.00 |", 1), SRC_FOR_TEST)
    assert problems and "gpt-e" in problems[0], problems
    # A rewrite clears the drift.
    fixed = write(PAGE_FOR_TEST, drifted)
    assert not check(PAGE_FOR_TEST, fixed)[0] and "gpt-old" in fixed, "write must keep RETIRED untouched"

    # A row the page dropped is drift, unless it has moved to RETIRED.
    gone = PAGE_FOR_TEST.replace("| gpt-b |", "| gpt-z |")
    problems, _ = check(gone, SRC_FOR_TEST)
    assert any("gpt-b" in p and "RETIRED" in p for p in problems), problems
    moved = SRC_FOR_TEST.replace('    ("gpt-b", openai(1.0, 0.05, 5.0)),\n', "").replace(
        '("gpt-old", openai(9.0, 9.0, 9.0)),', '("gpt-old", openai(9.0, 9.0, 9.0)),\n    ("gpt-b", openai(1.0, 0.05, 5.0)),')
    assert not check(gone, moved)[0], "a retired row is skipped"

    # Fast mode no longer twice Standard is flagged.
    problems, _ = check(PAGE_FOR_TEST.replace("| gpt-a | $4.00 | $20.00 |", "| gpt-a | $5.00 | $20.00 |"), SRC_FOR_TEST)
    assert any("gpt-a" in p and "Fast" in p for p in problems), problems

    # Layout changes are unreadable, never silent.
    expect_unreadable(PAGE_FOR_TEST.replace("Short context cached input", "Cached input"), "a column rename")
    expect_unreadable(PAGE_FOR_TEST.replace("### Standard pricing data", "### Prices"), "a heading rename")
    expect_unreadable(PAGE_FOR_TEST.replace("| gpt-a | $2.00 |", "| gpt-a | n/a |", 1), "a price that is not a price")
    expect_unreadable(PAGE_FOR_TEST, "a row in an unexpected shape",
                      SRC_FOR_TEST.replace("openai(2.0, 0.1, 10.0)", "openai(2.0, 0.1, 10.0) /* x */"))
    print("openai-prices self-test ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check", action="store_true", help="fail if usage.rs differs from the page")
    ap.add_argument("--write", action="store_true", help="rewrite the rates of usage.rs's OPENAI rows")
    ap.add_argument("--file", help="read a saved copy of the page instead of fetching it")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    try:
        page = Path(args.file).read_text(encoding="utf-8") if args.file else fetch(URL)
        src = USAGE_RS.read_text(encoding="utf-8")
        if args.write:
            USAGE_RS.write_text(write(page, src), encoding="utf-8")
            print(f"Rewrote the OPENAI rows in {USAGE_RS.relative_to(ROOT)}. Update the tests that name rows "
                  "and the \"read\" date in the comment above the table.")
            return 0
        if args.check:
            problems, extra = check(page, src)
            if problems:
                print("crates/core/src/usage.rs OPENAI rows no longer match " + URL + "\n")
                print("\n".join(problems))
                print("Fix: scripts/openai-prices.py --write, then update the row tests in usage.rs and the "
                      "\"read\" date in the comment above OPENAI. A dropped model moves to RETIRED.")
                return 1
            note = f" The page also lists {len(extra)} model(s) usage.rs does not price." if extra else ""
            print(f"usage.rs OPENAI rows match the pricing page ({len(usage_rows(src))} models).{note}")
            return 0
        have, missing, _, _ = page_rows(page, [m for m, _ in usage_rows(src)])
        print("\n".join(render_row(m, r) for m, r in have))
        return 0
    except Unreadable as e:
        print(f"openai-prices: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
