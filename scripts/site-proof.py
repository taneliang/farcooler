#!/usr/bin/env python3
"""Regenerate the "Far Cooler builds Far Cooler" section of the website (ov-499).

The section in services/site/public/index.html shows numbers that go stale:
a weekly commit chart, seven milestones, four stats and the open themes with
three cards each. This script rewrites exactly the part of the page between
two comment markers and nothing else:

    <!-- site-proof:begin ... -->
    ...generated...
    <!-- site-proof:end -->

    scripts/site-proof.py                 rewrite the page in place
    scripts/site-proof.py --check         change nothing; exit 1 if the page is stale
    scripts/site-proof.py --self-test

Where each part comes from:

  Chart     `git log <ref> --format=%ad --date=short` (author date), counted in
            weeks that start on Sunday, from the first commit's week. The week
            that holds today is partial and is left out. A week with no commits
            is drawn as a zero. The axis tops out at the next hundred above the
            busiest week; gridlines are every 500; only the busiest week carries
            a printed value.
  Milestones  scripts/site-proof-milestones.json, written by hand. They are not
            computed: each is a dated moment the owner chose, with a comment
            naming the commit or card it comes from. The chart badges a
            milestone in the week that holds its date. The same file holds the
            "pivot" date and sentence that open the takeaways under the chart, and
            "tokens_since", the day usage tracking began (a fact about the
            database, not computed).
  Stats     `farcooler-canary plan --repo overnight --json`: board_counts.done
            (cards done), lanes and trains in state "landed", and every ruling
            (decided for you) with the reversed ones counted apart. Two more
            come from elsewhere:
              Both tiles below stop where the chart does: the last day of the
              last full week, so each equals its chart headline.
              Lines of code  lines in the tracked source files on origin/main
                at that day (the last commit by then; see loc_by_week)
                (`git ls-tree`, read from the object store, so the tree being
                edited never matters). SOURCE RULE: a file counts when its
                extension is rs, swift, kt, kts, ts, mjs, py, sh, go, sql or
                proto, and no directory on its path is named fixtures, *-fixtures,
                baseline, vendor, third_party or node_modules. Every line counts,
                blank and comment lines too, tests too. Left out on purpose:
                lockfiles, JSON, Markdown, the generated C headers under
                include/, the site's HTML, fixtures and anything vendored.
              Tokens used  every Claude Code token (input, output, cache read
                and cache write) this project's agents have used from the start of tracking to that day. It
                is the plan's Cost-section query (crates/store/src/plan_cost.rs)
                with the seven-day window removed: turns filed to the board's
                repository, plus unfiled turns that belong to one of its cards.
                The CLI only offers the last seven days, so this reads the
                runner's database read-only. Its path is SITE_PROOF_USAGE_DB, or
                the Canary app's, or the release app's. Usage tracking began on
                Aug 31, so the total starts there, not at the first commit.
  Themes    the same plan plus `farcooler-canary task list --repo overnight
            --json` for card titles and statuses.

THEME RULE (the page's own, written down). A theme is open when it has at
least one card that is neither done nor cancelled. Open themes are ordered by
last_moved_at, newest first, and the first six are shown. Its progress is done
cards over all cards that are not cancelled. Each shows three cards:

  - up to two of the cards in progress or in review, then done cards, each
    group newest first (the highest ov number first), in that order;
  - then one card that has not started (backlog, todo or needs a decision),
    the oldest first, so the list always ends on what comes next;
  - if there is nothing not started, or fewer than two to show first, the
    remaining places are filled from whichever group has cards left.

The page's date line is the author date of main's newest commit, not the
clock, so a second run with no new commit and no board change writes the same
bytes. The clock only decides which week is partial (--today overrides it).

The script never writes outside the markers: it checks that the text before
the begin marker and after the end marker is byte-identical to what it read,
and refuses (exit 2, file untouched) when the markers are missing, repeated
or out of order.
"""
import datetime as dt
import html
import json
import os
import sqlite3
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
PAGE = os.path.join(ROOT, "services/site/public/index.html")
MILESTONES = os.path.join(HERE, "site-proof-milestones.json")
BEGIN = "      <!-- site-proof:begin (generated by scripts/site-proof.py; edit the data in scripts/site-proof-milestones.json, never this region) -->"
END = "      <!-- site-proof:end -->"
MONTHS = ["January", "February", "March", "April", "May", "June", "July",
          "August", "September", "October", "November", "December"]
WORDS = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight",
         "nine", "ten", "eleven", "twelve"]
