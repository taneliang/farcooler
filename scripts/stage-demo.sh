#!/bin/bash
# Stage a representative project on a scratch runner, for the website's hero shot (ov-476).
#
#   scripts/stage-demo.sh seed            make the runner and seed it (wipes an earlier one)
#   scripts/stage-demo.sh capture <out>   capture the real Mac window into <out>
#   scripts/stage-demo.sh stop            stop the scratch daemon and its tmux server
#
# The scene: Harbor, a small invoicing app. The owner sends the orchestrator one
# message with three unrelated asks, and the orchestrator files them three ways:
#
#   1. "invoices need a PDF export"  -> a new card, queued behind the card that
#                                       is rebuilding the invoice template;
#   2. "failed charges should retry" -> a new card, dispatched at once into a
#                                       lane that is building;
#   3. "blog code blocks ignore dark mode" -> added to the dark-mode card that
#                                       is already running in its lane.
#
# Everything on the board goes through the real CLI. The orchestrator's pane
# runs a stand-in for claude (a compiled program that only holds the pane
# open: no model, no network, no ~/.claude.json), and its conversation is a
# Claude Code transcript written in the shape claude writes, which the runner
# reads as it reads any other.
#
# The runner lives under /tmp/fc-t/h (short, since the daemon's socket is under
# it) with a HOME and CLAUDE_CONFIG_DIR of its own, so nothing here reads or
# writes your own home, your claude config, or your running Far Cooler.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
H=/tmp/fc-t/h
real_home="$HOME"
bin="${FARCOOLER_BIN:-$root/target/debug/farcooler}"

# The scratch runner's environment. SHELL is bash and HOME is empty, so a
# pane's shell reads no profile that could put a real claude first on PATH.
export FARCOOLER_HOME=$H/fc FARCOOLER_CONFIG=$H/fc/config.toml HOME=$H/home CLAUDE_CONFIG_DIR=$H/home/.claude SHELL=/bin/bash
export PATH=$H/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin
unset FARCOOLER_WORKSPACE FARCOOLER_TASK FARCOOLER_ACTOR TMUX
fc() { "$bin" "$@"; }
key() { python3 -c 'import json,sys; print(json.load(sys.stdin)["key"])'; }

stop() {
    local socket=""
    if [ -x "$bin" ] && [ -d "$FARCOOLER_HOME" ]; then
        socket=$(fc status 2>/dev/null | sed -n -E 's/.*tmux -L (farcooler-[0-9a-f]+).*/\1/p' | head -1)
        fc daemon stop >/dev/null 2>&1 || true
    fi
    # Whoever still holds this home's lock is this home's daemon, and no one else's.
    for pid in $(lsof -t "$FARCOOLER_HOME/farcoolerd.lock" 2>/dev/null); do kill "$pid" 2>/dev/null || true; done
    [ -n "$socket" ] && tmux -L "$socket" kill-server 2>/dev/null || true
    # The stand-in API, by the PID the seed wrote.
    [ -f "$H/api.pid" ] && kill "$(cat "$H/api.pid")" 2>/dev/null || true
}

case "${1:-}" in
    stop) stop; exit 0 ;;
    seed|capture) ;;
    *) sed -n '2,26p' "$0"; exit 1 ;;
esac

if [ "$1" = "capture" ]; then
    out="${2:?usage: scripts/stage-demo.sh capture <out-dir>}"
    mkdir -p "$out"; out="$(cd "$out" && pwd)"
    seed="$FARCOOLER_HOME/seed.json"
    [ -f "$seed" ] || { echo "stage-demo: seed first" >&2; exit 1; }
    ws=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["workspace"])' "$seed")
    term=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["orchestrator"])' "$seed")
    sso=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["sso"])' "$seed")
    cd "$root/apps/macos"
    # The build and the test run as you, with your toolchains; only the
    # runner they talk to is the scratch one.
    export HOME="$real_home" PATH="$real_home/.cargo/bin:$PATH" CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-6}"
    ./build-vt.sh >/dev/null
    # The workspace with the plan beside its chat; the conversation view on,
    # for this runner and this pane, as Settings and the pane's switch leave it.
    env -u FARCOOLER_WORKSPACE \
        FARCOOLER_BIN="$bin" FARCOOLER_CAPTURE_OUT="$out" FARCOOLER_CAPTURE_STAGE=hero \
        FARCOOLER_CAPTURE_THEN="${FARCOOLER_CAPTURE_THEN-fold:collapse
