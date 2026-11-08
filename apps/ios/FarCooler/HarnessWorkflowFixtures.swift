#if DEBUG
import Foundation

/// What the harness shows of a workflow change and of an agent's terminal (ov-976, ov-978), for captures.
///
/// Both are real: the question is the daemon's own producer's output (`crates/client/src/testdata/
/// workflow-change-question.txt`, which `HarnessWorkflowFixturesTests` in AgentKit holds this copy to), and the
/// bytes are tmux's own capture of a 120-column pane running long, colored output.
enum HarnessWorkflowFixtures {
    static let changeQuestion = """
Publish this workflow? 4 checks must pass, each ticket opens a pull request, you approve each ticket.

The orchestrator’s reason: `Gate every push on the copy lint and the AgentKit tests.`

This is the first workflow for this workspace.

What runs as you:
- The agent: `/usr/bin/claude`.
- `make check`
- `scripts/copy-lint.py --all`
- `cargo test -p farcooler-daemon --lib -- --test-threads=1`
- `swift test --package-path apps/shared/AgentKit`

Checks that must pass:
- `make check` (with `HOME=/home/tester`, `PATH=/usr/bin:/bin:/home/tester/.cargo/bin`)
- `scripts/copy-lint.py --all` (with `HOME=/home/tester`, `PATH=/usr/bin:/bin:/home/tester/.cargo/bin`)
- `cargo test -p farcooler-daemon --lib -- --test-threads=1` (with `HOME=/home/tester`, `PATH=/usr/bin:/bin:/home/tester/.cargo/bin`)
- `swift test --package-path apps/shared/AgentKit` (with `HOME=/home/tester`, `PATH=/usr/bin:/bin:/home/tester/.cargo/bin`)

How code lands:
- Each ticket opens a pull request against `main`. A person merges it.
- It waits for CI to pass on what landed.

Who approves:
- You approve each ticket before it lands.

In full, exactly as it runs:
- The agent runs `/usr/bin/claude`, with no environment variables, for up to 1 s.
- Check 1 runs `/bin/sh` `-c` `make check`, with environment `HOME=/home/tester`, `PATH=/usr/bin:/bin:/home/tester/.cargo/bin`, for up to 3 h.
- Check 2 runs `/bin/sh` `-c` `scripts/copy-lint.py --all`, with environment `HOME=/home/tester`, `PATH=/usr/bin:/bin:/home/tester/.cargo/bin`, for up to 3 h.
- Check 3 runs `/bin/sh` `-c` `cargo test -p farcooler-daemon --lib -- --test-threads=1`, with environment `HOME=/home/tester`, `PATH=/usr/bin:/bin:/home/tester/.cargo/bin`, for up to 3 h.
- Check 4 runs `/bin/sh` `-c` `swift test --package-path apps/shared/AgentKit`, with environment `HOME=/home/tester`, `PATH=/usr/bin:/bin:/home/tester/.cargo/bin`, for up to 3 h.
- No reviewer command.
- CI is the `ci` workflow on `push` in `o/r` on GitHub, read by running `/bin/sh` `-c` `true`, with no environment variables, for up to 1 s.
- Pull requests open in `far-cooler/overnight` on GitHub, against `main`. Their branches are pushed to `ssh://git@example.com/far-cooler/overnight.git`. They're opened by running `/bin/sh` `-c` `gh pr create --fill`, with no environment variables, for up to 1 s.
- Code is read from `/r` from commit `aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa`, and its landing ref there is `refs/heads/main`.
- Code lands on `refs/heads/main` in `/srv/landing.git`, as job `landing-1`. Where that repository pushes is fixed by its config, digest `wf1:0123456789abcdef`.

To publish exactly this, answer `approve a1b2c3`. To leave things as they are, answer `decline`. Nothing changes until you answer.
"""

