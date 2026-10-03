#!/bin/bash
# What canary-baseline.sh must do, against real git repositories and a real
# remote. Its only other test is a Canary run on main, where a mistake either
# records nothing — and proto-lint passes saying nothing shipped — or moves the
# baseline backwards and loses what the field is owed.
#
#   ./scripts/canary-baseline-test.sh
set -euo pipefail

cd "$(dirname "$0")/.."
SCRIPT="${CANARY_BASELINE_SCRIPT:-$PWD/scripts/canary-baseline.sh}"
PASS=0
FAIL=0

check() {
  local what="$1" want="$2" got="$3"
  if [ "$want" = "$got" ]; then
    PASS=$((PASS + 1))
  else
    FAIL=$((FAIL + 1))
    echo "FAIL: $what: want '$want', got '$got'" >&2
  fi
}

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
git_q() { git -c user.name=t -c user.email=t@t -c init.defaultBranch=main "$@"; }

git_q init --quiet --bare "$scratch/origin.git"
git_q clone --quiet "$scratch/origin.git" "$scratch/dev" 2>/dev/null
dev="$scratch/dev"

msg() { printf 'message %s { string %s = 1; }\n' "$1" "$1"; }

# A commit on main whose proto gains message $1, if it lacks it; prints its sha.
# Additive, as every real change must be: the script refuses a baseline that
# would break the one before it.
land() {
  mkdir -p "$dev/proto"
  touch "$dev/proto/farcooler.proto"
  grep -qx "$(msg "$1")" "$dev/proto/farcooler.proto" || msg "$1" >> "$dev/proto/farcooler.proto"
  echo "$2" > "$dev/other"
  git_q -C "$dev" add proto/farcooler.proto other
  git_q -C "$dev" commit --quiet -m "$1 $2"
  git_q -C "$dev" pull --quiet --rebase origin main 2>/dev/null || true
  git_q -C "$dev" push --quiet origin HEAD:main
  git -C "$dev" rev-parse HEAD
}

# The job: a fresh full clone of main, then the script.
record() {
  rm -rf "$scratch/ci"
  git clone --quiet "$scratch/origin.git" "$scratch/ci" 2>/dev/null
  (cd "$scratch/ci" && "$SCRIPT" "$1")
}

# The newest message in the baseline, which is the last land it recorded.
baseline_proto() { git -C "$scratch/origin.git" show main:proto/baseline/canary.proto | tail -n 1; }
baseline_sha() { git -C "$scratch/origin.git" show main:proto/baseline/canary.proto | head -1 | sed 's/.* at \([0-9a-f]*\)\..*/\1/'; }
commits() { git -C "$scratch/origin.git" rev-list --count main; }

a="$(land alpha 1)"
record "$a" >/dev/null
check "a first baseline is committed" "$(msg alpha)" "$(baseline_proto)"
check "and names the commit it came from" "$a" "$(baseline_sha)"

b="$(land alpha 2)"
before="$(commits)"
out="$(record "$b")"
check "an unchanged wire commits nothing" "$before" "$(commits)"
check "and says so" "canary baseline unchanged" "$out"
check "and keeps the first commit that shipped it" "$a" "$(baseline_sha)"

c="$(land gamma 3)"
# Main moves on past the shipped commit before the job runs: the proto comes
# from the commit that shipped, not from main.
land delta 4 >/dev/null
record "$c" >/dev/null
check "a changed wire is recorded from the shipped commit" "$(msg gamma)" "$(baseline_proto)"

before="$(commits)"
out="$(record "$b")"
check "a re-run of an older commit does not move it backwards" "$(msg gamma)" "$(baseline_proto)"
check "and commits nothing" "$before" "$(commits)"
case "$out" in *"::warning::"*"not advanced"*) got=warned ;; *) got="$out" ;; esac
check "and warns that it did not advance" warned "$got"

# A push that loses a race retries from the new main. The job's clone is taken,
# then a human push lands before the script runs; its fetch sees the new main.
e="$(land epsilon 5)"
rm -rf "$scratch/ci"
git clone --quiet "$scratch/origin.git" "$scratch/ci" 2>/dev/null
land epsilon 6 >/dev/null
(cd "$scratch/ci" && "$SCRIPT" "$e") >/dev/null
check "recorded on top of a push that landed first" "$(msg epsilon)" "$(baseline_proto)"
check "without dropping that push" "6" "$(git -C "$scratch/origin.git" show main:other)"

# A rejected push is retried rather than failing the job. The remote refuses
# the first push it is offered, as a lost race would.
f="$(land zeta 7)"
hook="$scratch/origin.git/hooks/pre-receive"
printf '#!/bin/sh\n[ -e "%s" ] && exit 0\ntouch "%s"\nexit 1\n' "$scratch/refused" "$scratch/refused" > "$hook"
chmod +x "$hook"
record "$f" >/dev/null 2>&1 || true
rm -f "$hook"
check "a rejected push is retried" "$(msg zeta)" "$(baseline_proto)"

