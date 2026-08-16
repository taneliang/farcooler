#!/usr/bin/env python3
"""Emit the Sparkle 2 appcast that tells one channel's app a newer build exists.

Sparkle polls a single URL per channel and offers whatever the one `<item>` in
it names. Not a history: Sparkle only ever offers the newest, and the GitHub
releases page is already the changelog, so a second one written in XML here
would just be a copy that drifts from the first the day someone edits one and
forgets the other.

`sparkle:minimumSystemVersion` is read out of `apps/macos/Package.swift`'s
`platforms:` line rather than typed again here. Two copies of that floor is a
way for them to disagree, and the direction that matters is the dangerous one:
raise the floor in `Package.swift` and forget this file, and the appcast keeps
telling Macs that cannot run the new build to go install it.

An unknown channel is refused outright, the way `version.sh` and
`icon-label.swift` refuse one — a name this script cannot read must not be
able to produce a feed that looks like it came from a real channel.

    ./scripts/appcast.py --channel canary --version 0.2.0 --build 1284 \\
        --url https://updates.farcooler.com/canary/Far%20Cooler-1284.dmg \\
        --length 12345678 --signature <edSignature> \\
        --notes https://github.com/farcooler/farcooler/releases/tag/v0.2.0
"""

import argparse
import email.utils
import pathlib
import re
import sys
import xml.dom.minidom
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parent.parent
PACKAGE_SWIFT = ROOT / "apps" / "macos" / "Package.swift"

# The three channels that publish a feed at all. `local` is deliberately not
# here: version.sh's `feed-url` is empty for it and the app never starts an
# updater without one, so `local` has no appcast for this script to be asked
# for — treating it as unknown rather than special-casing it means a caller
# that got the channel wrong fails the same way a genuinely bogus name does.
CHANNELS = ("canary", "preview", "stable")

SPARKLE_NS = "http://www.andymatuschak.org/xml-namespaces/sparkle"

# `platforms: [.macOS("26.0")]` — matched rather than parsed as Swift, for the
# reason proto-lint.py matches .proto by regex instead of importing a protobuf
# library: this file is ours, its shape is stable, and a generator that needed
# a Swift toolchain to answer "what is the floor" is one that stops working the
# first time this runs somewhere that toolchain is not installed (CI's `wire`
# job, which is Linux — see ci.yml).
MACOS_FLOOR = re.compile(r'\.macOS\("([^"]+)"\)')


def minimum_system_version():
    """The macOS version floor `Package.swift` actually declares."""
    text = PACKAGE_SWIFT.read_text()
    match = MACOS_FLOOR.search(text)
    if not match:
        print(
            f"could not find a .macOS(\"…\") platform floor in {PACKAGE_SWIFT}",
            file=sys.stderr,
        )
        sys.exit(1)
    return match.group(1)


def build_appcast(channel, version, build, url, length, signature, notes):
    ET.register_namespace("sparkle", SPARKLE_NS)

    rss = ET.Element("rss", {"version": "2.0"})
    channel_el = ET.SubElement(rss, "channel")
    ET.SubElement(channel_el, "title").text = f"Far Cooler ({channel})"
    ET.SubElement(channel_el, "link").text = notes
    ET.SubElement(channel_el, "description").text = f"Updates for Far Cooler, {channel} channel."
    ET.SubElement(channel_el, "language").text = "en"

    item = ET.SubElement(channel_el, "item")
    ET.SubElement(item, "title").text = f"{version} ({build})"
    ET.SubElement(item, "pubDate").text = email.utils.formatdate(usegmt=True)
    ET.SubElement(item, f"{{{SPARKLE_NS}}}releaseNotesLink").text = notes
    ET.SubElement(item, f"{{{SPARKLE_NS}}}minimumSystemVersion").text = minimum_system_version()
    ET.SubElement(
        item,
        "enclosure",
        {
            "url": url,
            "length": str(length),
            "type": "application/octet-stream",
            f"{{{SPARKLE_NS}}}version": str(build),
            f"{{{SPARKLE_NS}}}shortVersionString": version,
            f"{{{SPARKLE_NS}}}edSignature": signature,
        },
    )

    raw = ET.tostring(rss, encoding="unicode")
    # Reparsed and pretty-printed through minidom rather than emitted as one
    # line: this file is read by a person diffing a CI log or an R2 bucket
    # object, not only by Sparkle.
    pretty = xml.dom.minidom.parseString(raw).toprettyxml(indent="  ")
    # minidom's own declaration always says utf-8; ET's tree has none, so this
    # is the one line of XML in the output minidom is responsible for.
    lines = [line for line in pretty.splitlines() if line.strip()]
    return "\n".join(lines) + "\n"


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--channel", required=True)
    ap.add_argument("--version", required=True, help="the marketing version, e.g. 0.2.0")
    ap.add_argument("--build", required=True, help="the build number (commit count)")
    ap.add_argument("--url", required=True, help="the enclosure URL Sparkle downloads")
    ap.add_argument("--length", required=True, help="the enclosure size, in bytes")
    ap.add_argument("--signature", required=True, help="sign_update's sparkle:edSignature")
    ap.add_argument("--notes", required=True, help="the release notes link")
    args = ap.parse_args()

    if args.channel not in CHANNELS:
        print(
            f"unknown channel: {args.channel} (expected one of {', '.join(CHANNELS)})",
            file=sys.stderr,
        )
        return 1

    sys.stdout.write(
        build_appcast(
            args.channel,
            args.version,
            args.build,
            args.url,
            args.length,
            args.signature,
            args.notes,
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
