#!/usr/bin/env python3
"""Keep crates/core/src/usage.rs's price table equal to Anthropic's pricing page.

The table behind every API-equivalent dollar in Far Cooler was once typed by
hand, and two of its rows were wrong and three models were missing before
anyone looked (ov-460). The page is the source; this reads its "Model pricing"
and "Fast mode pricing" tables (the page is served as Markdown) and either

    scripts/claude-prices.py            # print the Rust rows
    scripts/claude-prices.py --check    # fail, with a diff, if usage.rs differs
    scripts/claude-prices.py --write    # rewrite usage.rs's rows and bump PRICE_TABLE
    scripts/claude-prices.py --self-test

`--file PAGE.md` reads a saved copy instead of the network. A daily CI job
(.github/workflows/claude-prices.yml) runs `--check`. Exit 1 is drift; exit 2
is a page this script cannot read, which is loud on purpose: a layout change
must never read as "no drift".

Rules it applies, each one a decision about the page:
  * An id is the model name lowercased, dots to hyphens: "Claude Opus 5.5" is
    `claude-opus-5-5`. A parenthetical (retired, limited availability) is
    dropped.
  * Claude Haiku 5.5 has two rows by prompt size. A turn in the store is the
    sum of many requests, with no request's size, so the row kept is the
    OVER-100,000-tier: an estimate can be high, never low.
  * Fast mode is listed per model on its own table; usage.rs prices it at twice
    the standard rates, so the check also requires the page to say so.
"""
import argparse
import datetime
import difflib
import re
import sys
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
USAGE_RS = ROOT / "crates/core/src/usage.rs"
URL = "https://platform.claude.com/docs/en/about-claude/pricing.md"
OVER_TIER = "over 100,000 tokens"
UNDER_TIER = "up to 100,000 tokens"
MIN_MODELS = 12  # the page lists 21 rows; far fewer means the layout changed


class Unreadable(Exception):
    """The page is not shaped the way this script expects."""


def fetch(url):
    last = None
    for attempt in range(3):
        try:
            req = urllib.request.Request(url, headers={"Accept": "text/markdown", "User-Agent": "farcooler-claude-prices"})
            with urllib.request.urlopen(req, timeout=30) as r:
                return r.read().decode("utf-8")
        except Exception as e:  # noqa: BLE001 - any failure is retried, then reported
            last = e
            time.sleep(2 * (attempt + 1))
    raise Unreadable(f"could not fetch {url}: {last}")


def table_after(page, heading_pattern):
    """The first Markdown table at or after a line matching `heading_pattern`."""
    lines = page.splitlines()
    start = next((i for i, l in enumerate(lines) if re.search(heading_pattern, l)), None)
    if start is None:
        raise Unreadable(f"no section matching {heading_pattern!r}")
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
        raise Unreadable(f"no table under {heading_pattern!r}")
    return rows


def money(cell):
    m = re.search(r"\$\s*([\d,]*\.?\d+)", re.sub(r"<[^>]+>", "", cell))
    if not m:
        raise Unreadable(f"no price in {cell!r}")
    return float(m.group(1).replace(",", ""))


def slug(name):
    name = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", name)  # links keep their text
    return re.sub(r"[^a-z0-9]+", "-", name.lower().replace(".", "-")).strip("-")


def parse_models(page):
    """{id: (input, write_5m, write_1h, read, output)} from "Model pricing"."""
    rows = table_after(page, r"^## Model pricing")
    head = [h.lower() for h in rows[0]]
    want = ["base input", "5m cache", "1h cache", "cache hits", "output"]
    cols = []
    for w in want:
        i = next((i for i, h in enumerate(head) if w in h), None)
        if i is None:
            raise Unreadable(f"no column {w!r} in {rows[0]}")
        cols.append(i)
    out = {}
    for cells in rows[1:]:
        label = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", cells[0])
        tier = re.search(r"\(for prompts ([^)]*)\)", label)
        if tier and tier.group(1).strip() == UNDER_TIER:
            continue  # the cheaper tier; see the module docstring
        if tier and tier.group(1).strip() != OVER_TIER:
            raise Unreadable(f"a pricing tier this script does not know: {label!r}")
        model = slug(re.sub(r"\(.*?\)", "", label))
        if not model.startswith("claude-"):
            raise Unreadable(f"{label!r} is not a Claude model name")
        if model in out:
            raise Unreadable(f"{model} listed twice")
        out[model] = tuple(money(cells[c]) for c in cols)
    if len(out) < MIN_MODELS:
        raise Unreadable(f"only {len(out)} models found")
    return out