DONE = '<svg viewBox="0 0 16 16"><circle cx="8" cy="8" r="6.5"/><path d="M5.2 8.2l1.9 1.9 3.7-3.9"/></svg>'
HALF = '<svg viewBox="0 0 16 16"><circle cx="8" cy="8" r="6.5"/><path class="fill" d="M8 1.5a6.5 6.5 0 0 1 0 13z"/></svg>'
EMPTY = '<svg viewBox="0 0 16 16"><circle cx="8" cy="8" r="6.5"/></svg>'
KEY = ('        <div class="key" aria-hidden="true"><span>' + DONE + 'Done</span><span>' + HALF
       + 'In progress</span><span>' + EMPTY + 'Not started</span></div>')
ACTIVE = ("in_progress", "in_review")
WAITING = ("backlog", "todo", "needs_decision")
THEMES_SHOWN = 6
SOURCE_EXT = {"rs", "swift", "kt", "kts", "ts", "mjs", "py", "sh", "go", "sql", "proto"}
SKIP_DIRS = {"fixtures", "baseline", "vendor", "third_party", "node_modules"}
USAGE_DBS = ["~/Library/Application Support/com.farcooler.FarCoolerCanary/farcooler.db",
             "~/Library/Application Support/com.farcooler.FarCooler/farcooler.db"]
TOKENS_FROM = """
  FROM agent_turns t JOIN agent_turn_models m ON m.turn_id = t.id
 WHERE t.harness = 'claude'
   AND (t.repository_id = (SELECT repository_id FROM workspaces WHERE task_prefix = ?1)
        OR (t.repository_id IS NULL
            AND t.task_id IN (SELECT id FROM tasks
                               WHERE workspace_id = (SELECT id FROM workspaces WHERE task_prefix = ?1))))
"""
TOKEN_SUM = "sum(m.input_tokens + m.output_tokens + m.cache_read_tokens + m.cache_write_tokens)"
TOKENS_SQL = "SELECT coalesce(" + TOKEN_SUM + ", 0)" + TOKENS_FROM
# The same turns by the UTC day each one ended (ended_at is milliseconds since the epoch).
TOKENS_DAILY_SQL = ("SELECT date(t.ended_at / 1000, 'unixepoch'), coalesce(" + TOKEN_SUM + ", 0)"
                    + TOKENS_FROM + " GROUP BY 1")


class Refusal(Exception):
    pass


def n(v):
    return f"{v:,}"


def short(d):
    return f"{MONTHS[d.month - 1][:3]} {d.day}"


def long_date(d):
    return f"{MONTHS[d.month - 1]} {d.day}"


def word(v):
    return WORDS[v] if v < len(WORDS) else str(v)


def text(s):
    """Board text as the page spells it: escaped, with typographic apostrophes."""
    return html.escape(s, quote=False).replace("'", "&rsquo;")


def counts_as_source(path):
    """The SOURCE RULE in the module docstring."""
    parts = path.split("/")
    if any(p in SKIP_DIRS or p.endswith("-fixtures") for p in parts[:-1]):
        return False
    return "." in parts[-1] and parts[-1].rsplit(".", 1)[1] in SOURCE_EXT


def source_lines(ref, cwd=ROOT):
    """Lines in the tracked source files at `ref`, read from git's object store."""
    names = [p for p in run(["git", "ls-tree", "-r", "--name-only", "-z", ref], cwd).split("\0")
             if p and counts_as_source(p)]
    proc = subprocess.run(["git", "cat-file", "--batch"], cwd=cwd, check=True, capture_output=True,
                          input=("\n".join(f"{ref}:{p}" for p in names) + "\n").encode())
    data, pos, total = proc.stdout, 0, 0
    for p in names:
        eol = data.index(b"\n", pos)
        head = data[pos:eol].split()
        if len(head) != 3:
            raise Refusal(f"git could not read {p}")
        size = int(head[2])
        body = data[eol + 1:eol + 1 + size]
        total += body.count(b"\n") + (1 if body and not body.endswith(b"\n") else 0)
        pos = eol + 1 + size + 1
    return total


def usage_query(sql, prefix, db):
    """Rows of `sql` against the runner's usage database, opened read-only."""
    env = os.environ.get("SITE_PROOF_USAGE_DB")
    paths = [db] if db else [env] if env else USAGE_DBS
    for p in paths:
        p = os.path.expanduser(p)
        if os.path.exists(p):
            con = sqlite3.connect(f"file:{p}?mode=ro", uri=True)
            try:
                return con.execute(sql, (prefix,)).fetchall()
            except sqlite3.Error as e:
                raise Refusal(f"could not read token usage from {p}: {e}")
            finally:
                con.close()
    raise Refusal("no usage database found; set SITE_PROOF_USAGE_DB")


def tokens_used(prefix="ov", db=None):
    """All-time Claude Code tokens for the board with this task prefix."""
    return int(usage_query(TOKENS_SQL, prefix, db)[0][0])


