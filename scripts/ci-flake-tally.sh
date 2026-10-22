#!/bin/bash
# Which tests fail on main's CI, worst first, with infrastructure failures apart
# (ov-697). The parsing is Python (ci-flake-tally.py, which documents the
# flags); this is the entry point.
#
#   ./scripts/ci-flake-tally.sh [--runs 30]
exec python3 "$(dirname "$0")/ci-flake-tally.py" "$@"