def parse_fast(page, models):
    """The ids that offer fast mode, and an error list where a price is not twice standard."""
    rows = table_after(page, r"^### Fast mode pricing")
    ids, bad = [], []
    for cells in rows[1:]:
        for name in cells[0].split(" / "):
            model = slug(name)
            if model not in models:
                raise Unreadable(f"fast mode lists {name!r}, which is not in the model table")
            ids.append(model)
            std = models[model]
            fast_in, fast_out = money(cells[1]), money(cells[2])
            if (fast_in, fast_out) != (std[0] * 2, std[4] * 2):
                bad.append(f"{model}: fast ${fast_in}/${fast_out} is not twice ${std[0]}/${std[4]}")
    if not ids:
        raise Unreadable("no model in the fast mode table")
    return ids, bad


def num(x):
    return repr(round(x, 6))


def render_table(models):
    lines = ["const TABLE: &[(&str, Rates)] = &["]
    for model, r in models.items():
        lines.append(f'    ("{model}", rate({", ".join(num(v) for v in r)})),')
    lines.append("];")
    return "\n".join(lines)


def render_fast(ids):
    items = ", ".join(f'"{i}"' for i in ids)
    return f"const FAST_MODE: &[&str] = &[{items}];"


def usage_blocks(src):
    t = re.search(r"^const TABLE: &\[\(&str, Rates\)\] = &\[\n.*?^\];", src, re.M | re.S)
    f = re.search(r"^const FAST_MODE: &\[&str\] = &\[[^\]]*\];", src, re.M)
    d = re.search(r'^pub const PRICE_TABLE: &str = "([^"]*)";', src, re.M)
    if not (t and f and d):
        raise Unreadable(f"{USAGE_RS} has no TABLE, FAST_MODE or PRICE_TABLE in the expected shape")
    return t, f, d


def tidy_fast(text):
    """usage.rs may wrap FAST_MODE over lines; compare on its content."""
    return re.sub(r"\s+", " ", text).replace("[ ", "[").replace(", ]", "]").replace(",]", "]")


def check(models, fast_ids, src):
    t, f, _ = usage_blocks(src)
    problems = []
    want_t, have_t = render_table(models), t.group(0)
    if want_t != have_t:
        problems.append("".join(difflib.unified_diff(
            (have_t + "\n").splitlines(True), (want_t + "\n").splitlines(True), "usage.rs TABLE", "pricing page", n=1)))
    want_f, have_f = render_fast(fast_ids), f.group(0)
    if tidy_fast(want_f) != tidy_fast(have_f):
        problems.append(f"FAST_MODE differs:\n  usage.rs: {tidy_fast(have_f)}\n  page:     {want_f}\n")
    return problems


def write(models, fast_ids, src):
    t, f, d = usage_blocks(src)
    today = datetime.date.today().isoformat()
    # Later spans first, so earlier offsets stay valid.
    for m, text in sorted(
        [(t, render_table(models)), (f, render_fast(fast_ids)),
         (d, f'pub const PRICE_TABLE: &str = "{today}";')], key=lambda p: -p[0].start()):
        src = src[:m.start()] + text + src[m.end():]
    return src


