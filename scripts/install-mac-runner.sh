#!/bin/bash
# Bootstrap from a trusted checkout. This file is also carried in each signed runner archive.
set -euo pipefail
umask 077

if [[ "$(/usr/bin/uname -s)" != Darwin || "$(/usr/bin/id -u)" == 0 ]]; then
    echo "Run this installer on the Mac as the account that will run Far Cooler." >&2
    exit 1
fi

# Avoid ~/.curlrc. The signed index
# binds the channel, architecture, build number and archive digest together.
fetch() {
    /usr/bin/curl -q --fail --silent --show-error --proto '=https' \
        --connect-timeout 15 --max-time 600 --output "$2" "$1"
}
feed=https://updates.farcooler.com/canary/runner
arch="$(/usr/bin/uname -m)"
if [[ "$(/usr/sbin/sysctl -n hw.optional.arm64 2>/dev/null || true)" == 1 ]]; then
    arch=arm64
fi
case "$arch" in
    arm64|x86_64) ;;
    *) echo "Unsupported Mac architecture: $arch" >&2; exit 1 ;;
esac

# The launchd job has this same path even when started from the login window.
export PATH="$HOME/.local/bin:$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
if ! command -v tmux >/dev/null; then
    echo "Install tmux before installing the runner. launchd needs to find it on $PATH." >&2
    exit 1
fi
if [[ -n "${FARCOOLER_HOME+x}" ]]; then
    echo "Unset FARCOOLER_HOME before installing the standalone runner." >&2
    exit 1
fi
# Downloads and verification use only system tools.
export PATH="/usr/bin:/bin:/usr/sbin:/sbin"

base="$HOME/.local/share/farcooler-runner-canary"
mkdir -p "$base"
stage="$(/usr/bin/mktemp -d "$base/.download.XXXXXXXX")"
trap 'rm -r "$stage"' EXIT
fetch "$feed/$arch/latest" "$stage/index"

# A single object is the publication boundary. The first line is the signed
# manifest and the remaining lines are its armored OpenSSH signature.
head -n 1 "$stage/index" > "$stage/manifest"
tail -n +2 "$stage/index" > "$stage/manifest.sig"
# The canary public key from apps/macos/sparkle-public-keys.txt in SSH wire format.
printf '%s\n' 'canary ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAILJkUUVllLmgiPB44a8wYn8FFsqUjXZJe7jgQ8o5Y+I+' > "$stage/allowed"
if ! /usr/bin/ssh-keygen -Y verify -I canary -n runner@farcooler.com \
    -f "$stage/allowed" -s "$stage/manifest.sig" < "$stage/manifest" >/dev/null; then
    echo "The canary runner manifest signature failed verification." >&2
    exit 1
fi

IFS=' ' read -r format channel machine build checksum extra < "$stage/manifest"
if [[ "$format" != runner-v1 || "$channel" != canary || "$machine" != "$arch" ||
      ! "$build" =~ ^[1-9][0-9]{0,15}$ || ! "$checksum" =~ ^[0-9a-f]{64}$ || -n "$extra" ]]; then
    echo "The signed runner manifest does not describe a canary build for this Mac." >&2
    exit 1
fi
current="$(/usr/bin/readlink "$base/current" || true)"
if [[ "$current" =~ ^releases/([1-9][0-9]{0,15})[-] ]] && (( build < BASH_REMATCH[1] )); then
    echo "The canary feed is older than the installed runner." >&2
    exit 1
fi

fetch "$feed/$arch/$build-$checksum.tar.gz" "$stage/runner.tar.gz"
actual="$(/usr/bin/shasum -a 256 "$stage/runner.tar.gz")"
if [[ "${actual%% *}" != "$checksum" ]]; then
    echo "The canary runner archive checksum failed verification." >&2
    exit 1
fi
mkdir "$stage/payload"
/usr/bin/tar -xzf "$stage/runner.tar.gz" -C "$stage/payload"
for binary in farcooler-canary farcoolerd-canary farcooler-lfs-filter; do
    /usr/bin/codesign --verify --strict "$stage/payload/$binary"
done
"$stage/payload/farcooler-canary" runner standalone-activate \
    --build "$build" --payload "$stage/payload" "$@"