def tokens_by_week(weeks, prefix="ov", db=None):
    """Tokens in each charted week, by the UTC day each turn ended; weeks start Sunday."""
    per = {}
    for day, total in usage_query(TOKENS_DAILY_SQL, prefix, db):
        w = week_start(dt.date.fromisoformat(day))
        per[w] = per.get(w, 0) + int(total)
    return [per.get(w, 0) for w, _ in weeks]


def loc_by_week(weeks, ref, cwd=ROOT, count=None):
    """Lines of code on `ref` at the end of each charted week: the last commit on the
    first-parent line at or before midnight UTC after the week's Saturday. A week with
    no commits carries the week before. The tree is read from the object store."""
    count = count or (lambda sha: source_lines(sha, cwd))
    seen, out = {}, []
    for w, _ in weeks:
        end = (w + dt.timedelta(days=7)).isoformat()
        sha = run(["git", "rev-list", "-1", "--first-parent", f"--before={end}T00:00:00+0000", ref], cwd).strip()
        if sha and sha not in seen:
            seen[sha] = count(sha)
        out.append(seen.get(sha, 0))
    return out


def billions(v):
    if v >= 1e9:
        return f"{v / 1e9:.1f}B"
    return f"{round(v / 1e6)}M"


def week_start(d):
    return d - dt.timedelta(days=(d.weekday() + 1) % 7)


def weekly(dates, today):
    """[(week start, commits)] from the first commit's week to the last full week."""
    if not dates:
        raise Refusal("no commits to chart")
    last_full = week_start(today) - dt.timedelta(days=7)
    first = week_start(min(dates))
    counts = {}
    for d in dates:
        w = week_start(d)
        if w <= last_full:
            counts[w] = counts.get(w, 0) + 1
    weeks, w = [], first
    while w <= last_full:
        weeks.append((w, counts.get(w, 0)))
        w += dt.timedelta(days=7)
    return weeks


def ratio_phrase(after, before):
    r = after / before
    if r < 0.9:
        return "fewer than in"
    if r < 1.1:
        return "about as many as in"
    if r < 1.9 or r >= 2.1:
        return f"{r:.1f} times as many as in" if r < 10 else f"about {round(r)} times as many as in"
    return "almost twice as many as in" if r < 2 else "about twice as many as in"


def takeaway(weeks, data):
    pivot = week_start(dt.date.fromisoformat(data["pivot"]))
    after = [c for w, c in weeks if w > pivot]
    before = [c for w, c in weeks if w <= pivot]
    out = data["pivot_sentence"]
    if after and before and sum(before):
        a, b = len(after), len(before)
        out += (f" In the {word(a)} week{'s' if a != 1 else ''} after, main took {n(sum(after))} commits, "
                f"{ratio_phrase(sum(after), sum(before))} the {word(b)} week{'s' if b != 1 else ''} before.")
    return out


def compact(v):
    """200k, 604k, 412M, 2.4B: three digits or one decimal."""
    for size, suffix in ((1e9, "B"), (1e6, "M"), (1e3, "k")):
        if v >= size:
            x = v / size
            return (f"{x:.0f}" if x >= 100 else f"{x:.1f}".rstrip("0").rstrip(".")) + suffix
    return str(v)


def spoken(v):
    """A token count the way a screen reader should say it."""
    if v >= 1e9:
        return f"{v / 1e9:.1f}".rstrip("0").rstrip(".") + " billion"
    if v >= 1e6:
        return f"{v / 1e6:.0f} million"
    return n(v)


def nice_step(top, lines=5):
    """The smallest 1, 2 or 5 times a power of ten that puts at most `lines` steps under `top`."""
    unit = 1
    while True:
        for k in (1, 2, 5):
            if top <= lines * k * unit:
                return k * unit
        unit *= 10


def split_weeks(weeks, data):
    """Indexes of the weeks up to and including the pivot's week, and of those after it."""
    pivot = week_start(dt.date.fromisoformat(data["pivot"]))
    return ([i for i, (w, _) in enumerate(weeks) if w <= pivot],
            [i for i, (w, _) in enumerate(weeks) if w > pivot])


def plural(v, noun):
    return f"{word(v)} {noun}{'s' if v != 1 else ''}"


def loc_takeaway(weeks, lines, data):
    before, after = split_weeks(weeks, data)
    out = data["pivot_sentence"]
    if before and after:
        base, gain = lines[before[-1]], lines[-1] - lines[before[-1]]
        out += (f" In the {plural(len(after), 'week')} after, main {'gained' if gain >= 0 else 'lost'} {n(abs(gain))} lines, "
                f"against {n(base)} in the {plural(len(before), 'week')} before.")
    return out