def self_test():
    page = """## Model pricing

| Model | Base input tokens | 5m cache writes | 1h cache writes | Cache hits and refreshes | Output tokens |
| :-- | :-- | :-- | :-- | :-- | :-- |
""" + "".join(
        f"| Claude Model {i} | $1 / MTok | $1.25 / MTok | $2 / MTok | $0.10 / MTok<sup>1</sup> | $5 / MTok |\n"
        for i in range(MIN_MODELS)) + """| Claude Haiku 5.5 (for prompts up to 100,000 tokens) | $0.10 / MTok | $0.125 / MTok | $0.20 / MTok | $0.01 / MTok | $0.50 / MTok |
| Claude Haiku 5.5 (for prompts over 100,000 tokens) | $0.50 / MTok | $0.625 / MTok | $1 / MTok | $0.05 / MTok | $2.50 / MTok |
| Claude Mythos 5.1 ([limited availability](https://x.test)) | $10 / MTok | $12.50 / MTok | $20 / MTok | $0.25 / MTok | $50 / MTok |

### Fast mode pricing

| Model | Input | Output |
| --- | --- | --- |
| Claude Model 1 / Claude Model 2 | $2 / MTok | $10 / MTok |
"""
    models = parse_models(page)
    assert models["claude-haiku-5-5"] == (0.5, 0.625, 1.0, 0.05, 2.5), models["claude-haiku-5-5"]
    assert models["claude-mythos-5-1"][0] == 10.0 and models["claude-model-3"][3] == 0.1
    ids, bad = parse_fast(page, models)
    assert ids == ["claude-model-1", "claude-model-2"] and not bad, (ids, bad)
    src = (f'pub const PRICE_TABLE: &str = "2000-01-01";\n{render_table({"claude-old": (1.0, 1.25, 2.0, 0.1, 5.0)})}\n'
           f"{render_fast(['claude-old'])}\n")
    assert check(models, ids, src), "drift must be reported"
    fixed = write(models, ids, src)
    assert not check(models, ids, fixed), "a rewrite must clear the drift"
    assert datetime.date.today().isoformat() in fixed
    page_bad = page.replace("$2 / MTok | $10 / MTok", "$3 / MTok | $10 / MTok")
    assert parse_fast(page_bad, parse_models(page_bad))[1], "a fast price that is not twice must be flagged"
    try:
        parse_models(page.replace("Cache hits and refreshes", "Reads"))
    except Unreadable:
        pass
    else:
        raise AssertionError("a column rename must be unreadable, not silent")
    print("claude-prices self-test ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check", action="store_true", help="fail if usage.rs differs from the page")
    ap.add_argument("--write", action="store_true", help="rewrite usage.rs's table and PRICE_TABLE")
    ap.add_argument("--file", help="read a saved copy of the page instead of fetching it")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    try:
        page = Path(args.file).read_text(encoding="utf-8") if args.file else fetch(URL)
        models = parse_models(page)
        fast_ids, bad = parse_fast(page, models)
        src = USAGE_RS.read_text(encoding="utf-8")
        if bad:
            print("The page's fast mode prices are no longer twice standard; usage.rs assumes they are:", *bad, sep="\n  ")
            return 2
        if args.write:
            USAGE_RS.write_text(write(models, fast_ids, src), encoding="utf-8")
            print(f"Rewrote {USAGE_RS.relative_to(ROOT)}: {len(models)} models. Update the tests that name rows.")
            return 0
        if args.check:
            problems = check(models, fast_ids, src)
            if problems:
                print("crates/core/src/usage.rs no longer matches " + URL + "\n")
                print("\n".join(problems))
                print("Fix: scripts/claude-prices.py --write, then update the row tests in usage.rs "
                      "(every_model_on_the_pricing_page_has_its_listed_rates). --write also bumps PRICE_TABLE.")
                return 1
            print(f"usage.rs matches the pricing page ({len(models)} models, fast mode on {len(fast_ids)}).")
            return 0
        print(render_table(models))
        print()
        print(render_fast(fast_ids))
        return 0
    except Unreadable as e:
        print(f"claude-prices: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