# A commit dispatched from a branch is not recorded, and says why. Recorded, it
# would be a header no later main commit descends from, and the baseline would
# stop advancing for good.
git_q -C "$dev" checkout --quiet -b feature
msg eta >> "$dev/proto/farcooler.proto"
git_q -C "$dev" commit --quiet -am eta
g="$(git -C "$dev" rev-parse HEAD)"
git_q -C "$dev" push --quiet origin feature
git_q -C "$dev" checkout --quiet main
before="$(commits)"
out="$(record "$g" 2>&1)"
check "a commit not on main is not recorded" "$(msg zeta)" "$(baseline_proto)"
check "and commits nothing" "$before" "$(commits)"
case "$out" in *"::warning::"*"not on main"*) got=warned ;; *) got="$out" ;; esac
check "and warns" warned "$got"
h="$(land theta 9)"
record "$h" >/dev/null
check "and main still advances after it" "$(msg theta)" "$(baseline_proto)"

# A push that loses a race is never forced over the winner. A `git` shim lands
# a commit on main after the script's fetch and just before its first push, so
# the push is refused as non-fast-forward — unless it is forced, which would
# erase the commit that landed.
i="$(land iota 10)"
real_git="$(command -v git)"
mkdir -p "$scratch/shim"
cat > "$scratch/shim/git" <<SHIM
#!/bin/bash
if [ "\$1" = push ] && [ ! -e "$scratch/raced" ]; then
  touch "$scratch/raced"
  printf 'x\n' > "$dev/raced"
  "$real_git" -C "$dev" -c user.name=t -c user.email=t@t add raced
  "$real_git" -C "$dev" -c user.name=t -c user.email=t@t commit --quiet -m raced
  "$real_git" -C "$dev" push --quiet origin HEAD:main
fi
exec "$real_git" "\$@"
SHIM
chmod +x "$scratch/shim/git"
rm -rf "$scratch/ci"
git clone --quiet "$scratch/origin.git" "$scratch/ci" 2>/dev/null
(cd "$scratch/ci" && PATH="$scratch/shim:$PATH" "$SCRIPT" "$i") >/dev/null 2>&1 || true
check "the race was staged" yes "$([ -e "$scratch/raced" ] && echo yes || echo no)"
check "a push that lost a race keeps the winner" x "$(git -C "$scratch/origin.git" show main:raced 2>/dev/null)"
check "and still records" "$(msg iota)" "$(baseline_proto)"

# A break is refused, and says so. Canary's own `wire` job keeps one from
# shipping; this is the lock that holds if that gate is ever lost. Recorded, the
# break would BE the baseline, and every later lint would pass against it.
git_q -C "$dev" pull --quiet --rebase origin main 2>/dev/null
grep -vx "$(msg alpha)" "$dev/proto/farcooler.proto" > "$scratch/removed"
cp "$scratch/removed" "$dev/proto/farcooler.proto"
echo 11 > "$dev/other"
git_q -C "$dev" commit --quiet -am "alpha removed"
git_q -C "$dev" push --quiet origin HEAD:main
k="$(git -C "$dev" rev-parse HEAD)"
before="$(commits)"
was="$(git -C "$scratch/origin.git" show main:proto/baseline/canary.proto)"
status=0
out="$(record "$k" 2>&1)" || status=$?
check "a break fails the job" 1 "$status"
check "and leaves the baseline as it was" "$was" "$(git -C "$scratch/origin.git" show main:proto/baseline/canary.proto)"
check "with the removed field still in it" yes "$(git -C "$scratch/origin.git" show main:proto/baseline/canary.proto | grep -qx "$(msg alpha)" && echo yes || echo no)"
check "and commits nothing" "$before" "$(commits)"
case "$out" in *"::error::"*"alpha tag 1 (alpha) was removed"*) got=said ;; *) got="$out" ;; esac
check "and says what broke" said "$got"

# No lint beside the script: refused, and said so, rather than recorded
# unchecked or blamed on a break that may not exist.
mkdir -p "$scratch/nolint"
cp "$SCRIPT" "$scratch/nolint/canary-baseline.sh"
l="$(land kappa 12)"
before="$(commits)"
status=0
out="$(cd "$scratch/ci" && "$scratch/nolint/canary-baseline.sh" "$l" 2>&1)" || status=$?
check "a missing lint fails the job" 1 "$status"
check "and commits nothing" "$before" "$(commits)"
case "$out" in *"::error::the wire lint is unavailable"*) got=said ;; *) got="$out" ;; esac
check "and says the lint is unavailable" said "$got"

# A lint that crashes: the same, with its own words.
printf '#!/bin/sh\necho Traceback >&2\nexit 1\n' > "$scratch/nolint/proto-lint.py"
chmod +x "$scratch/nolint/proto-lint.py"
status=0
out="$(cd "$scratch/ci" && "$scratch/nolint/canary-baseline.sh" "$l" 2>&1)" || status=$?
check "a crashing lint fails the job" 1 "$status"
check "and commits nothing" "$before" "$(commits)"
case "$out" in *"::error::the wire lint failed to run"*) got=said ;; *) got="$out" ;; esac
check "and says the lint failed to run" said "$got"

echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
