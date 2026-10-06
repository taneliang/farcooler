#!/usr/bin/env python3
"""Stands in for `farcooler terminal agent-subscribe ... --follow` (and the
one-shot form) for StreamingReplyStallTests (ov-382; first written for the
ov-358 forensics): prints the batch lines `agent_follow.rs::line` would.

FC_FX: a JSON list of event payloads (streaming-reply-fixture.py writes one).
FC_FX_DELAY: seconds before the first line, as a link takes to set up.
FC_FX_STREAM: how many batches to stream after it, one per 200 ms, each five
words appended to the last row (role FC_FX_ROLE, Agent by default).
Exits when the app that started it goes away."""
import json, os, sys, time

args = sys.argv[1:]
if 'agent-subscribe' not in args:
    print('{}')
    sys.exit(0)
frm = int(args[args.index('--from-seq') + 1])
epoch = int(args[args.index('--epoch') + 1])
payloads = json.load(open(os.environ['FC_FX']))
time.sleep(float(os.environ.get('FC_FX_DELAY', '0')))
E = 1


def line(evs):
    sys.stdout.write(json.dumps({"epoch": E, "events": evs}) + "\n")
    sys.stdout.flush()


start = 0 if epoch != E else frm
line([{"seq": i, "payloadJson": p} for i, p in enumerate(payloads) if i >= start])
if '--follow' not in args:
    sys.exit(0)
seq = len(payloads)
n = int(os.environ.get('FC_FX_STREAM', '0'))
ppid = os.getppid()
role = os.environ.get("FC_FX_ROLE", "Agent")
words = "streaming words arrive a few at a time while the agent writes its reply ".split()
for k in range(n):
    time.sleep(0.2)
    if os.getppid() != ppid:
        sys.exit(0)
    evs = []
    for j in range(5):
        text = words[(k * 5 + j) % len(words)] + " "
        evs.append({"seq": seq, "payloadJson": json.dumps({"Message": {"role": role, "text": text, "parent": None}})})
        seq += 1
    line(evs)
while os.getppid() == ppid:
    time.sleep(0.5)
