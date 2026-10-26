#!/usr/bin/env bash
# Builds the web client into dist/, which wrangler uploads as the Worker's
# assets. One script for both channels, run by .github/workflows/web.yml and by
# a train's gates: `./build.sh --channel canary|stable [--build <sha>]`.
# Then `node scripts/size-gate.mjs dist` before any deploy.
set -euo pipefail
cd "$(dirname "$0")"
exec node scripts/build.mjs "$@"