    private static let paneBase64 = """
G1A+ZmFyY29vbGVyLXNpemU7MTIwOzQwG1wbWzFtG1szMm3il48gc3RlcCAxMxtbMG0gIBtbMm1ydW5uaW5nG1swbSBjYXJnbyB0
ZXN0IC1wIGZhcmNvb2xlci1kYWVtb24gLS1saWIgd29ya2Zsb3dfcnVudGltZTo6Y29kaW5nX25hdGl2ZTo6dGVzdHM6OmNhc2Vf
MTMgLS0gLS1leGFjdCAtLW5vYw0KYXB0dXJlIC0tdGVzdC10aHJlYWRzPTENChtbMW0bWzMzbeKXjyBzdGVwIDE0G1swbSAgG1sy
bXJ1bm5pbmcbWzBtIGNhcmdvIHRlc3QgLXAgZmFyY29vbGVyLWRhZW1vbiAtLWxpYiB3b3JrZmxvd19ydW50aW1lOjpjb2Rpbmdf
bmF0aXZlOjp0ZXN0czo6Y2FzZV8xNCAtLSAtLWV4YWN0IC0tbm9jDQphcHR1cmUgLS10ZXN0LXRocmVhZHM9MQ0KG1sxbRtbMzRt
4pePIHN0ZXAgMTUbWzBtICAbWzJtcnVubmluZxtbMG0gY2FyZ28gdGVzdCAtcCBmYXJjb29sZXItZGFlbW9uIC0tbGliIHdvcmtm
bG93X3J1bnRpbWU6OmNvZGluZ19uYXRpdmU6OnRlc3RzOjpjYXNlXzE1IC0tIC0tZXhhY3QgLS1ub2MNCmFwdHVyZSAtLXRlc3Qt
dGhyZWFkcz0xDQobWzFtG1szNW3il48gc3RlcCAxNhtbMG0gIBtbMm1ydW5uaW5nG1swbSBjYXJnbyB0ZXN0IC1wIGZhcmNvb2xl
ci1kYWVtb24gLS1saWIgd29ya2Zsb3dfcnVudGltZTo6Y29kaW5nX25hdGl2ZTo6dGVzdHM6OmNhc2VfMTYgLS0gLS1leGFjdCAt
LW5vYw0KYXB0dXJlIC0tdGVzdC10aHJlYWRzPTENChtbMW0bWzM2beKXjyBzdGVwIDE3G1swbSAgG1sybXJ1bm5pbmcbWzBtIGNh
cmdvIHRlc3QgLXAgZmFyY29vbGVyLWRhZW1vbiAtLWxpYiB3b3JrZmxvd19ydW50aW1lOjpjb2RpbmdfbmF0aXZlOjp0ZXN0czo6
Y2FzZV8xNyAtLSAtLWV4YWN0IC0tbm9jDQphcHR1cmUgLS10ZXN0LXRocmVhZHM9MQ0KG1sxbRtbMzFt4pePIHN0ZXAgMTgbWzBt
ICAbWzJtcnVubmluZxtbMG0gY2FyZ28gdGVzdCAtcCBmYXJjb29sZXItZGFlbW9uIC0tbGliIHdvcmtmbG93X3J1bnRpbWU6OmNv
ZGluZ19uYXRpdmU6OnRlc3RzOjpjYXNlXzE4IC0tIC0tZXhhY3QgLS1ub2MNCmFwdHVyZSAtLXRlc3QtdGhyZWFkcz0xDQobWzFt
G1szMm3il48gc3RlcCAxORtbMG0gIBtbMm1ydW5uaW5nG1swbSBjYXJnbyB0ZXN0IC1wIGZhcmNvb2xlci1kYWVtb24gLS1saWIg
d29ya2Zsb3dfcnVudGltZTo6Y29kaW5nX25hdGl2ZTo6dGVzdHM6OmNhc2VfMTkgLS0gLS1leGFjdCAtLW5vYw0KYXB0dXJlIC0t
dGVzdC10aHJlYWRzPTENChtbMW0bWzMzbeKXjyBzdGVwIDIwG1swbSAgG1sybXJ1bm5pbmcbWzBtIGNhcmdvIHRlc3QgLXAgZmFy
Y29vbGVyLWRhZW1vbiAtLWxpYiB3b3JrZmxvd19ydW50aW1lOjpjb2RpbmdfbmF0aXZlOjp0ZXN0czo6Y2FzZV8yMCAtLSAtLWV4
YWN0IC0tbm9jDQphcHR1cmUgLS10ZXN0LXRocmVhZHM9MQ0KG1sxbRtbMzRt4pePIHN0ZXAgMjEbWzBtICAbWzJtcnVubmluZxtb
MG0gY2FyZ28gdGVzdCAtcCBmYXJjb29sZXItZGFlbW9uIC0tbGliIHdvcmtmbG93X3J1bnRpbWU6OmNvZGluZ19uYXRpdmU6OnRl
c3RzOjpjYXNlXzIxIC0tIC0tZXhhY3QgLS1ub2MNCmFwdHVyZSAtLXRlc3QtdGhyZWFkcz0xDQobWzFtG1szNW3il48gc3RlcCAy
MhtbMG0gIBtbMm1ydW5uaW5nG1swbSBjYXJnbyB0ZXN0IC1wIGZhcmNvb2xlci1kYWVtb24gLS1saWIgd29ya2Zsb3dfcnVudGlt
ZTo6Y29kaW5nX25hdGl2ZTo6dGVzdHM6OmNhc2VfMjIgLS0gLS1leGFjdCAtLW5vYw0KYXB0dXJlIC0tdGVzdC10aHJlYWRzPTEN
ChtbMW0bWzM2beKXjyBzdGVwIDIzG1swbSAgG1sybXJ1bm5pbmcbWzBtIGNhcmdvIHRlc3QgLXAgZmFyY29vbGVyLWRhZW1vbiAt
LWxpYiB3b3JrZmxvd19ydW50aW1lOjpjb2RpbmdfbmF0aXZlOjp0ZXN0czo6Y2FzZV8yMyAtLSAtLWV4YWN0IC0tbm9jDQphcHR1
cmUgLS10ZXN0LXRocmVhZHM9MQ0KG1sxbRtbMzFt4pePIHN0ZXAgMjQbWzBtICAbWzJtcnVubmluZxtbMG0gY2FyZ28gdGVzdCAt
cCBmYXJjb29sZXItZGFlbW9uIC0tbGliIHdvcmtmbG93X3J1bnRpbWU6OmNvZGluZ19uYXRpdmU6OnRlc3RzOjpjYXNlXzI0IC0t
IC0tZXhhY3QgLS1ub2MNCmFwdHVyZSAtLXRlc3QtdGhyZWFkcz0xDQobWzFtG1szMm3il48gc3RlcCAyNRtbMG0gIBtbMm1ydW5u
aW5nG1swbSBjYXJnbyB0ZXN0IC1wIGZhcmNvb2xlci1kYWVtb24gLS1saWIgd29ya2Zsb3dfcnVudGltZTo6Y29kaW5nX25hdGl2
ZTo6dGVzdHM6OmNhc2VfMjUgLS0gLS1leGFjdCAtLW5vYw0KYXB0dXJlIC0tdGVzdC10aHJlYWRzPTENChtbMW0bWzMzbeKXjyBz
dGVwIDI2G1swbSAgG1sybXJ1bm5pbmcbWzBtIGNhcmdvIHRlc3QgLXAgZmFyY29vbGVyLWRhZW1vbiAtLWxpYiB3b3JrZmxvd19y
dW50aW1lOjpjb2RpbmdfbmF0aXZlOjp0ZXN0czo6Y2FzZV8yNiAtLSAtLWV4YWN0IC0tbm9jDQphcHR1cmUgLS10ZXN0LXRocmVh
ZHM9MQ0KG1sxbRtbMzRt4pePIHN0ZXAgMjcbWzBtICAbWzJtcnVubmluZxtbMG0gY2FyZ28gdGVzdCAtcCBmYXJjb29sZXItZGFl
bW9uIC0tbGliIHdvcmtmbG93X3J1bnRpbWU6OmNvZGluZ19uYXRpdmU6OnRlc3RzOjpjYXNlXzI3IC0tIC0tZXhhY3QgLS1ub2MN
CmFwdHVyZSAtLXRlc3QtdGhyZWFkcz0xDQobWzFtG1szNW3il48gc3RlcCAyOBtbMG0gIBtbMm1ydW5uaW5nG1swbSBjYXJnbyB0
ZXN0IC1wIGZhcmNvb2xlci1kYWVtb24gLS1saWIgd29ya2Zsb3dfcnVudGltZTo6Y29kaW5nX25hdGl2ZTo6dGVzdHM6OmNhc2Vf
MjggLS0gLS1leGFjdCAtLW5vYw0KYXB0dXJlIC0tdGVzdC10aHJlYWRzPTENChtbMW0bWzM2beKXjyBzdGVwIDI5G1swbSAgG1sy
bXJ1bm5pbmcbWzBtIGNhcmdvIHRlc3QgLXAgZmFyY29vbGVyLWRhZW1vbiAtLWxpYiB3b3JrZmxvd19ydW50aW1lOjpjb2Rpbmdf
bmF0aXZlOjp0ZXN0czo6Y2FzZV8yOSAtLSAtLWV4YWN0IC0tbm9jDQphcHR1cmUgLS10ZXN0LXRocmVhZHM9MQ0KG1sxbRtbMzFt
4pePIHN0ZXAgMzAbWzBtICAbWzJtcnVubmluZxtbMG0gY2FyZ28gdGVzdCAtcCBmYXJjb29sZXItZGFlbW9uIC0tbGliIHdvcmtm
bG93X3J1bnRpbWU6OmNvZGluZ19uYXRpdmU6OnRlc3RzOjpjYXNlXzMwIC0tIC0tZXhhY3QgLS1ub2MNCmFwdHVyZSAtLXRlc3Qt
dGhyZWFkcz0xDQobWzMybSAgICBGaW5pc2hlZBtbMzltIGB0ZXN0YCBwcm9maWxlIFt1bm9wdGltaXplZCArIGRlYnVnaW5mb10g
dGFyZ2V0KHMpIGluIDQxLjIwcw0KG1sxbT4gQXBwbHlpbmcgdGhlIGNoYW5nZSB0byBhcHBzL3NoYXJlZC9BZ2VudEtpdC9Tb3Vy
Y2VzL0FnZW50S2l0L1dvcmtmbG93UnVuc1ZpZXcuc3dpZnQgYW5kIGFwcHMvYW5kcm9pZC9hcHAvc3JjL21haW4vamF2YQ0KL2Nv
bS9mYXJjb29sZXIvdWkvV29ya2Zsb3dSdW5zVmlldy5rdCwgdGhlbiBydW5uaW5nIGV2ZXJ5IGNoZWNrG1swbQ0KDQo=
"""

    static var paneBytes: [UInt8] {
        [UInt8](Data(base64Encoded: paneBase64.replacingOccurrences(of: "\n", with: "")) ?? Data())
    }
}

/// A watch that replays a recorded pane, then ends the way `-phone-agent-watch` says: `running` (no end),
/// `finished`, `lost` or `not-running`. It can start and stop and nothing else, like the real one.
struct HarnessReplayWatch: AgentWatchTransport {
    let state: String
    func startWatch(workspace: String, item: String, onChunk: @escaping @Sendable ([UInt8]) -> Void,
                    onEnd: @escaping @Sendable (String?) -> Void) async -> String? {
        if state != "not-running" {
            let bytes = HarnessWorkflowFixtures.paneBytes
            for start in stride(from: 0, to: bytes.count, by: 700) { onChunk(Array(bytes[start..<min(start + 700, bytes.count)])) }
        }
        switch state {
        case "finished", "not-running": onEnd(nil)
        case "lost": onEnd("lost")
        default: break
        }
        return "harness"
    }
    func stopWatch(_ name: String) async {}
}
#endif
