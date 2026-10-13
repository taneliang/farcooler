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
            "tokens_since", the first day of transcripts worth counting (a fact
            about ~/.claude, not computed).
  Stats     `farcooler-canary plan --repo overnight --json`: board_counts.done
            (cards done), lanes and trains in state "landed", and every ruling
            (decided for you) with the reversed ones counted apart. Two more
            come from elsewhere:
              Both tiles below stop where the chart does: the last day of the
              last full week, so each equals its chart headline.
              Lines of code  lines ADDED to source files, per week and in total:
                insertions from `git log --first-parent --numstat` on origin/main,
                bucketed by author date like the commits. Deletions subtract
                nothing. SOURCE RULE: a file counts when its
                extension is rs, swift, kt, kts, ts, mjs, py, sh, go, sql or
                proto, and no directory on its path is named fixtures, *-fixtures,
                baseline, vendor, third_party or node_modules. Every added line
                counts, blank and comment lines too, tests too. Left out on purpose:
                lockfiles, JSON, Markdown, the generated C headers under
                include/, the site's HTML, fixtures and anything vendored.
              Tokens used  every Claude Code token (input, output, cache read
                and cache write) that Claude Code recorded for this repository,
                per week, read from its session transcripts: the *.jsonl files
                under ~/.claude/projects (SITE_PROOF_TRANSCRIPTS overrides),
                subagent runs included, read-only. A session counts when its
                cwd is inside the repository or one of its worktrees. A message
                streamed in several lines is one message, found by message.id
                (else requestId), with its largest usage. Weeks are the UTC
                weeks of the message timestamps. Messages before "tokens_since"
                are dropped. The runner's database is not used: it misses the
                interactive sessions.
              Claude Code deletes transcripts after 30 days, so a finished
                week is FROZEN in scripts/site-proof-tokens.json ({week start:
                tokens}) and read from there. Only the current week and the one
                before are recounted; each is stored as the larger of the file's
                number and the recount, so pruning never lowers a number. The
                generator rewrites that file and the refresher commits it with
                the page.
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
import glob
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
TOKENS_FILE = os.path.join(HERE, "site-proof-tokens.json")
TRANSCRIPTS = "~/.claude/projects"


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


def added_by_week(weeks, ref, cwd=ROOT):
    """Lines added to source files on `ref`'s first-parent line in each charted week, by
    author date like the commit counts. Insertions only: a deleted or rewritten line
    subtracts nothing. Files by the SOURCE RULE; binary files have no line count."""
    log = run(["git", "-c", "core.quotepath=off", "log", "--first-parent", "-m", "--no-renames", "--numstat",
               "--format=@%ad", "--date=short", ref], cwd)
    per, day = {}, None
    for line in log.split("\n"):
        if line.startswith("@"):
            day = week_start(dt.date.fromisoformat(line[1:]))
        elif line.count("\t") == 2:
            ins, _, path = line.split("\t")
            if ins != "-" and counts_as_source(path):
                per[day] = per.get(day, 0) + int(ins)
    return [per.get(w, 0) for w, _ in weeks]


def repo_root():
    """The main checkout's directory, from any of its worktrees."""
    common = run(["git", "rev-parse", "--path-format=absolute", "--git-common-dir"]).strip()
    return os.path.dirname(common)


def in_repo(cwd, root):
    """A session's cwd is the repo, inside it (its worktrees live under .claude/worktrees),
    or inside a Far Cooler-managed worktree of it."""
    return bool(cwd) and (cwd == root or cwd.startswith(root + "/")
                          or ("com.farcooler" in cwd and f"/worktrees/{os.path.basename(root)}" in cwd))