def tokens_takeaway(weeks, used, data):
    before, after = split_weeks(weeks, data)
    since = dt.date.fromisoformat(data["tokens_since"])
    out = data["pivot_sentence"]
    tracked = [i for i in before if weeks[i][0] + dt.timedelta(days=6) >= since]
    if tracked and after:
        out += (f" In the {plural(len(after), 'week')} after, agents used {spoken(sum(used[i] for i in after))} tokens, "
                f"against {spoken(sum(used[i] for i in tracked))} in the {plural(len(tracked), 'week')} before. "
                f"Usage tracking began on {long_date(since)}.")
    return out


def series_specs(weeks, lines, used, data):
    """The three charts: what each shows, how each number is written, and where it ends."""
    span = f"{long_date(weeks[0][0])} to {long_date(weeks[-1][0] + dt.timedelta(days=6))}"
    since = dt.date.fromisoformat(data["tokens_since"])
    return [
        dict(key="commits", tab="Commits", values=[c for _, c in weeks], unit="commits", said="commits",
             head=f"{n(sum(c for _, c in weeks))} commits to main", sub=f"Per week, {span}",
             tip=n, say=n, peak=n, axis=n, column="Commits",
             take=takeaway(weeks, data)),
        dict(key="loc", tab="Lines of code", values=lines, unit="lines of code",
             said="lines of code at the end of the week",
             head=f"{n(lines[-1])} lines of code", sub=f"On main at the end of each week, {span}",
             tip=n, say=n, peak=compact, axis=compact, column="Lines of code at the end of the week",
             take=loc_takeaway(weeks, lines, data)),
        dict(key="tokens", tab="Tokens", values=used, unit="tokens", said="tokens",
             head=f"{spoken(sum(used))} tokens", sub=f"Per week, counted from {long_date(since)} to {long_date(weeks[-1][0] + dt.timedelta(days=6))}",
             tip=compact, say=spoken, peak=compact, axis=compact, column="Tokens",
             take=tokens_takeaway(weeks, used, data)),
    ]


