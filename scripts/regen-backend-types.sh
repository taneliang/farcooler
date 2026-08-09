#!/usr/bin/env bash
# Refresh the vendor protocol artifacts the native backends are written
# against, and record the versions they came from.
#
# Run by a human, never by cargo. `cargo build` must not need the network and
# CI must not need npm to compile a Rust workspace, so the OUTPUT is committed
# and this script is how it gets there.
#
# Codex publishes a JSON Schema generated from the exact binary installed.
# Claude publishes no schema at all — only a TypeScript declaration that ships
# with the SDK — which is why its handshake types are hand-written against
# `vendor/claude-sdk.d.ts` rather than generated from it.
set -euo pipefail
cd "$(dirname "$0")/.."

command -v codex >/dev/null || { echo "codex is not installed"; exit 1; }
command -v npm   >/dev/null || { echo "npm is not installed"; exit 1; }
command -v node  >/dev/null || { echo "node is not installed"; exit 1; }

mkdir -p vendor
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

codex_version=$(codex --version | awk '{print $NF}')
codex app-server generate-json-schema --out "$tmp/codex" >/dev/null
cp "$tmp/codex/codex_app_server_protocol.v2.schemas.json" vendor/codex-app-server.schema.json

npm pack @anthropic-ai/claude-agent-sdk --pack-destination "$tmp" >/dev/null
tar xzf "$tmp"/anthropic-ai-claude-agent-sdk-*.tgz -C "$tmp"
cp "$tmp/package/sdk.d.ts" vendor/claude-sdk.d.ts
claude_sdk_version=$(node -p "require('$tmp/package/package.json').version")

cat > vendor/PINNED <<EOF
codex-cli $codex_version
@anthropic-ai/claude-agent-sdk $claude_sdk_version
EOF

echo "pinned:"
cat vendor/PINNED