def transcript_tokens(since, root=None, repo=None):
    """{week start: tokens} from Claude Code's transcripts. Opens files read-only."""
    root = os.path.expanduser(root or os.environ.get("SITE_PROOF_TRANSCRIPTS") or TRANSCRIPTS)
    repo = repo or repo_root()
    seen = {}
    for path in glob.glob(os.path.join(root, "**", "*.jsonl"), recursive=True):
        try:
            with open(path, errors="replace") as f:
                records = []
                for line in f:
                    try:
                        records.append(json.loads(line))
                    except ValueError:
                        pass
        except OSError:
            continue
        cwd = next((r["cwd"] for r in records if isinstance(r, dict) and r.get("cwd")), None)
        if not in_repo(cwd, repo):
            continue
        for r in records:
            m = r.get("message") if isinstance(r, dict) else None
            if r.get("type") != "assistant" or not isinstance(m, dict) or not isinstance(m.get("usage"), dict):
                continue
            u = m["usage"]
            total = sum(u.get(k) or 0 for k in ("input_tokens", "output_tokens",
                                                "cache_creation_input_tokens", "cache_read_input_tokens"))
            key = m.get("id") or r.get("requestId") or r.get("uuid")
            if key and r.get("timestamp") and (key not in seen or total > seen[key][1]):
                seen[key] = (r["timestamp"], total)
    per = {}
    for ts, total in seen.values():
        day = dt.datetime.fromisoformat(ts.replace("Z", "+00:00")).astimezone(dt.timezone.utc).date()
        if day >= since:
            per[week_start(day)] = per.get(week_start(day), 0) + total
    return per


def read_frozen(path):
    if not os.path.exists(path):
        return {}
    return {dt.date.fromisoformat(k): int(v) for k, v in json.load(open(path)).items()}


def freeze_tokens(frozen, live, today):
    """The week-to-tokens map after this run. A week that is over is the file's number; only the
    current week and the one before are recounted, each kept at the larger of the two."""
    out = dict(frozen)
    this = week_start(today)
    for w in (this - dt.timedelta(days=7), this):
        out[w] = max(frozen.get(w, 0), live.get(w, 0))
    return out


def frozen_text(frozen):
    return json.dumps({str(w): v for w, v in sorted(frozen.items())}, indent=1) + "\n"


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


def loc_takeaway(weeks, added, data):
    before, after = split_weeks(weeks, data)
    out = data["pivot_sentence"]
    if before and after:
        a, b = sum(added[i] for i in after), sum(added[i] for i in before)
        out += (f" In the {plural(len(after), 'week')} after, main added {n(a)} lines, {n(round(a / len(after)))} a week. "
                f"In the {plural(len(before), 'week')} before, it averaged {n(round(b / len(before)))} a week.")
    return out


def tokens_takeaway(weeks, used, data):
    before, after = split_weeks(weeks, data)
    had = [i for i in before if used[i] > 0]
    out = data["pivot_sentence"]
    if had and after:
        avg = lambda idx: sum(used[i] for i in idx) / len(idx)
        out += (f" In the {plural(len(after), 'week')} after, agents used {spoken(avg(after))} tokens a week on average. "
                f"In the {plural(len(had), 'week')} before that had usage, they averaged {spoken(avg(had))} a week.")
    return out


