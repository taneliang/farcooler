#!/usr/bin/env python3
"""Write the staged orchestrator's turn as a Claude Code transcript (ov-476).

    stage-demo-transcript.py <steps.jsonl> <session-id> <cwd> <claude-config-dir>

`scripts/stage-demo.sh` runs the orchestrator's commands for real and records
each one, with its output, in <steps.jsonl>. This writes them in the shape
claude writes its own session log (crates/core/fixtures/session-logs), at
<config>/projects/<slug of cwd>/<session>.jsonl, where the runner looks for a
pane's conversation. Records are spaced a few seconds apart, ending now.
"""

import json
import sys
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path

VERSION = "2.1.290"
MODEL = "claude-opus-5"


def main() -> None:
    steps_path, session, cwd, config = sys.argv[1:5]
    steps = [json.loads(line) for line in open(steps_path) if line.strip()]
    slug = "".join(c if c.isascii() and (c.isalnum() or c == "-") else "-" for c in cwd)
    out = Path(config) / "projects" / slug / f"{session}.jsonl"
    out.parent.mkdir(parents=True, exist_ok=True)

    records: list[dict] = []
    gaps = sum(float(step["text"]) for step in steps if step["kind"] == "gap")
    clock = datetime.now(timezone.utc) - timedelta(seconds=12 * len(steps) + 30 + gaps)
    parent = None
    prompt_id = str(uuid.uuid4())

    def stamp(seconds: float) -> str:
        nonlocal clock
        clock += timedelta(seconds=seconds)
        return clock.strftime("%Y-%m-%dT%H:%M:%S.") + f"{clock.microsecond // 1000:03d}Z"

    def base(kind: str, seconds: float) -> dict:
        nonlocal parent
        me = str(uuid.uuid4())
        record = {
            "parentUuid": parent,
            "isSidechain": False,
            "type": kind,
            "uuid": me,
            "timestamp": stamp(seconds),
            "userType": "external",
            "entrypoint": "cli",
            "cwd": cwd,
            "sessionId": session,
            "version": VERSION,
            "gitBranch": "main",
        }
        parent = me
        return record

    def assistant(content: list, stop: str, seconds: float) -> dict:
        record = base("assistant", seconds)
        record["message"] = {
            "model": MODEL,
            "id": "msg_" + uuid.uuid4().hex[:24],
            "type": "message",
            "role": "assistant",
            "content": content,
            "stop_reason": stop,
            "stop_sequence": None,
            "usage": {
                "input_tokens": 4,
                "cache_creation_input_tokens": 1820,
                "cache_read_input_tokens": 48211,
                "output_tokens": 412,
                "service_tier": "standard",
            },
        }
        record["requestId"] = "req_" + uuid.uuid4().hex[:24]
        records.append(record)
        return record

    def tool(name: str, tool_input: dict, result: str, extra: dict | None = None, seconds: float = 6) -> None:
        tool_id = "toolu_" + uuid.uuid4().hex[:24]
        call = assistant([{"type": "tool_use", "id": tool_id, "name": name, "input": tool_input}], "tool_use", seconds)
        record = base("user", 2.5)
        record["promptId"] = prompt_id
        record["message"] = {
            "role": "user",
            "content": [{"tool_use_id": tool_id, "type": "tool_result", "content": result, "is_error": False}],
        }
        record["toolUseResult"] = extra or {"stdout": result, "stderr": "", "interrupted": False, "isImage": False}
        record["sourceToolAssistantUUID"] = call["uuid"]
        records.append(record)

    for step in steps:
        kind = step["kind"]
        if kind == "gap":
            stamp(float(step["text"]))
        elif kind == "prompt":
            prompt_id = str(uuid.uuid4())
            record = base("user", 0)
            record.update(
                {
                    "promptId": prompt_id,
                    "message": {"role": "user", "content": step["text"]},
                    "permissionMode": "default",
                    "origin": {"kind": "human"},
                    "promptSource": "typed",
                }
            )
            records.append(record)
        elif kind == "bash":
            tool("Bash", {"command": step["command"], "description": step["desc"]}, step["output"])
        elif kind == "text":
            assistant([{"type": "text", "text": step["text"]}], "tool_use", 9)
        elif kind == "final":
            assistant([{"type": "text", "text": step["text"]}], "end_turn", 8)
        elif kind == "agent":
            description, agent = step["text"].split("|", 1)
            brief = (
                f"You're building {description.split(':')[0]} in the charge-retry worktree, on its own branch. "
                "Read the card first: farcooler task show "
                f"{description.split(':')[0]}. Report to the orchestrator when it's ready for review."
            )
            tool_input = {
                "description": description,
                "subagent_type": "general-purpose",
                "model": "sonnet",
                "run_in_background": True,
                "prompt": brief,
            }
            result = (
                f"Async agent launched successfully.\nagentId: {agent} (internal ID - do not mention to user. "
                "Use SendMessage with to: '" + agent + "' to continue this agent.)\n"
                "The agent is working in the background. You will be notified automatically when it completes."
            )
            extra = {
                "isAsync": True,
                "status": "async_launched",
                "agentId": agent,
                "description": description,
                "resolvedModel": "claude-sonnet-5-5",
                "prompt": brief,
            }
            tool("Agent", tool_input, result, extra, seconds=7)
        elif kind == "send":
            agent, message = step["text"].split("|", 1)
            tool(
                "SendMessage",
                {"to": agent, "summary": "Code blocks join the dark-mode card", "message": message},
                f"Message sent to {agent}. It will be delivered when the agent finishes its current tool call.",
                {"success": True, "to": agent},
                seconds=6,
            )

    with open(out, "w") as f:
        for record in records:
            f.write(json.dumps(record) + "\n")
    print(f"stage-demo: wrote {len(records)} records to {out}", file=sys.stderr)


if __name__ == "__main__":
    main()
