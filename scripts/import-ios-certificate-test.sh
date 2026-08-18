#!/bin/bash
# What import-ios-certificate.sh does, against real keychains.
#
# Worth testing rather than eyeballing, because every one of its failures is
# quiet by nature: the script exists to stop the archive from minting a
# certificate per run, and a script that silently does nothing looks exactly
# like a script that works — until the developer account is full again.
#
# The certificates here are self-signed and last a day. That covers everything
# except the final `-v` check, which asks whether an identity chains to a
# trusted root: a self-signed one cannot, by construction. So the happy path
# proper is the archive step on a runner, and what is checked here is that the
# identity really lands in the keychain and that the two ways it can fail say
# different things.
#
# Safe to run on a laptop: it saves the default keychain and the search list and
# puts both back on the way out, including on failure.
set -uo pipefail

# `|| exit`, because there is deliberately no `set -e` here: a failing check
# must be counted and reported rather than end the run at the first one.
cd "$(dirname "$0")/.." || exit 1
script="$PWD/scripts/import-ios-certificate.sh"

work="$(mktemp -d)"

original_default="$(security default-keychain | sed -e 's/^ *"//' -e 's/"$//')"
original_list="$(security list-keychains -d user)"

restore() {
  keychains=()
  while read -r entry; do keychains+=("${entry//\"/}"); done <<< "$original_list"
  security list-keychains -d user -s "${keychains[@]}"
  security default-keychain -s "$original_default"
  for keychain in "$work"/*/ios-signing.keychain-db; do
    [ -e "$keychain" ] && security delete-keychain "$keychain" 2> /dev/null
  done
  rm -rf "$work"
}
trap restore EXIT

passed=0
failed=0

check() {
  local name="$1" want="$2" got="$3"
  if [ "$got" = "$want" ]; then
    passed=$((passed + 1))
  else
    echo "FAIL  $name (exit $got, wanted $want)" >&2
    failed=$((failed + 1))
  fi
}

# A certificate and its key, in the one format `security import` reads.
# `-legacy`, because a PKCS#12 encrypted the way OpenSSL 3 prefers is one macOS
# refuses to open at all.
make_p12() {
  openssl req -x509 -newkey rsa:2048 -keyout "$work/$1.key" -out "$work/$1.crt" \
    -days 1 -nodes -subj "/CN=$2" -addext "extendedKeyUsage=codeSigning" 2> /dev/null
  openssl pkcs12 -export -inkey "$work/$1.key" -in "$work/$1.crt" \
    -out "$work/$1.p12" -passout pass:test -legacy 2> /dev/null
}

run() { # directory, base64 certificate — stderr lands in $work/err
  mkdir -p "$work/$1"
  GITHUB_ACTIONS=true RUNNER_TEMP="$work/$1" \
    IOS_CERTIFICATE="$2" IOS_CERTIFICATE_PASSWORD=test "$script" 2> "$work/err" > /dev/null
}

# Off a runner it must not touch the keychain at all, whatever else is set.
env -u GITHUB_ACTIONS IOS_CERTIFICATE=irrelevant "$script" > /dev/null 2>&1
check "refuses to run off a runner" 1 $?

# An empty secret is the failure this script was written for: it must stop,
# rather than hand the archive back to cloud signing.
run empty ""
check "refuses an empty certificate" 1 $?
grep -q "cannot hold another" "$work/err"
check "  and says what an empty one costs" 0 $?

# A certificate with no Apple Development identity in it — the shape of a `.p12`
# exported without the private key underneath.
make_p12 wrong "Some Other Certificate"
run wrong "$(base64 < "$work/wrong.p12")"
check "refuses a certificate with no Apple Development identity" 1 $?
grep -q "with the private key underneath it" "$work/err"
check "  and names the private key" 0 $?

# Present, importable, and not valid, which must read as its own problem rather
# than as the missing key above.
make_p12 good "Apple Development: Test Person (ABCDE12345)"
run good "$(base64 < "$work/good.p12")"
check "refuses an identity that is not valid" 1 $?
grep -q "imported but is not valid" "$work/err"
check "  and distinguishes that from a missing key" 0 $?

# The import itself, which is the part the rest of this file is really about.
# It is also where `security import` guesses the format from a file extension
# the script's temporary file does not have — a wrong guess reads as `Unknown
# format in import`, as though the secret were corrupt.
security find-identity -p codesigning "$work/good/ios-signing.keychain-db" \
  | grep -q "Apple Development"
check "really imported the identity into its keychain" 0 $?

# The search list is REPLACED by `-s`, so the login keychain has to be passed
# back deliberately. Losing it breaks every later step that reads a credential.
security list-keychains -d user | grep -q "$original_default"
check "kept the existing keychains in the search list" 0 $?
security list-keychains -d user | grep -q "$work/good/ios-signing.keychain-db"
check "put the imported keychain in the search list" 0 $?
security default-keychain | grep -q "$work/good/ios-signing.keychain-db"
check "made the imported keychain the default one" 0 $?

if [ "$failed" -eq 0 ]; then
  echo "import-ios-certificate.sh: $passed checks passed"
else
  echo "import-ios-certificate.sh: $failed of $((passed + failed)) checks failed" >&2
  exit 1
fi
