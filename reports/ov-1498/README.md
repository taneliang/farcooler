# ov-1498 — Mid-turn message acknowledgement

The web chat shows “Message received and queued.” on a message as soon as the
runner’s transcript feed reports it as Waiting. When the row advances, chat
shows the new state. A compose reply contains a queue flag without a message
ID, so the acknowledgement comes from the transcript row and stays in sync
regardless of which response arrives first. The manager skill also asks the
orchestrator to acknowledge a message in one visible line at its next step and
to keep turns short. The workspace charter can override that default.

## Baseline

The ov-1494 diagnosis (`.claude/agent/reports/ov-1494/diagnosis.md`, section 1,
candidate 1) analyzed the orchestrator’s Claude Code transcript starting
Nov 26, 2026 at 08:00 Singapore time (00:00 UTC). Of about 50 owner messages,
37 started their own turns and 13 were absorbed into a running turn. All 13
had matching `queued_command` attachments
showing that they reached the model. For all 48 messages that could be timed,
the first reply text came at turn end. Across 146 turns, the median turn
lasted 61 s and the longest ran 32 min 26 s.

| Transcript measure | Started a turn | Absorbed mid-turn |
| --- | ---: | ---: |
| Median time to first reply text | 31 s | 66 s |
| Longest time to first reply text | 203 s | 433 s |

The first column includes messages that queued and then started a new turn.
Its 90th-percentile wait was 124 s. The sample covers messages recorded by
Claude Code.

## Repeat the measurement

After deploying the web client and starting an orchestrator with the updated
manager skill and default charter behavior, take a comparable window from the
same workspace’s transcript. Use timestamps and hashes of message text to
match records without exporting the text.

1. For a direct prompt, start the clock at its user record. For a queued prompt,
   start at its matching `queue-operation enqueue`. Count prompts that start
   their own turn separately from those with `remove` and
   `reason: absorbed_mid_turn`. Match the latter to the model’s
   `queued_command` attachments and measure enqueue to absorption as well.
2. End the clock at the first visible assistant text after the message
   reached the model. Include a one-line acknowledgement and record
   whether it came before the final `end_turn` message. Report untimed
   messages separately and compare counts, median and maximum by group. Use
   `turn_duration` records to compare turn lengths.
3. Measure the web receipt separately if timing the browser. In a chat with a
   busy agent, record Send to the first display of the queue acknowledgement
   for a confirmed queued compose. The transcript baseline measures agent
   reply latency, starting when the message is recorded by Claude Code.
