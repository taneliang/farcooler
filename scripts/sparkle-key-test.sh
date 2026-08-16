#!/bin/bash
# That we can derive a Sparkle public key from its private half, correctly.
#
# CI uses this derivation to refuse to sign with a key that is not the pair of
# the one every installed app trusts. That check is only worth having if the
# derivation is right, and "right" here has a published answer: RFC 8032's
# ed25519 test vectors. A key pair we invented ourselves would only prove the
# code agrees with itself.
#
# The failure this guards against is the quietest one in the update path. A
# mismatched pair signs happily, publishes happily, serves happily, and is
# refused by every client — because a signature that does not verify looks
# exactly like a forged one from Sparkle's side. Nothing in CI would ever know.
#
#   ./scripts/sparkle-key-test.sh
set -euo pipefail

cd "$(dirname "$0")/.."
PASS=0
FAIL=0

check() {
  local what="$1" want="$2" got="$3"
  if [ "$want" = "$got" ]; then
    PASS=$((PASS + 1))
  else
    FAIL=$((FAIL + 1))
    echo "FAIL: $what" >&2
    echo "  want: $want" >&2
    echo "  got:  $got" >&2
  fi
}

b64() { python3 -c "import base64,binascii,sys;print(base64.b64encode(binascii.unhexlify(sys.argv[1])).decode())" "$1"; }
derive() { printf '%s' "$1" | swift scripts/sparkle-public-key.swift; }

# --- RFC 8032, section 7.1, test vector 1 ---------------------------------
check "the first published vector derives its published public key" \
  "$(b64 d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a)" \
  "$(derive "$(b64 9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60)")"

# --- RFC 8032, section 7.1, test vector 2 ---------------------------------
#
# A second vector, because one passing vector is also what a function that
# ignored its input and printed a constant would produce.
check "the second published vector derives a different, published key" \
  "$(b64 3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c)" \
  "$(derive "$(b64 4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb)")"

# --- refusals, because a wrong key must never derive a plausible answer ----
#
# Each of these is a way a secret can arrive damaged — unset, mangled in
# transit, or in Sparkle's older 64-byte format — and each must stop CI rather
# than produce a key that merely looks like one.
refused() {
  set +e
  printf '%s' "$1" | swift scripts/sparkle-public-key.swift >/dev/null 2>&1
  local code=$?
  set -e
  echo "$code"
}

check "an empty key is refused" "1" "$(refused "")"
check "a key that is not base64 is refused" "1" "$(refused 'not base64 at all!!')"
check "a key of the wrong length is refused" "1" "$(refused "$(b64 0011223344556677)")"
check "an older 64-byte key is refused rather than half-read" "1" \
  "$(refused "$(b64 9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a)")"

echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