|$ws|task:$sso
|$ws|}" \
        FARCOOLER_CAPTURE_ONLY="${FARCOOLER_CAPTURE_ONLY:-plan}" \
        FARCOOLER_CAPTURE_PLACES="${FARCOOLER_CAPTURE_PLACES:-plan=|$ws|}" \
        FARCOOLER_CAPTURE_DEFAULTS="nativeAgent.enabled=true
nativeAgent.view.$term=false
${FARCOOLER_CAPTURE_DEFAULTS:-}" \
        FARCOOLER_CAPTURE_SCALE=2 FARCOOLER_CAPTURE_WIDTH="${FARCOOLER_CAPTURE_WIDTH:-1600}" \
        FARCOOLER_CAPTURE_HEIGHT="${FARCOOLER_CAPTURE_HEIGHT:-1000}" \
        swift test -j "${SWIFT_JOBS:-3}" --filter RealWindowCaptures
    exit 0
fi

# ---------------------------------------------------------------------------
# seed
# ---------------------------------------------------------------------------
[ -x "$bin" ] || (cd "$root" && CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" "$real_home/.cargo/bin/cargo" build -q -p farcooler-cli -p farcooler-daemon)
stop
rm -rf "$H"
mkdir -p "$H/bin" "$H/src" "$FARCOOLER_HOME" "$CLAUDE_CONFIG_DIR"

# The orchestrator's pane runs the real claude, on haiku, in this scratch home.
# Far Cooler starts it as `claude --session-id <id>`; the wrapper waits for the
# seeded transcript of that session, then resumes it, so the pane shows the
# conversation in claude's own terminal UI. claude talks only to a stand-in API
# on 127.0.0.1 (no network, no real key), and nothing bypasses its permissions.
real_claude=$(PATH="$real_home/.local/bin:$PATH" command -v claude)
port=47613
cat >"$H/bin/api.py" <<'EOF'
# A stand-in for the Messages API: any POST gets a short, valid reply.
import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer

REPLY = {"id": "msg_standin", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
         "content": [{"type": "text", "text": "On it."}], "stop_reason": "end_turn", "stop_sequence": None,
         "usage": {"input_tokens": 1, "output_tokens": 1}}

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("content-length") or 0)) or b"{}")
        if "count_tokens" in self.path:
            return self.send(200, {"input_tokens": 1})
        if not body.get("stream"):
            return self.send(200, REPLY)
        events = [("message_start", {"type": "message_start", "message": {**REPLY, "content": []}}),
                  ("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
                  ("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "On it."}}),
                  ("content_block_stop", {"type": "content_block_stop", "index": 0}),
                  ("message_delta", {"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 1}}),
                  ("message_stop", {"type": "message_stop"})]
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.end_headers()
        for name, data in events:
            self.wfile.write(f"event: {name}\ndata: {json.dumps(data)}\n\n".encode())

    def do_GET(self):
        self.send(200, {})

    def send(self, code, obj):
        data = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass

HTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
EOF
cat >"$H/bin/claude" <<EOF
#!/bin/bash
# Far Cooler's launch: claude --session-id <id> [...]. Wait for the seeded
# transcript of <id>, then resume it with the real claude on haiku.
sid=""
while [ \$# -gt 0 ]; do
    case "\$1" in --session-id) sid=\$2; shift 2 ;; *) shift ;; esac
done
until [ -n "\$sid" ] && ls "$CLAUDE_CONFIG_DIR"/projects/*/"\$sid.jsonl" >/dev/null 2>&1; do sleep 0.5; done
export ANTHROPIC_BASE_URL=http://127.0.0.1:$port ANTHROPIC_API_KEY=sk-ant-standin-000000000000000000000000
export CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 DISABLE_AUTOUPDATER=1
# claude asks for tmux focus events on start; this scratch server turns them on.
tmux set -g focus-events on 2>/dev/null
exec "$real_claude" --resume "\$sid" --model haiku
EOF
chmod +x "$H/bin/claude"
nohup python3 "$H/bin/api.py" "$port" >/dev/null 2>&1 &
echo $! >"$H/api.pid"

# The repository: a small Next.js app with a short history.
repo="$H/src/harbor"
mkdir -p "$repo" && cd "$repo"
g() { git -c user.name="Maya Okafor" -c user.email=maya@harbor.dev -c commit.gpgsign=false "$@"; }
g init -q -b main
mkdir -p src/invoices src/payments src/blog/styles
printf '{\n  "name": "harbor",\n  "private": true,\n  "scripts": { "dev": "next dev", "test": "vitest run" }\n}\n' >package.json
printf '# Harbor\n\nInvoicing and payments for small studios.\n' >README.md
g add -A && g commit -q -m "Start Harbor"
printf 'export type Line = { description: string; amount: number };\nexport type Invoice = { id: string; lines: Line[] };\n' >src/invoices/types.ts
printf 'import type { Invoice } from "./types";\n\nexport function renderInvoice(invoice: Invoice): string {\n  return invoice.lines.map((l) => `${l.description}\\t${l.amount}`).join("\\n");\n}\n' >src/invoices/render.ts
g add -A && g commit -q -m "Render invoices"
printf 'export async function chargeInvoice(stripe, customer, amount) {\n  return stripe.paymentIntents.create({ customer, amount, currency: "usd", confirm: true });\n}\n' >src/payments/charge.ts
g add -A && g commit -q -m "Charge invoices through Stripe"
printf '.prose pre { background: #f6f8fa; color: #24292f; }\n' >src/blog/styles/code.css
g add -A && g commit -q -m "Blog styles"
cd "$root"

fc --json daemon ensure >/dev/null 2>&1 || fc --json daemon ensure >/dev/null
fc settings set-projector on >/dev/null
fc root add "$H/src" >/dev/null
fc repo register "$repo" >/dev/null
ws=$(fc --json workspace list | python3 -c 'import json,sys; print(json.load(sys.stdin)["workspaces"][0]["id"])')
fc workspace rename Main Harbor --repo harbor >/dev/null
W=(--repo harbor --workspace Harbor)

card() { # title intent status -> key
    local k
    k=$(fc --json task create "${W[@]}" --actor manager --title "$1" --intent "$2" | key)
    [ "$3" = backlog ] || fc task set "$k" --repo harbor --status "$3" >/dev/null
    echo "$k"
}

# Themes, modeled on Far Cooler's own board: why each group of cards exists.
# Four, so the sidebar and the plan read calm.
theme() { fc plan theme create "${W[@]}" --actor manager "$1" --outcome "$2" >/dev/null; }
theme "Invoicing v2" "Invoices clients can read, print and pay, in any currency."
theme "Teams and accounts" "A studio of any size runs Harbor with its whole team, under its own sign-in."
theme "Payments you can trust" "Every charge that can succeed does, and none can be forged."
theme "Marketing" "harbor.dev explains Harbor in a minute, in light and dark."
in_theme() { fc plan theme cards "${W[@]}" --actor manager "$1" --add "$2" >/dev/null; }
tcard() { # theme title intent status -> key
    local k; k=$(card "$2" "$3" "$4"); in_theme "$1" "$k"; echo "$k"
}

# What came before: a little landed work, enough for each theme to show progress.
k_num=$(tcard "Invoicing v2" "Invoices: number them per studio" "" done)
tcard "Teams and accounts" "Auth: sign in with Google" "" done >/dev/null
k_pay=$(tcard "Payments you can trust" "Payments: take cards through Stripe" "" done)
tcard "Marketing" "Blog: first post and RSS" "" done >/dev/null

# Now: what's running, in review, landing, and waiting.
k_csv=$(tcard "Teams and accounts" "Onboarding: import clients from a CSV" "Studios moving from spreadsheets bring their clients in one go." in_review)
k_hooks=$(tcard "Payments you can trust" "Payments: verify Stripe webhook signatures" "A forged webhook can't mark an invoice paid." in_progress)
k_dark=$(tcard "Marketing" "Blog: dark mode on every page" "The blog follows the reader's light or dark setting, everywhere." in_progress)
fc task set "$k_dark" --repo harbor --accept "Every blog page follows the system appearance" --accept "No white flash on load in dark mode" >/dev/null
k_invite=$(tcard "Teams and accounts" "Settings: invite team members" "A studio's owner can add a bookkeeper." todo)
k_refund=$(tcard "Payments you can trust" "Payments: refund from the invoice page" "Refunds without opening Stripe." backlog)
k_tpl=$(tcard "Invoicing v2" "Invoices: rebuild the template on the new layout" "Invoices use the new layout: logo, terms, and a totals block that reads at a glance." in_progress)

# Lanes, each in its own worktree on its own branch.
lane() { # name title card state agent model reason
    local name=$1 title=$2 card=$3 state=$4 agent=$5 model=$6 reason=$7 wt
    wt=$(fc --json worktree create harbor "$name" --branch "$name" --fork-only | python3 -c 'import json,sys; print(json.load(sys.stdin)["worktree"])')
    if [ -n "$agent" ]; then
        fc plan lane start "${W[@]}" --actor manager "$name" --title "$title" --card "$card" --reason "$reason" \
            --path "$wt" --branch "$name" --agent "$agent" --model "$model" >/dev/null
    else
        fc plan lane start "${W[@]}" --actor manager "$name" --title "$title" --card "$card" --reason "$reason" \
            --path "$wt" --branch "$name" >/dev/null
    fi
    [ "$state" = building ] || [ "$state" = queued ] || fc plan lane set "${W[@]}" --actor manager "$name" --state "$state" >/dev/null
}
lane invoice-template "Invoices use the new layout" "$k_tpl" building a7c41e9d02b85f316 sonnet "Unblocks every invoice change after it"
lane blog-dark-mode "The blog follows dark mode" "$k_dark" building a2e98b1d4c07f5a63 sonnet "Readers on dark mode see a white page"
lane csv-import "Clients import from a CSV" "$k_csv" building a5b07d3e91c2f8e44 sonnet "Asked for by three studios this week"
fc plan lane set "${W[@]}" --actor manager csv-import --agent a91f3c6e70d2b5a18 --role review --model opus >/dev/null
lane stripe-webhooks "Stripe webhooks are verified" "$k_hooks" building a6d2f8b0e13c94a75 sonnet "Security: a forged event could mark an invoice paid"
fc plan lane set "${W[@]}" --actor manager stripe-webhooks --agent a0c8e5f2d7b31964e --role review --model opus >/dev/null
fc plan lane set "${W[@]}" --actor manager stripe-webhooks --agent a0c8e5f2d7b31964e --ended >/dev/null
fc plan lane set "${W[@]}" --actor manager stripe-webhooks --state landing --reason "Review passed; gates green" >/dev/null
fc plan train start "${W[@]}" --actor manager integ-7 --lane stripe-webhooks --base origin/main \
    --title "Webhook signatures" >/dev/null

# The orchestrator, on the stand-in.
fc workspace start-orchestrator Harbor --repo harbor >/dev/null
sleep 2
orch=$(fc --json workspace list | python3 -c 'import json,sys; print(json.load(sys.stdin)["workspaces"][0]["orchestrator"])')
session=$(sqlite3 "$FARCOOLER_HOME/farcooler.db" "select agent_session_id from terminals where agent_session_id is not null" | head -1)
[ -n "$orch" ] && [ -n "$session" ] || { echo "stage-demo: no orchestrator session" >&2; exit 1; }

# The orchestrator's turn, run for real: each step's command goes through the
# CLI as the orchestrator's pane would run it, and its output is what the
# transcript's tool result says.
steps="$H/steps.jsonl"; : >"$steps"
export FARCOOLER_WORKSPACE="$ws" FARCOOLER_ACTOR=manager
step() { # kind description shown-command [real-command]
    local kind=$1 desc=$2 shown=$3 real=${4:-$3} output
    output=$(eval "${real//farcooler /fc }" 2>&1)
    python3 -c 'import json,sys; print(json.dumps(dict(zip(["kind","desc","command","output"], sys.argv[1:]))))' \
        "$kind" "$desc" "$shown" "$output" >>"$steps"
}
note() { python3 -c 'import json,sys; print(json.dumps({"kind": sys.argv[1], "text": sys.argv[2]}))' "$1" "$2" >>"$steps"; }

# The keys the orchestrator's own creates will get, in order: the discounts in
# an earlier round, then the brain dump's three new cards.
n=${k_tpl#ha-}
k_disc="ha-$((n + 1))" k_pdf="ha-$((n + 2))" k_retry="ha-$((n + 3))" k_sso="ha-$((n + 4))"
retry_agent=a3d90b7e5f1c28a46
dark_agent=a2e98b1d4c07f5a63
new_lane() { # name -> the worktree a lane's agent works in, made as the orchestrator's launch makes it
    fc --json worktree create harbor "$1" --branch "$1" --fork-only | python3 -c 'import json,sys; print(json.load(sys.stdin)["worktree"])'
}

# Earlier rounds, so the conversation has history above the brain dump. Each
# step runs the command it shows; the discounts card is filed here, and lands
# off-screen (its lane's work) before the round that asks about it. Their times
# are spread over the morning.
note prompt "stripe cards are live in prod, right? checking before I tell Northwind"
step bash "Check $k_pay" "farcooler task show $k_pay"
note final "Yes: **$k_pay** has been live since last week. Cards, 3-D Secure and receipts all work, and CI has been green on main since."
note gap 5400
note prompt "line items should take discounts, a flat amount or a percent"
step bash "File the discounts" \
    "farcooler task create --title \"Invoices: line items take discounts\" --intent \"A line takes a flat or percent discount, and the totals show it.\" && farcooler plan theme cards \"Invoicing v2\" --add $k_disc"
[ "$(fc --json task show "$k_disc" --repo harbor | python3 -c 'import json,sys; print(json.load(sys.stdin)["task"]["key"])')" = "$k_disc" ] || { echo "stage-demo: the discounts card isn't $k_disc" >&2; exit 1; }
note final "Filed as **$k_disc** under Invoicing v2: a flat or percent discount on each line, shown in the totals. Building now; I'll tell you when it lands."
fc task set "$k_disc" --repo harbor --status done >/dev/null
note gap 7200
note prompt "did the discounts land?"
step bash "Check $k_disc" "farcooler task show $k_disc"
note final "Landed, CI green: **$k_disc** is on main. Percent and flat discounts work per line, and the totals block shows both."
note gap 2400
note prompt "how's the CSV import looking"
step bash "Check $k_csv" "farcooler task show $k_csv"
note final "Built, and waiting on you: **$k_csv** is in Needs You, ready for review."
note gap 5400

note prompt "ok brain dump before I forget. invoices need a PDF export, clients keep asking. failed charges should retry instead of giving up after one decline. oh and code blocks on the blog ignore dark mode, still bright white. Northwind won't sign without SSO + SCIM so we need real enterprise team management at some point"
step bash "Read the plan" "farcooler plan"
note text "Four asks. Checking each against what's already moving."
step bash "File the PDF export behind the template" \
    "farcooler task create --title \"Invoices: export as PDF\" --intent \"Clients download any invoice as a PDF that matches the new template.\" && farcooler task block $k_pdf --on $k_tpl --reason \"It prints the template $k_tpl is rebuilding\" && farcooler plan theme cards \"Invoicing v2\" --add $k_pdf && farcooler plan lane start invoice-pdf --title \"Invoices export as PDF\" --card $k_pdf --reason \"Starts when $k_tpl lands\""
step bash "File the charge retries" \
    "farcooler task create --title \"Payments: retry failed charges\" --intent \"A declined charge is retried 1, 3 and 7 days later before the invoice is marked failed.\" && farcooler plan theme cards \"Payments you can trust\" --add $k_retry"
wt_retry=$(new_lane charge-retry)
# No agent record: a resumed claude reports a background agent from the
# seeded session as unfinished. The lane start below is what dispatches it.
step bash "Put it in a lane" \
    "farcooler plan lane start charge-retry --title \"Failed charges retry\" --card $k_retry --branch charge-retry --path $wt_retry --agent $retry_agent --model sonnet --reason \"Declined cards cost studios a week of chasing\""
fc task set "$k_retry" --status in_progress >/dev/null
step bash "Add the code blocks to $k_dark" \
    "farcooler task set $k_dark --accept \"Every blog page follows the system appearance\" --accept \"No white flash on load in dark mode\" --accept \"Code blocks in posts follow dark mode too\""
note send "$dark_agent|One more for $k_dark: code blocks in posts stay bright white in dark mode. src/blog/styles/code.css hard-codes their colors; move them onto the theme variables. It's on the card as a third acceptance line."
step bash "File SSO and SCIM, and ask" \
    "farcooler task create --title \"Teams: enterprise SSO and SCIM\" --intent \"Northwind's IT signs people in and out of Harbor from their identity provider.\" && farcooler plan theme cards \"Teams and accounts\" --add $k_sso && farcooler task ask $k_sso --body \"SAML only, or OIDC as well? Northwind needs SAML; OIDC roughly doubles the provider work.\""
note final "Four asks, in order:

- **PDF export** ($k_pdf): queued behind $k_tpl, which is rebuilding the template it prints.
- **Charge retries** ($k_retry): building now in \`charge-retry\`.
- **Blog code blocks**: added to $k_dark, the dark-mode card already building. Same stylesheets; I told its agent.
- **SSO + SCIM** ($k_sso): under Teams and accounts, with one question for you in Needs You: SAML only, or OIDC too?"
unset FARCOOLER_WORKSPACE FARCOOLER_ACTOR

# The transcript, where claude would write it for the orchestrator's home.
home_dir=$(fc --json workspace list | python3 -c 'import json,sys; print(json.load(sys.stdin)["workspaces"][0]["home"])')
# claude's own settings, in the scratch config dir: onboarding done, dark,
# the stand-in key approved and the orchestrator's home trusted, so it opens
# straight onto the conversation with no dialog.
python3 - "$CLAUDE_CONFIG_DIR/.claude.json" "$(cd "$home_dir" && pwd -P)" <<'EOF2'
import json, sys
path, home = sys.argv[1:]
json.dump({"hasCompletedOnboarding": True, "theme": "dark", "numStartups": 5,
           "customApiKeyResponses": {"approved": ["000000000000000000000000"[-20:]], "rejected": []},
           "projects": {home: {"hasTrustDialogAccepted": True, "hasCompletedProjectOnboarding": True}}},
          open(path, "w"), indent=2)
EOF2
python3 "$root/scripts/stage-demo-transcript.py" "$steps" "$session" "$(cd "$home_dir" && pwd -P)" "$CLAUDE_CONFIG_DIR"

# The SSO card's id (a selection names a card by id, not by key), for the capture.
sso_id=$(fc --json task show "$k_sso" --repo harbor | python3 -c 'import json,sys; print(json.load(sys.stdin)["notes"][0]["task_id"])')
python3 - "$FARCOOLER_HOME/seed.json" "$ws" "$orch" "$sso_id" <<'EOF'
import json, sys
path, ws, orch, sso = sys.argv[1:]
json.dump({"workspace": ws, "orchestrator": orch, "sso": sso}, open(path, "w"), indent=2)
EOF
echo "stage-demo: seeded; next, scripts/stage-demo.sh capture <out>" >&2
