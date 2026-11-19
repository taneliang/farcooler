#!/bin/sh
exec python3 "$(dirname "$0")/daemon-layers.py" "$@"
