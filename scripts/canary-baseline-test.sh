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

# A commit on main whose proto says $1; prints its sha.
land() {
  mkdir -p "$dev/proto"
  printf 'message Foo { string %s = 1; }\n' "$1" > "$dev/proto/farcooler.proto"
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

baseline_proto() { git -C "$scratch/origin.git" show main:proto/baseline/canary.proto | tail -n +2; }
baseline_sha() { git -C "$scratch/origin.git" show main:proto/baseline/canary.proto | head -1 | sed 's/.* at \([0-9a-f]*\)\..*/\1/'; }
commits() { git -C "$scratch/origin.git" rev-list --count main; }

a="$(land alpha 1)"
record "$a" >/dev/null
check "a first baseline is committed" "message Foo { string alpha = 1; }" "$(baseline_proto)"
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
check "a changed wire is recorded from the shipped commit" "message Foo { string gamma = 1; }" "$(baseline_proto)"

before="$(commits)"
out="$(record "$b")"
check "a re-run of an older commit does not move it backwards" "message Foo { string gamma = 1; }" "$(baseline_proto)"
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
check "recorded on top of a push that landed first" "message Foo { string epsilon = 1; }" "$(baseline_proto)"
check "without dropping that push" "6" "$(git -C "$scratch/origin.git" show main:other)"

# A rejected push is retried rather than failing the job. The remote refuses
# the first push it is offered, as a lost race would.
f="$(land zeta 7)"
hook="$scratch/origin.git/hooks/pre-receive"
printf '#!/bin/sh\n[ -e "%s" ] && exit 0\ntouch "%s"\nexit 1\n' "$scratch/refused" "$scratch/refused" > "$hook"
chmod +x "$hook"
record "$f" >/dev/null 2>&1 || true
rm -f "$hook"
check "a rejected push is retried" "message Foo { string zeta = 1; }" "$(baseline_proto)"

# A commit dispatched from a branch is not recorded, and says why. Recorded, it
# would be a header no later main commit descends from, and the baseline would
# stop advancing for good.
git_q -C "$dev" checkout --quiet -b feature
printf 'message Foo { string eta = 1; }\n' > "$dev/proto/farcooler.proto"
git_q -C "$dev" commit --quiet -am eta
g="$(git -C "$dev" rev-parse HEAD)"
git_q -C "$dev" push --quiet origin feature
git_q -C "$dev" checkout --quiet main
before="$(commits)"
out="$(record "$g" 2>&1)"
check "a commit not on main is not recorded" "message Foo { string zeta = 1; }" "$(baseline_proto)"
check "and commits nothing" "$before" "$(commits)"
case "$out" in *"::warning::"*"not on main"*) got=warned ;; *) got="$out" ;; esac
check "and warns" warned "$got"
h="$(land theta 9)"
record "$h" >/dev/null
check "and main still advances after it" "message Foo { string theta = 1; }" "$(baseline_proto)"

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
check "and still records" "message Foo { string iota = 1; }" "$(baseline_proto)"

echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