def chart(weeks, milestones, spec):
    """The gridlines, the columns and the table rows of one series."""
    vals = spec["values"]
    top = max(vals)
    if spec["key"] == "commits":
        axis, step = max(100, -(-top // 100) * 100), 500
    else:
        step = nice_step(top)
        axis = max(step, -(-top // step) * step)
    pct = lambda v: f"{v / axis * 100:.2f}"
    grid = "".join(f'<span style="top:{100 - v / axis * 100:.2f}%" data-v="{spec["axis"](v)}"></span>'
                   for v in range(0, axis + 1, step))
    peak = max(range(len(vals)), key=lambda i: (vals[i], -i))
    lines = []
    for i, ((w, _), v) in enumerate(zip(weeks, vals)):
        name = f"Week of {short(w)}"
        h = pct(v)
        label = f'<b style="bottom:calc({h}% + 4px)">{spec["peak"](v)}</b>' if i == peak else ""
        badges = "".join(f'<span class="badge">{k}</span>' for k, m in enumerate(milestones, 1)
                         if week_start(dt.date.fromisoformat(m["date"])) == w)
        lines.append(
            f'              <div class="col" tabindex="0" role="img" data-week="{name}" data-v="{spec["tip"](v)}" '
            f'aria-label="{name}: {spec["say"](v)} {spec["said"]}"><div class="area"><i style="height:{h}%"></i>{label}</div>'
            f'<span class="x" aria-hidden="true">{short(w) if i % 2 == 0 else ""}</span>'
            f'<span class="ms" aria-hidden="true">{badges}</span></div>')
    rows = "".join(f'<tr><th scope="row">Week of {short(w)}</th><td>{n(v)}</td></tr>' for (w, _), v in zip(weeks, vals))
    return grid, "\n".join(lines), rows


def pick_cards(cards):
    """Three cards by the theme rule in the module docstring."""
    num = lambda c: int(c["key"].rsplit("-", 1)[1])
    live = sorted((c for c in cards if c["status"] in ACTIVE), key=num, reverse=True)
    done = sorted((c for c in cards if c["status"] == "done"), key=num, reverse=True)
    wait = sorted((c for c in cards if c["status"] in WAITING), key=num)
    lead = live + done
    chosen = lead[:2]
    chosen += wait[:3 - len(chosen)]
    chosen += lead[2:2 + 3 - len(chosen)]
    return chosen


def themes_html(plan, tasks):
    by_key = {t["key"]: t for t in tasks}
    open_themes = []
    for th in plan["themes"]:
        cards = [by_key[c["key"]] for c in th["cards"] if c["key"] in by_key]
        cards = [c for c in cards if c["status"] != "cancelled"]
        if any(c["status"] != "done" for c in cards):
            open_themes.append((th, cards))
    open_themes.sort(key=lambda p: (-p[0]["last_moved_at"], p[0]["name"]))
    out = []
    for th, cards in open_themes[:THEMES_SHOWN]:
        done = sum(1 for c in cards if c["status"] == "done")
        total = len(cards)
        out += ['        <div class="th">',
                f'          <div class="t"><b>{text(th["name"])}</b><span>{done} of {total} done</span></div>',
                f'          <div class="meter" role="img" aria-label="{done} of {total} cards done"><i style="width:{done / total * 100:.1f}%"></i></div>',
                "          <ul>"]
        for c in pick_cards(cards):
            icon, said = (DONE, "Done") if c["status"] == "done" else (
                (HALF, "In progress") if c["status"] in ACTIVE else (EMPTY, "Not started"))
            out.append(f'            <li>{icon}<span class="vh">{said}: </span><span>{text(c["title"])} '
                       f'<span class="k">{c["key"]}</span></span></li>')
        out += ["          </ul>", "        </div>"]
    return "\n".join(out)


def region(plan, tasks, dates, today, data, newest, lines, used):
    weeks = weekly(dates, today)
    ms = data["milestones"]
    cut = weeks[-1][0] + dt.timedelta(days=6)  # the stats and the chart end on the same day
    count_cols = f"repeat({len(weeks)}, minmax(0, 1fr))"
    panels, tables = [], []
    specs = series_specs(weeks, lines, used, data)
    for k, spec in enumerate(specs):
        grid, cols, rows = chart(weeks, ms, spec)
        panels += [
            f'        <div class="series" data-series="{spec["key"]}"{"" if k == 0 else " hidden"}>',
            f'          <h3>{spec["head"]}</h3>',
            f'          <p class="sub">{spec["sub"]}</p>',
            f'          <div class="plot" data-unit="{spec["unit"]}">',
            f'            <div class="grid" aria-hidden="true">{grid}</div>',
            f'            <div class="cols" style="grid-template-columns:{count_cols}">',
            cols,
            "            </div>",
            '            <div class="tip" aria-hidden="true"><strong></strong><span></span></div>',
            "          </div>",
            f'          <p class="takeaway">{text(spec["take"])}</p>',
            "        </div>"]
        tables.append(f'<table><caption>{spec["tab"]} per week</caption><thead><tr><th scope="col">Week</th>'
                      f'<th scope="col">{spec["column"]}</th></tr></thead><tbody>{rows}</tbody></table>')
    switch = ['        <div class="seg" role="group" aria-label="Chart" hidden>',
              *[f'          <button type="button" data-series="{sp["key"]}" aria-pressed="{"true" if k == 0 else "false"}">{sp["tab"]}</button>'
                for k, sp in enumerate(specs)],
              "        </div>"]
    count = lambda xs, s: sum(1 for x in xs if x["state"] == s)
    rulings = plan["rulings"]
    moments = []
    for k, m in enumerate(ms, 1):
        d = dt.date.fromisoformat(m["date"])
        moments += [f'          <!-- {m["date"]} · {m["source"]} -->',
                    f'          <li><span class="badge" aria-hidden="true">{k}</span><time datetime="{m["date"]}">{short(d)}</time><span>{text(m["label"])}</span></li>']
    return "\n".join([
        '      <figure class="history" style="margin-left:0;margin-right:0">',
        "        <div>",
        *switch,
        '        <div class="serieswrap">',
        *panels,
        "        </div>",
        f'        <div class="sr-only">{"".join(tables)}</div>',
        "        </div>",
        '        <ol class="moments">',
        *moments,
        "        </ol>",
        "      </figure>",
        "",
        '      <div class="stats">',
        f'        <div class="stat"><b>{n(lines[-1])}</b><span>lines of code on {long_date(cut)}</span></div>',
        f'        <div class="stat"><b>{billions(sum(used))}</b><span>tokens used building it through {long_date(cut)}</span></div>',
        f'        <div class="stat"><b>{n(plan["board_counts"]["done"])}</b><span>cards done</span></div>',
        f'        <div class="stat"><b>{n(count(plan["lanes"], "landed"))}</b><span>lanes landed</span></div>',
        f'        <div class="stat"><b>{n(count(plan["trains"], "landed"))}</b><span>trains landed</span></div>',
        f'        <div class="stat"><b>{n(len(rulings))}</b><span>decided for&nbsp;you, {n(count(rulings, "reversed"))}&nbsp;reversed</span></div>',
        "      </div>",
        "",
        '      <div class="building">',
        "        <h3>What it’s building now</h3>",
        KEY,
        '        <div class="themes">',
        themes_html(plan, tasks),
        "        </div>",
        "      </div>",
        f"      <p class=\"note\">Counted from this repository’s git history and board on {long_date(newest)}.</p>",
    ])


def splice(page, new_region):
    """The page with the region between the markers replaced; nothing else moves."""
    lines = page.split("\n")
    b = [i for i, l in enumerate(lines) if l.strip() == BEGIN.strip()]
    e = [i for i, l in enumerate(lines) if l.strip() == END.strip()]
    if len(b) != 1 or len(e) != 1 or b[0] > e[0]:
        raise Refusal(f"expected one begin marker before one end marker, found {len(b)} and {len(e)}")
    head, tail = "\n".join(lines[:b[0] + 1]), "\n".join(lines[e[0]:])
    out = head + "\n" + new_region + "\n" + tail
    if not (out.startswith(head + "\n") and out.endswith("\n" + tail)):
        raise Refusal("the rewrite would touch text outside the markers")
    return out


def summarize(old, new):
    """What moved between two regions, one phrase per line, for a commit message."""
    import re
    grab = lambda pat, t: re.findall(pat, t)
    out = []
    a, b = grab(r"<h3>([\d,]+) commits", old), grab(r"<h3>([\d,]+) commits", new)
    if a != b:
        out.append(f"- commits to main: {a[0] if a else 'none'} to {b[0] if b else 'none'}")
    sa, sb = grab(r"<b>([\d.,]+[BM]?)</b><span>([^<]*)</span></div>", old), grab(r"<b>([\d.,]+[BM]?)</b><span>([^<]*)</span></div>", new)
    strip = lambda t: re.sub(r" (on|through) [A-Z][a-z]+ \d+$", "", t)
    before = {strip(label): x for x, label in sa}
    for y, label in sb:
        label = strip(label)
        x = before.get(label)
        if x != y:
            out.append(f"- {html.unescape(label).replace(chr(160), ' ')}: {x if x else 'new'} to {y}")
    ta, tb = grab(r'<div class="t"><b>(.*?)</b><span>(.*?)</span>', old), grab(r'<div class="t"><b>(.*?)</b><span>(.*?)</span>', new)
    na, nb = [x for x, _ in ta], [x for x, _ in tb]
    for name in nb:
        if name not in na:
            out.append(f"- theme shown: {html.unescape(name)}")
    for name in na:
        if name not in nb:
            out.append(f"- theme no longer shown: {html.unescape(name)}")
    moved = sum(1 for name, prog in tb if (name, prog) not in ta and name in na)
    if moved:
        out.append(f"- progress changed on {moved} theme{'s' if moved != 1 else ''}")
    if not out:
        out.append("- chart, themes' cards or the date line")
    return out


def run(cmd, cwd=ROOT):
    return subprocess.run(cmd, cwd=cwd, check=True, capture_output=True, text=True).stdout


def load(args):
    """Inputs: from files when given (the tests), else from the board and git."""
    j = lambda p: json.load(open(p))
    plan = j(args["plan"]) if args.get("plan") else json.loads(run(["farcooler-canary", "plan", "--repo", "overnight", "--json"]))
    tasks = j(args["tasks"]) if args.get("tasks") else json.loads(run(["farcooler-canary", "task", "list", "--repo", "overnight", "--json"]))
    tasks = tasks["tasks"] if isinstance(tasks, dict) else tasks
    if args.get("commits"):
        raw = open(args["commits"]).read().split()
    else:
        raw = run(["git", "log", args.get("ref", "origin/main"), "--format=%ad", "--date=short"]).split()
    dates = [dt.date.fromisoformat(x) for x in raw]
    if not dates:
        raise Refusal("no commits found")
    today = dt.date.fromisoformat(args["today"]) if args.get("today") else dt.date.today()
    data = j(args.get("milestones") or MILESTONES)
    ref = args.get("ref", "origin/main")
    weeks = weekly(dates, today)
    ints = lambda key: [int(x) for x in args[key].split(",")]
    lines = ints("loc_weeks") if args.get("loc_weeks") else loc_by_week(weeks, ref)
    used = ints("tokens_weeks") if args.get("tokens_weeks") else tokens_by_week(weeks, db=args.get("usage_db"))
    if len(lines) != len(weeks) or len(used) != len(weeks):
        raise Refusal(f"the chart has {len(weeks)} weeks but {len(lines)} line counts and {len(used)} token counts")
    return plan, tasks, dates, today, data, max(dates), lines, used


def generate(args):
    path = args.get("page") or PAGE
    page = open(path, encoding="utf-8", newline="").read()
    out = splice(page, region(*load(args)))
    return path, page, out


def parse(argv):
    a, flags = {}, set()
    names = {"--page": "page", "--plan": "plan", "--tasks": "tasks", "--commits": "commits",
             "--milestones": "milestones", "--ref": "ref", "--today": "today",
             "--usage-db": "usage_db",
             "--loc-weeks": "loc_weeks", "--tokens-weeks": "tokens_weeks"}
    it = iter(argv)
    for x in it:
        if x in names:
            a[names[x]] = next(it)
        elif x in ("--check", "--self-test"):
            flags.add(x)
        else:
            raise SystemExit(f"site-proof: unknown argument {x}")
    return a, flags


def main(argv):
    a, flags = parse(argv)
    if "--self-test" in flags:
        return self_test()
    try:
        path, page, out = generate(a)
    except Refusal as r:
        print(f"site-proof: refused: {r}", file=sys.stderr)
        return 2
    if out == page:
        print("site-proof: unchanged")
        return 0
    if "--check" in flags:
        print("site-proof: the page is stale")
        return 1
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(out)
    print("site-proof: updated")
    print("\n".join(summarize(page, out)))
    return 0


# ---- self-test ----------------------------------------------------------------

FIX = os.path.join(HERE, "site-proof-fixtures")


def source_rule_and_tokens_tests(check):
    """The two counters against a throwaway git repo and a throwaway database."""
    files = {"a.rs": "x\ny\n", "b/c.swift": "one\ntwo\nthree", "lock/Cargo.lock": "no\n",
             "d.json": "no\n", "test/fixtures/e.rs": "no\n", "x/y-fixtures/f.py": "no\n",
             "m/vendor/g.go": "no\n", "include/h.h": "no\n", "README.md": "no\n", "empty.sh": ""}
    with tempfile.TemporaryDirectory() as tmp:
        git = lambda *a: subprocess.run(["git", "-c", "commit.gpgsign=false", *a], cwd=tmp, check=True,
                                         capture_output=True, text=True)
        git("init", "-q")
        for name, body in files.items():
            os.makedirs(os.path.dirname(os.path.join(tmp, name)), exist_ok=True)
            open(os.path.join(tmp, name), "w").write(body)
        git("add", ".")
        git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "x")
        open(os.path.join(tmp, "a.rs"), "w").write("dirty\n" * 50)  # the working tree never counts
        check(source_lines("HEAD", tmp) == 5, "lines of code: source counts; fixtures, vendor, lockfiles, JSON and docs do not")
        db = os.path.join(tmp, "u.db")
        ms = lambda y, m, d, hh, mm, ss: int(dt.datetime(y, m, d, hh, mm, ss, tzinfo=dt.timezone.utc).timestamp() * 1000)
        ended = [ms(2026, 10, 3, 23, 59, 59), ms(2026, 10, 4, 0, 0, 0)] + [ms(2026, 10, 5, 12, 0, 0)] * 3
        con = sqlite3.connect(db)
        con.executescript("""
            CREATE TABLE workspaces(id, task_prefix, repository_id);
            CREATE TABLE tasks(id, workspace_id);
            CREATE TABLE agent_turns(id, harness, repository_id, task_id, ended_at);
            CREATE TABLE agent_turn_models(turn_id, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens);
            INSERT INTO workspaces VALUES (1,'ov',10),(2,'zz',20);
            INSERT INTO tasks VALUES (100,1),(200,2);
            INSERT INTO agent_turns VALUES (1,'claude',10,NULL,{}),(2,'claude',NULL,100,{}),(3,'claude',20,NULL,{}),
                                           (4,'claude',NULL,200,{}),(5,'codex',10,NULL,{});
            INSERT INTO agent_turn_models VALUES (1,1,2,3,4),(2,10,0,0,0),(3,1000,0,0,0),(4,1000,0,0,0),(5,1000,0,0,0);
        """.format(*ended))
        con.commit()
        con.close()
        check(tokens_used("ov", db) == 20, "tokens: this board's Claude turns, filed or by card, and no one else's")
        weeks = [(dt.date(2026, 9, 27), 0), (dt.date(2026, 10, 4), 0), (dt.date(2026, 10, 11), 0)]
        check(tokens_by_week(weeks, "ov", db) == [10, 10, 0],
              "tokens by week: a turn counts in the UTC week it ended, weeks start Sunday")
    with tempfile.TemporaryDirectory() as tmp:
        def commit(name, body, when):
            env = dict(os.environ, GIT_AUTHOR_DATE=when, GIT_COMMITTER_DATE=when)
            open(os.path.join(tmp, name), "w").write(body)
            for cmd in (["add", name], ["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false",
                                        "commit", "-q", "-m", name]):
                subprocess.run(["git", *cmd], cwd=tmp, check=True, capture_output=True, env=env)
        subprocess.run(["git", "init", "-q"], cwd=tmp, check=True)
        commit("a.rs", "1\n2\n", "2026-09-29T12:00:00+0000")
        commit("b.rs", "1\n2\n3\n", "2026-10-04T00:30:00+0000")
        weeks = [(dt.date(2026, 9, 20), 0), (dt.date(2026, 9, 27), 0), (dt.date(2026, 10, 4), 0), (dt.date(2026, 10, 11), 0)]
        check(loc_by_week(weeks, "HEAD", tmp) == [0, 2, 5, 5],
              "lines by week: main as of each week's end; an empty week carries the one before")


def self_test():
    """Two checks, each able to fail: the region equals the golden file the
    fixture produces, and nothing outside the markers is touched."""
    failures = []

    def check(ok, what):
        print(("ok   " if ok else "FAIL ") + what)
        if not ok:
            failures.append(what)

    args = {k: os.path.join(FIX, v) for k, v in
            {"plan": "plan.json", "tasks": "tasks.json", "commits": "commits.txt",
             "milestones": "milestones.json"}.items()}
    args["today"] = "2026-10-11"
    args["loc_weeks"], args["tokens_weeks"] = "100,100,150,400,520", "0,300000000,450000000,2400000000,3100000000"
    with tempfile.TemporaryDirectory() as tmp:
        page = os.path.join(tmp, "index.html")
        src = open(os.path.join(FIX, "page.html"), encoding="utf-8", newline="").read()
        open(page, "w", encoding="utf-8", newline="").write(src)
        args["page"] = page
        golden = open(os.path.join(FIX, "expected.html"), encoding="utf-8", newline="").read()

        _, _, out = generate(args)
        check(out == golden, "the region matches the generator's golden output for the fixture")

        # Each series renders, and the switch is there.
        import re
        panels = re.findall(r'<div class="series" data-series="(\w+)"( hidden)?>', out)
        check([k for k, _ in panels] == ["commits", "loc", "tokens"] and [bool(h) for _, h in panels] == [False, True, True],
              "three series render, and only commits shows without script")
        tiles = re.findall(r'<div class="stat"><b>([^<]*)</b><span>(lines of code|tokens used)[^<]*</span>', out)
        check(tiles == [("520", "lines of code"), ("6.2B", "tokens used")],
              "the stat tiles print the chart headlines' numbers, cut at the same day")
        check(re.findall(r"<h3>([^<]*)</h3>", out)[:3] == ["12 commits to main", "520 lines of code", "6.2 billion tokens"],
              "each series has its own headline")
        check(re.findall(r'<div class="seg" role="group" aria-label="Chart" hidden>(.*?)</div>', out, re.S)
              and re.findall(r'<button type="button" data-series="(\w+)" aria-pressed="(\w+)">([^<]*)</button>', out)
              == [("commits", "true", "Commits"), ("loc", "false", "Lines of code"), ("tokens", "false", "Tokens")],
              "the switch has Commits, Lines of code and Tokens, Commits pressed, and starts hidden")
        check(len(re.findall(r'<div class="col" ', out)) == 15 and out.count("repeat(5, minmax(0, 1fr))") == 3,
              "each series draws one column per week, five here")
        check(re.findall(r"<caption>([^<]*)</caption>", out) == ["Commits per week", "Lines of code per week", "Tokens per week"]
              and out.count('<th scope="row">Week of Oct 4</th>') == 3,
              "each series has its table rows for screen readers")
        check('data-v="3.1B"' in out and 'aria-label="Week of Oct 4: 3.1 billion tokens"' in out
              and 'aria-label="Week of Oct 4: 520 lines of code at the end of the week"' in out,
              "the values read right in the tooltip and to a screen reader")
        check("main gained 370 lines, against 150 in the three weeks before." in out
              and "agents used 5.5 billion tokens, against 750 million in the two weeks before. Usage tracking began on September 14." in out,
              "the lines and tokens takeaways state their numbers")

        # Outside the markers: every byte before and after survives, whatever the region held.
        b, e = out.index(BEGIN), out.index(END)
        check(out[:b] == src[:src.index(BEGIN)] and out[e:] == src[src.index(END):],
              "text outside the markers is byte-identical")
        check("SENTINEL-BEFORE" in out[:b] and "SENTINEL-AFTER" in out[e:],
              "the sentinels outside the markers survive")

        # Idempotent: a second run on its own output changes nothing.
        open(page, "w", encoding="utf-8", newline="").write(out)
        _, _, again = generate(args)
        check(again == out, "a second run leaves the page byte-identical")

        # Refusals leave the file alone.
        for name, broken in {
            "no begin marker": src.replace(BEGIN, "<!-- gone -->"),
            "no end marker": src.replace(END, "<!-- gone -->"),
            "markers reversed": src.replace(BEGIN, "@@").replace(END, BEGIN).replace("@@", END),
            "begin marker twice": src.replace(BEGIN, BEGIN + "\n" + BEGIN),
        }.items():
            open(page, "w", encoding="utf-8", newline="").write(broken)
            code = main(["--page", page] + [x for k, v in args.items() if k != "page" for x in ("--" + k.replace("_", "-"), v)])
            check(code == 2 and open(page, encoding="utf-8", newline="").read() == broken,
                  f"refuses and leaves the page alone: {name}")
    source_rule_and_tokens_tests(check)
    if failures:
        print(f"site-proof: {len(failures)} self-test failure(s)", file=sys.stderr)
        return 1
    print("site-proof: self-test passed")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
