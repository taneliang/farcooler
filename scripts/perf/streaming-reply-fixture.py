#!/usr/bin/env python3
"""A fixture for StreamingReplyStallTests (ov-382): three finished turns, then
an agent reply of N characters that's still open, for fake-agent-follow.py to
stream into. The prose is the generator the ov-358 forensics measured with.

    scripts/perf/streaming-reply-fixture.py 25600 /tmp/fc-t/<lane>/r25.json [prose]

With `prose`, the open reply has no lists: one prose run of paragraphs, the
shape that's costliest to settle.
"""
import json, sys
words = "the quick brown fox jumps over a lazy dog while `code` and **bold** text with [links](https://example.com) flow by in a reply".split(" ")
def prose(chars, seed, lists=True):
    out = ""; i = seed
    while len(out) < chars:
        out += words[i % len(words)] + " "; i += 7
        if i % 23 == 0: out += "\n\n"
        if lists and i % 97 == 0: out += "\n\n- a bullet item\n- another one\n\n"
    return out
J = lambda o: json.dumps(o, separators=(",", ":"))
ev = []
for t in range(3):
    ev.append(J({"Message": {"role": "User", "text": f"Please do task number {t} and explain it.", "parent": None}}))
    for k in range(6):
        tid = f"toolu_{t}_{k}"
        ev.append(J({"ToolCall": {"id": tid, "title": "Terminal", "kind": "execute", "status": "Pending", "locations": [], "subagent": False}}))
        out = "\n".join(f"line {i} of output from tool {k}: " + "x" * 60 for i in range(60))
        ev.append(J({"ToolUpdate": {"id": tid, "status": "Completed", "title": f"cargo test -p thing {k}", "content": out, "diff": None, "locations": []}}))
    ev.append(J({"Message": {"role": "Agent", "text": prose(2400, t * 3), "parent": None}}))
    ev.append(J({"TurnEnded": {"reason": "EndTurn"}}))
ev.append(J({"Message": {"role": "User", "text": "Write it all up.", "parent": None}}))
ev.append(J({"Message": {"role": "Agent", "text": prose(int(sys.argv[1]), 5, lists=sys.argv[3:] != ["prose"]), "parent": None}}))
json.dump(ev, open(sys.argv[2], "w"))
