#!/bin/bash
# Put a KNOWN tmux on a CI runner's PATH (ov-554), and fail if it isn't.
#
# tmux 3.8 ends a pane's pipe when its program dies and writes
# `#{window_layout}` as JSON; 3.7c and older do neither. `brew install tmux`
# gave whichever version the runner image's Homebrew had: of the macOS runs
# from Oct 13 to 15, every one that drew 3.8 failed and every green one had
# drawn 3.7c. So:
#
#   - macOS builds the MACOS_VERSION release itself, from its checksummed
#     tarball, with Homebrew's configure flags (`--enable-utf8proc
#     --enable-sixel`) against Homebrew's libevent and utf8proc. Not its
#     ncurses: macOS's own has the `tmux-256color` entry, so the default TERM
#     is the same. Not jemalloc either, so a cached build never links a library
#     another image lacks. The build is cached at $RUNNER_TEMP/tmux by the
#     workflow (keyed on this file's hash, so a change here rebuilds it), and
#     reused only if it still runs and says the right version. It is linked
#     into /opt/homebrew/bin and /usr/local/bin, because a daemon a test starts
#     finds tmux through a login shell, not through $GITHUB_PATH, and the
#     script checks that the login shells find this one.
#   - Linux keeps the image's tmux, checked against LINUX_VERSION. The Rust
#     job runs on `ubuntu-24.04`, not `ubuntu-latest`, so that moves only when
#     somebody moves it.
#
# Both behaviors are covered on purpose: Ubuntu's 3.4 keeps a pipe through a
# death, 3.8 ends it. To move either version, run `cargo nextest run --profile
# ci -p farcooler-tmux -p farcooler-daemon` with that tmux first on PATH, read
# its CHANGES, then change it here.
#
# Usage: scripts/ci-install-tmux.sh   (in a GitHub Actions step)
set -euo pipefail

MACOS_VERSION=3.8
MACOS_SHA256=e79c699c7e949dccd0a4a125e17b8d1261e311b16979dbc8eb34542f3966d82e
LINUX_VERSION=3.4

if [ "$(uname -s)" = "Linux" ]; then
  if ! command -v tmux >/dev/null 2>&1; then
    # Bounded: `apt-get update` once hung for an hour against a mirror that
    # accepted the connection and stopped answering.
    APT_OPTS=(-o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30)
    sudo DEBIAN_FRONTEND=noninteractive apt-get "${APT_OPTS[@]}" update
    sudo DEBIAN_FRONTEND=noninteractive apt-get "${APT_OPTS[@]}" install -y tmux
  fi
  want="tmux $LINUX_VERSION"
else
  want="tmux $MACOS_VERSION"
  prefix="${RUNNER_TEMP:?}/tmux"
  # The binary links these dynamically, so they are installed even when the
  # build itself comes from the cache. Without the auto-update, which costs a
  # minute or more and changes what gets installed from run to run.
  HOMEBREW_NO_AUTO_UPDATE=1 brew install libevent utf8proc pkgconf
  if [ "$("$prefix/bin/tmux" -V 2>/dev/null || true)" != "$want" ]; then
    rm -rf "$prefix"
    work="$(mktemp -d "$RUNNER_TEMP/tmux-src.XXXXXX")"
    curl -fsSL --retry 3 -o "$work/tmux.tar.gz" \
      "https://github.com/tmux/tmux/releases/download/$MACOS_VERSION/tmux-$MACOS_VERSION.tar.gz"
    echo "$MACOS_SHA256  $work/tmux.tar.gz" | shasum -a 256 -c
    tar xzf "$work/tmux.tar.gz" -C "$work"
    (
      cd "$work/tmux-$MACOS_VERSION"
      ./configure --prefix="$prefix" --enable-utf8proc --enable-sixel --disable-jemalloc
      make -j3 install
    )
    rm -rf "$work"
  fi
  export PATH="$prefix/bin:$PATH"
  if [ -n "${GITHUB_PATH:-}" ]; then
    echo "$prefix/bin" >> "$GITHUB_PATH"
  fi
  # $GITHUB_PATH reaches the steps, not a daemon a test starts with a stripped
  # PATH. That daemon finds tmux the way `farcooler_core::programs::find` does:
  # the login shell's PATH, then known prefixes. /usr/local/bin is on both
  # (path_helper's /etc/paths, and KNOWN_PREFIXES), so the build is linked
  # there. Without it the daemon tests and the Mac's RealCLI tests found no
  # tmux at all (run 38698364896): Homebrew's used to sit there. Homebrew's
  # own bin too, which a profile's `brew shellenv` puts first, so no tmux the
  # image's Homebrew might have can come ahead of this one. A runner is thrown
  # away after the job; a local run names its own directories.
  for link_dir in ${TMUX_LINK_DIRS:-/opt/homebrew/bin /usr/local/bin}; do
    if [ -d "$link_dir" ] && [ -w "$link_dir" ]; then
      ln -sf "$prefix/bin/tmux" "$link_dir/tmux"
    else
      sudo mkdir -p "$link_dir"
      sudo ln -sf "$prefix/bin/tmux" "$link_dir/tmux"
    fi
  done
fi

fail() {
  echo "::error::$1 See the header of scripts/ci-install-tmux.sh before changing it."
  exit 1
}

have="$(tmux -V)"
echo "on this step's PATH: $have"
[ "$have" = "$want" ] || fail "This runner has $have, and the script expects $want."

# And what a daemon started with nothing inherited would find: the login
# shells `programs::find` asks. /bin/sh is the one a test run names
# (FARCOOLER_TEST_SHELL in .cargo/config.toml); the user's own is the one
# anything else uses.
if [ "$(uname -s)" = "Linux" ]; then
  users_shell="$(getent passwd "$(id -un)" | cut -d: -f7)"
else
  users_shell="$(dscl . -read "/Users/$(id -un)" UserShell | awk '{print $2}')"
fi
for shell in /bin/sh "$users_shell"; do
  [ -n "$shell" ] || continue
  seen="$(env -i HOME="$HOME" USER="$(id -un)" "$shell" -lc 'tmux -V' 2>/dev/null || true)"
  echo "a login $shell finds: ${seen:-no tmux}"
  [ "$seen" = "$want" ] || fail "A login $shell finds ${seen:-no tmux}, not $want: a daemon would run that."
done
