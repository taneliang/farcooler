#!/bin/bash
# The Apple Development identity the iOS archive signs with.
#
# The archive uses automatic signing with `-allowProvisioningUpdates` and an App
# Store Connect key, which is what lets it create the provisioning profiles a
# fresh runner cannot have. Profiles are free and unlimited. CERTIFICATES ARE
# NOT: Apple caps how many an account may hold, and cloud signing does not know
# the difference. A runner with an empty keychain has no Apple Development
# identity, so Xcode dutifully creates one — signs with it, uploads a perfectly
# good build, and is then destroyed along with the private key.
#
# It looks like it works, and it does, for about eleven runs. Then every archive
# fails with `Choose a certificate to revoke. Your account has reached the
# maximum number of certificates.`, followed by `No profiles for
# 'com.farcooler.ios.canary' were found` — which is not a second problem. No
# certificate could be made, so no profile could be made either, and the profile
# error is the one that reads like the cause.
#
# So the identity is imported rather than minted. Automatic signing reuses a
# valid one when the keychain has it and only creates a certificate when it
# finds none, which makes this script the whole fix: the API key keeps making
# profiles, and stops making certificates.
#
# This exists as a script rather than as YAML because canary.yml and release.yml
# both archive for the phone, and a copy in each is a copy that gets fixed once.
#
#   IOS_CERTIFICATE=… IOS_CERTIFICATE_PASSWORD=… ./scripts/import-ios-certificate.sh
#
# On a runner only, and the guard below is not caution for its own sake: this
# changes the DEFAULT keychain and does not change it back, because the archive
# that needs it runs in a later step. On a laptop that is somebody's login
# keychain being quietly dropped out of the way for the rest of the session.
#
# EMPTY IS AN ERROR, unlike the WorkOS client id next door, and the difference is
# the point. A build with no client id merely has no sign-in button. A build with
# no certificate goes back to minting one per run — the exact failure this
# script exists to end, silently, until the account fills up again. Call it only
# from a step that is already gated on having the App Store key.
set -euo pipefail

if [ -z "${GITHUB_ACTIONS:-}" ]; then
  echo "This is for a runner: it makes a throwaway keychain the default one and leaves it that way, which on a laptop displaces your login keychain for the rest of the session." >&2
  exit 1
fi

if [ -z "${IOS_CERTIFICATE:-}" ]; then
  echo "IOS_CERTIFICATE is empty: without it the archive would create a new Apple Development certificate on every run, until the account cannot hold another." >&2
  exit 1
fi

# The password may legitimately be empty — a `.p12` can be exported without one
# — so it is defaulted rather than required.
password="${IOS_CERTIFICATE_PASSWORD:-}"

# In RUNNER_TEMP, which is wiped with the runner; the fallback is for a
# self-hosted one that somehow has not set it, and is not a way around the guard
# above. Named, not `build.keychain` like the Mac job's, so that a future job
# doing both cannot have one clobber the other.
keychain="${RUNNER_TEMP:-$(mktemp -d)}/ios-signing.keychain-db"
keychain_password=actions

p12="$(mktemp -t ios-certificate)"
trap 'rm -f "$p12"' EXIT
echo "$IOS_CERTIFICATE" | base64 --decode > "$p12"

security create-keychain -p "$keychain_password" "$keychain"
# A fresh keychain locks itself after five minutes idle by default, and an
# archive plus an export takes longer than that. The lock does not fail the
# build in any legible way: codesign simply stops finding the identity, and
# automatic signing goes looking for a certificate to create.
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"

# `-f pkcs12` explicitly. Without it `security` guesses the format from the
# FILE EXTENSION, and a `mktemp` name has none — so a perfectly good
# certificate fails with `SecKeychainItemImport: Unknown format in import`,
# which reads like the secret is corrupt.
security import "$p12" -f pkcs12 -k "$keychain" -P "$password" \
  -T /usr/bin/codesign -T /usr/bin/security
# Without this, codesign finds the key and then blocks forever on a UI prompt
# for permission to use it — on a runner with nobody to click it.
security set-key-partition-list -S apple-tool:,apple: \
  -s -k "$keychain_password" "$keychain" > /dev/null

# Both, and neither alone is reliably enough: the search list is what xcodebuild
# looks through, and the default keychain is what some of codesign's paths reach
# for first.
#
# `-s` REPLACES the search list rather than adding to it, so the existing
# entries are read back and passed along — otherwise the login keychain
# disappears out from under everything else in the job. `read` without `IFS=`
# is deliberate: one variable takes the whole line with the indentation
# trimmed, leaving the quotes to strip and any spaces inside a path intact.
keychains=("$keychain")
while read -r existing; do
  keychains+=("${existing//\"/}")
done < <(security list-keychains -d user)
security list-keychains -d user -s "${keychains[@]}"
security default-keychain -s "$keychain"

# Worth asserting, and worth asserting in two parts, because the two ways this
# goes wrong have nothing to do with each other and one error message for both
# would point at the wrong fix half the time. Either way the build that follows
# would go back to creating a certificate per run — invisible until the account
# is full again, weeks later.

# `find-identity` without `-v` lists identities whether or not they are usable.
# Nothing here means the import brought a certificate and no private key, which
# `security import` accepts in silence.
if ! security find-identity -p codesigning "$keychain" | grep -q "Apple Development"; then
  echo "No Apple Development identity in the imported certificate: export the .p12 from Keychain Access with the private key underneath it, not the certificate alone." >&2
  exit 1
fi

# `-v` is `valid` — the identity chains to a trusted root and has not expired.
# An identity that is present but not valid is one codesign will refuse, and on
# a runner the usual cause is the Apple WWDR intermediate missing from the
# `.p12`: it is on the exporting Mac already, so it is easy to leave behind.
if ! security find-identity -v -p codesigning "$keychain" | grep -q "Apple Development"; then
  echo "The Apple Development identity imported but is not valid: it has expired, or the .p12 was exported without the Apple Worldwide Developer Relations intermediate above it." >&2
  security find-identity -v -p codesigning "$keychain" >&2
  exit 1
fi

echo "Imported an Apple Development identity; cloud signing will reuse it"
