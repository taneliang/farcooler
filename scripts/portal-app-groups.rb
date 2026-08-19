#!/usr/bin/env ruby
# Attach each channel's App Group to the five App IDs that need it.
#
# The App Store Connect API cannot do this. It can register a bundle id and it
# can toggle the APP_GROUPS capability, but it has no `/v1/appGroups` and no way
# to say WHICH group an App ID gets — so a profile comes back without
# `com.apple.security.application-groups` and every target fails to sign with
# "doesn't match the entitlements file's value". Apple has left that gap open
# for years; see the developer forums thread on it.
#
# So this drives the developer portal the only way anything can: spaceship,
# which speaks the portal's own private API using an Apple ID cookie session
# rather than an App Store Connect key. That is why this needs a 2FA login and
# the rest of this repo's Apple automation does not.
#
# Usage:
#   brew install fastlane         # or: gem install fastlane
#   ruby scripts/portal-app-groups.rb local
#   ruby scripts/portal-app-groups.rb local canary preview stable
#
# Idempotent. Creating a group that exists is skipped, and re-associating a
# group an App ID already has is a no-op, so re-running after adding a target
# costs nothing.

# Homebrew's fastlane keeps its gems inside its own Cellar rather than anywhere
# a bare `ruby` looks, so `require "spaceship"` fails with a LoadError that
# reads like fastlane is missing when it is installed and working. Finding it
# here — rather than telling a person to export two variables — is the
# difference between this script running and this script being abandoned.
if `gem list -i spaceship 2>/dev/null`.strip != "true"
  libexec = Dir.glob("/opt/homebrew/Cellar/fastlane/*/libexec").max
  libexec ||= Dir.glob("/usr/local/Cellar/fastlane/*/libexec").max # Intel Homebrew
  if libexec
    ENV["GEM_HOME"] = libexec
    ENV["GEM_PATH"] = libexec
    Gem.clear_paths
  end
end

begin
  require "spaceship"
rescue LoadError
  abort(<<~MSG)
    Could not load spaceship.

    Install fastlane, which vendors it:
      brew install fastlane

    If it is installed somewhere this did not find, point GEM_PATH at its
    libexec and re-run:
      GEM_HOME=<path> GEM_PATH=<path> ruby scripts/portal-app-groups.rb local
  MSG
end

TEAM = ENV.fetch("FARCOOLER_TEAM_ID", "H6A2TRW47J")

# The five targets that carry the App Groups entitlement. The app and each
# extension are signed separately, so one missing association fails exactly one
# target and nothing else — which is why this list has to stay complete rather
# than assume the app's profile covers what it embeds.
SUFFIXES = ["", ".activity", ".notify", ".watchkitapp", ".watchkitapp.widgets"]

# Stable keeps the bare identifier — it is the App Store record that already
# exists, and giving it a suffix would create a second app rather than update
# the one people have. Mirrors `BUNDLE_ID` in apps/ios/generate-project.py.
def bundle_base(channel)
  channel == "stable" ? "com.farcooler.ios" : "com.farcooler.ios.#{channel}"
end

channels = ARGV.empty? ? ["local"] : ARGV
unknown = channels - %w[local canary preview stable]
abort("unknown channel(s): #{unknown.join(', ')}") unless unknown.empty?

# The Apple ID, from the environment rather than a prompt.
#
# spaceship prompts interactively when it has no username, which fails outright
# in any non-interactive shell — including the one an agent runs commands in —
# with "Missing username, and running in non-interactive shell". Reading it here
# means the only interactive step left is the 2FA itself, which genuinely cannot
# be automated and is done once by `fastlane spaceauth`.
apple_id = ENV["FASTLANE_USER"] || ENV["FARCOOLER_APPLE_ID"]
abort(<<~MSG) if apple_id.nil? || apple_id.empty?
  No Apple ID. Set one and re-run:
    FASTLANE_USER=you@example.com ruby scripts/portal-app-groups.rb #{channels.join(' ')}
MSG

begin
  Spaceship::Portal.login(apple_id)
rescue StandardError => e
  abort(<<~MSG)
    Could not sign in as #{apple_id}: #{e.message}

    If this mentions a prompt, a password or two-factor authentication, the
    session has expired or was never created. Run this ONCE in a real terminal
    — not through an agent, which has no tty for the 2FA code:

      fastlane spaceauth -u #{apple_id}

    That caches a session under ~/.fastlane/spaceship/ for about a month, after
    which this script runs without prompting.
  MSG
end

Spaceship::Portal.client.team_id = TEAM
puts "signed in as #{apple_id}, team #{TEAM}"

channels.each do |channel|
  base = bundle_base(channel)
  group_id = "group.#{base}"

  group = Spaceship::Portal.app_group.all.find { |g| g.group_id == group_id }
  if group
    puts "\n#{channel}: group #{group_id} already exists"
  else
    group = Spaceship::Portal.app_group.create!(group_id: group_id,
                                                name: "Far Cooler #{channel}")
    puts "\n#{channel}: created group #{group_id}"
  end

  SUFFIXES.each do |suffix|
    identifier = base + suffix
    app = Spaceship::Portal.app.find(identifier)
    if app.nil?
      # Not an error worth stopping for: Xcode registers these itself on the
      # first `-allowProvisioningUpdates` build, so a channel that has never
      # been built simply has nothing to attach to yet.
      puts "  #{identifier}: no such App ID yet — build the channel once, then re-run"
      next
    end

    # `associate_groups` OVERWRITES rather than adds, so this passes the whole
    # intended set. Every one of these apps wants exactly its own channel's
    # group; passing only the new one is correct here and would be a silent
    # removal if that ever stopped being true.
    app.associate_groups([group])
    puts "  #{identifier}: attached #{group_id}"
  end
end

puts "\nDone. Rebuild — Xcode re-mints profiles on each attempt, so nothing to clear."
