#!/usr/bin/env bash
# Downloads one GitHub release asset, authenticated and with retries (ov-1030).
#
#   scripts/gh-download.sh <owner/repo> <tag> <asset> <out-file> <sha256>
#
# `gh release download` sends GH_TOKEN (set it to the job's GITHUB_TOKEN), where
# a bare curl of the release URL is anonymous, and anonymous release downloads
# were the first thing GitHub throttled in its Nov 9 incident (504s). A failed
# fetch, or one whose checksum is wrong, waits 5, 15, 45, then 90 seconds and
# tries again; only the last failure fails the step. Each attempt is cut off
# after GH_DOWNLOAD_TIMEOUT seconds (default 300), so a stalled one retries
# rather than hanging the job. The sha256 is required: an unpinned download
# fails closed, because what is fetched here can run with signing keys.
set -euo pipefail

if [ "$#" -ne 5 ] || [ -z "$5" ]; then
  echo "usage: $0 <owner/repo> <tag> <asset> <out-file> <sha256>" >&2
  exit 2
fi
repo=$1 tag=$2 asset=$3 out=$4 sha=$5
waits=${GH_DOWNLOAD_WAITS:-"0 5 15 45 90"}
limit=${GH_DOWNLOAD_TIMEOUT:-300}

# Runs gh in the background and kills it at the limit; macOS has no timeout(1).
fetch() {
  gh release download "$tag" --repo "$repo" --pattern "$asset" --output "$out" &
  local pid=$! killer status=0
  ( sleep "$limit"; kill "$pid" 2>/dev/null ) &
  killer=$!
  wait "$pid" || status=$?
  kill "$killer" 2>/dev/null || true
  wait "$killer" 2>/dev/null || true
  return "$status"
}

for wait in $waits; do
  sleep "$wait"
  rm -f "$out"
  if fetch && echo "$sha  $out" | shasum -a 256 -c -; then
    exit 0
  fi
  echo "download of $asset from $repo $tag failed; retrying" >&2
done
echo "could not download $asset from $repo $tag" >&2
exit 1