def series_specs(weeks, lines, used, data):
    """The three charts: what each shows, how each number is written, and where it ends."""
    span = f"{long_date(weeks[0][0])} to {long_date(weeks[-1][0] + dt.timedelta(days=6))}"
    first = next((w for (w, _), v in zip(weeks, used) if v), weeks[0][0])
    first = max(first, dt.date.fromisoformat(data["tokens_since"]))
    return [
        dict(key="commits", tab="Commits", values=[c for _, c in weeks], unit="commits", said="commits",
             head=f"{n(sum(c for _, c in weeks))} commits to main", sub=f"Per week, {span}",
             tip=n, say=n, peak=n, axis=n, column="Commits",
             take=takeaway(weeks, data)),
        dict(key="loc", tab="Lines of code", values=lines, unit="lines of code",
             said="lines of code added",
             head=f"{n(sum(lines))} lines of code shipped", sub=f"Added to main per week, {span}",
             tip=n, say=n, peak=compact, axis=compact, column="Lines of code added",
             take=loc_takeaway(weeks, lines, data)),
        dict(key="tokens", tab="Tokens", values=used, unit="tokens", said="tokens",
             head=f"{spoken(sum(used))} tokens used", sub=f"Used per week, counted from {long_date(first)} to {long_date(weeks[-1][0] + dt.timedelta(days=6))}",
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
        f'        <div class="stat"><b>{n(sum(lines))}</b><span>lines of code shipped through {long_date(cut)}</span></div>',
        f'        <div class="stat"><b>{billions(sum(used))}</b><span>tokens used through {long_date(cut)}</span></div>',
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
    lines = ints("loc_weeks") if args.get("loc_weeks") else added_by_week(weeks, ref)
    if args.get("tokens_weeks"):
        used = ints("tokens_weeks")
    else:
        path = args.get("tokens_file") or TOKENS_FILE
        live = transcript_tokens(dt.date.fromisoformat(data["tokens_since"]), args.get("transcripts"), args.get("repo"))
        frozen = freeze_tokens(read_frozen(path), live, today)
        used = [frozen.get(w, 0) for w, _ in weeks]
        args["_tokens"] = (path, frozen_text(frozen))
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
             "--tokens-file": "tokens_file", "--transcripts": "transcripts", "--repo": "repo",
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
    tokens = a.get("_tokens")
    if tokens and "--check" not in flags and (not os.path.exists(tokens[0]) or open(tokens[0]).read() != tokens[1]):
        with open(tokens[0], "w") as f:
            f.write(tokens[1])
        print("site-proof: token counts updated")
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
    with tempfile.TemporaryDirectory() as tmp:
        def commit(files, when, deletes=()):
            env = dict(os.environ, GIT_AUTHOR_DATE=when, GIT_COMMITTER_DATE=when)
            for name, body in files.items():
                os.makedirs(os.path.dirname(os.path.join(tmp, name)), exist_ok=True)
                open(os.path.join(tmp, name), "w").write(body)
            for name in deletes:
                os.remove(os.path.join(tmp, name))
            for cmd in (["add", "-A", "."], ["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false",
                                             "commit", "-q", "-m", "x"]):
                subprocess.run(["git", *cmd], cwd=tmp, check=True, capture_output=True, env=env)
        subprocess.run(["git", "init", "-q"], cwd=tmp, check=True)
        # Week of Sep 27: two source files (5 lines; the last has no trailing newline) and every
        # kind the SOURCE RULE leaves out.
        commit({"a.rs": "x\ny\n", "b/c.swift": "one\ntwo\nthree", "lock/Cargo.lock": "no\n",
                "d.json": "no\n", "test/fixtures/e.rs": "no\n", "x/y-fixtures/f.py": "no\n",
                "m/vendor/g.go": "no\n", "include/h.h": "no\n", "README.md": "no\n", "empty.sh": ""},
               "2026-09-29T12:00:00+0000")
        # Week of Oct 4: a.rs loses 2 lines and gains 3 (counts 3); a markdown file grows (ignored).
        commit({"a.rs": "new1\nnew2\nnew3\n", "README.md": "no\nno\nno\n"}, "2026-10-05T12:00:00+0000")
        # Same week: an add-only commit (4), then a commit that only deletes source (adds 0).
        commit({"add.rs": "1\n2\n3\n4\n"}, "2026-10-06T12:00:00+0000")
        commit({}, "2026-10-07T12:00:00+0000", deletes=["b/c.swift"])
        with open(os.path.join(tmp, "a.rs"), "w") as dirty:
            dirty.write("dirty\n" * 50)  # the working tree never counts
        weeks = [(dt.date(2026, 9, 20), 0), (dt.date(2026, 9, 27), 0), (dt.date(2026, 10, 4), 0), (dt.date(2026, 10, 11), 0)]
        check(added_by_week(weeks, "HEAD", tmp) == [0, 5, 7, 0],
              "lines added by week: source insertions only; deletions subtract nothing; fixtures, vendor, lockfiles, JSON and docs are ignored")


def transcript_tests(check):
    """Token counting against a throwaway ~/.claude/projects and a throwaway frozen file."""
    repo = "/work/repo"
    day = lambda d, h=12: f"2026-{d}T{h:02d}:00:00.000Z"

    def msg(id_, ts, inp, out=0, cw=0, cr=0, cwd=None, req=None, kind="assistant"):
        r = {"type": kind, "timestamp": ts, "requestId": req,
             "message": {"id": id_, "usage": {"input_tokens": inp, "output_tokens": out,
                                              "cache_creation_input_tokens": cw, "cache_read_input_tokens": cr}}}
        if cwd:
            r["cwd"] = cwd
        return json.dumps(r)

    with tempfile.TemporaryDirectory() as tmp:
        def write(rel, lines):
            path = os.path.join(tmp, rel)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            open(path, "w").write("\n".join(lines) + "\n")
        # Week of Sep 27 (UTC): 100 in, 10 out, 5 cache write, 1000 cache read = 1115 once; the same
        # message streamed twice counts once at its largest; a second message has no id, only a requestId.
        write("p/a.jsonl", [msg("m1", day("09-29"), 100, 5, cr=1000, cwd=repo + "/.claude/worktrees/x"),
                            msg("m1", day("09-29"), 100, 10, cw=5, cr=1000),
                            msg("m1", day("09-29"), 100, 2, cr=1000),
                            msg(None, day("09-30"), 20, req="r1"), msg(None, day("09-30"), 20, req="r1"),
                            msg("u", day("09-30"), 999, kind="user"), "not json"])
        # A subagent file in the same project; the cwd is in its first record. Sunday 00:00 UTC opens Oct 4's week.
        write("p/s/subagents/agent-1.jsonl", [msg("m2", "2026-10-04T00:00:00.000Z", 300, cwd=repo),
                                              msg("m3", "2026-10-03T23:59:59.000Z", 7)])
        # Outside the repo, and a message from before tokens_since.
        write("q/b.jsonl", [msg("m4", day("09-29"), 5000, cwd="/work/other")])
        write("p/old.jsonl", [msg("m5", day("08-24"), 40, cwd=repo)])
        live = transcript_tokens(dt.date(2026, 8, 28), tmp, repo)
        check(live == {dt.date(2026, 9, 27): 1115 + 20 + 7, dt.date(2026, 10, 4): 300},
              "transcripts: duplicates and streamed repeats count once at the largest usage, a subagent file counts, "
              "another repo's session and pre-start messages do not, weeks are UTC Sundays")

        old, prev, this = dt.date(2026, 9, 20), dt.date(2026, 10, 4), dt.date(2026, 10, 11)
        today = dt.date(2026, 10, 13)
        frozen = {old: 900, dt.date(2026, 9, 27): 5000, prev: 200}
        got = freeze_tokens(frozen, {old: 50, dt.date(2026, 9, 27): 10, prev: 300, this: 40}, today)
        check(got[old] == 900 and got[dt.date(2026, 9, 27)] == 5000,
              "frozen: a finished week is the file's number, never recounted or lowered")
        check(got[prev] == 300 and got[this] == 40,
              "frozen: the current week and the one before are recounted")
        check(freeze_tokens({prev: 800}, {prev: 300}, today)[prev] == 800,
              "frozen: a recount that comes out lower (pruned transcripts) keeps the larger number")

        # The whole path through generate(): the chart reads the frozen file, the file is rewritten.
        path = os.path.join(tmp, "tokens.json")
        open(path, "w").write(frozen_text({dt.date(2026, 9, 6): 700, dt.date(2026, 9, 13): 0, dt.date(2026, 9, 20): 900,
                                           dt.date(2026, 9, 27): 5000, prev: 200}))
        args = {k: os.path.join(FIX, v) for k, v in
                {"plan": "plan.json", "tasks": "tasks.json", "commits": "commits.txt",
                 "milestones": "milestones.json"}.items()}
        args.update(today="2026-10-11", loc_weeks="1,1,1,1,1", tokens_file=path, transcripts=tmp, repo=repo,
                    page=os.path.join(FIX, "page.html"))
        _, _, out = generate(args)
        saved = json.loads(args["_tokens"][1])
        check(saved["2026-09-27"] == 5000 and saved["2026-10-04"] == 300 and saved["2026-09-06"] == 700
              and saved["2026-10-11"] == 0 and "<td>5,000</td>" in out and "<td>300</td>" in out,
              "generate: finished weeks come from the file, the live weeks from the transcripts, and the file is rewritten")


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
        tiles = re.findall(r'<div class="stat"><b>([^<]*)</b><span>(lines of code shipped|tokens used)[^<]*</span>', out)
        check(tiles == [("1,270", "lines of code shipped"), ("6.2B", "tokens used")],
              "the stat tiles print the chart headlines' numbers, cut at the same day")
        check(re.findall(r"<h3>([^<]*)</h3>", out)[:3] == ["12 commits to main", "1,270 lines of code shipped", "6.2 billion tokens used"],
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
              and 'aria-label="Week of Oct 4: 520 lines of code added"' in out,
              "the values read right in the tooltip and to a screen reader")
        check("main added 920 lines, 460 a week. In the three weeks before, it averaged 117 a week." in out
              and "agents used 2.8 billion tokens a week on average. In the two weeks before that had usage, they averaged 375 million a week." in out,
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
    transcript_tests(check)
    if failures:
        print(f"site-proof: {len(failures)} self-test failure(s)", file=sys.stderr)
        return 1
    print("site-proof: self-test passed")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
