#!/usr/bin/perl
# A stand-in for claude's or codex's TUI, for answer_wake's tests: never the
# real agent. Run as a copy of perl named `claude` or `codex`, so the pane's
# foreground process carries that name as the real one does.
#
#   <agent> stand_in.pl <claude|codex> <control-file> <log-file>
#
# It draws the agent's idle box (claude: `❯` between two `─` rules; codex:
# `›` over a blank line and the model footer, the placeholder dim), turns on
# bracketed paste, appends a paste or typed characters to its box, and on
# Enter logs `SUBMIT <box>` and clears it. The control file picks what it
# shows: idle, working, menu (a permission prompt), picker (a menu with no
# box), mangle (a paste shows as `[Pasted text #1]`), slow (a paste shows
# a second late), nobracket (bracketed paste off), draft:<text> (a box
# already holding <text>), or menu-on-paste (idle until a paste arrives, then
# the menu, drawn in the same pass that logs PASTE, so a test never races a
# menu against the read-back).
#
# While working, Enter queues the box as the real agents do (ov-360): it
# logs `QUEUED <box>`, draws it above the box as claude 2.1.290 or codex
# 0.153.4 does, and submits it when the mode next turns idle. A claude
# stand-in with CLAUDE_CONFIG_DIR set keeps claude's session registry and
# transcript there, an `enqueue` record per queued message; working-quiet
# works the same and records nothing.
use strict;
use warnings;
use utf8;
use Encode qw(decode_utf8);

my ($agent, $control, $log) = @ARGV;
binmode STDOUT, ':utf8';
$| = 1;
system("stty raw -echo 2>/dev/null");
print "\e[?2004h";

my ($composer, $mode, $pasting, $late, $late_at, @said, @queued) = ("", "", 0, "", 0);
my $transcript;
if ($agent eq 'claude' && ($ENV{CLAUDE_CONFIG_DIR} // '') ne '') {
    use Cwd qw(getcwd);
    my ($config, $cwd) = ($ENV{CLAUDE_CONFIG_DIR}, getcwd());
    (my $project = $cwd) =~ s/[^A-Za-z0-9]/-/g;
    mkdir "$config/sessions";
    mkdir "$config/projects";
    mkdir "$config/projects/$project";
    $transcript = "$config/projects/$project/stand-in.jsonl";
    open(my $f, '>:utf8', "$config/sessions/$$.json") or die;
    print $f '{"pid":' . $$ . ',"sessionId":"stand-in","cwd":' . json($cwd) . '}';
    close $f;
}

sub json {
    my ($s) = @_;
    $s =~ s/(["\\])/\\$1/g;
    $s =~ s/([\x00-\x1f])/sprintf("\\u%04x", ord($1))/ge;
    return "\"$s\"";
}

sub record {
    return unless defined $transcript;
    open(my $f, '>>:utf8', $transcript) or return;
    print $f "$_[0]\n";
    close $f;
}

sub working { return $_[0] eq 'working' || $_[0] eq 'working-quiet' }

sub mode {
    open(my $f, '<', $control) or return "idle";
    local $/;
    my $m = <$f> // "";
    close $f;
    $m = decode_utf8($m);
    $m =~ s/\s+\z//;
    return $m eq "" ? "idle" : $m;
}

sub logit {
    open(my $f, '>>:utf8', $log) or return;
    print $f "$_[0]\n";
    close $f;
}

sub wrap {
    my ($text, $width) = @_;
    my @lines = ("");
    for my $word (split / /, $text) {
        if ($lines[-1] ne "" && length($lines[-1]) + 1 + length($word) > $width) {
            push @lines, $word;
        } else {
            $lines[-1] = $lines[-1] eq "" ? $word : "$lines[-1] $word";
        }
    }
    return @lines;
}

sub draw {
    my @rows;
    my @box = wrap($composer, 60);
    if ($agent eq 'claude') {
        push @rows, " Claude Code stand-in", "";
        push @rows, map { "⏺ $_" } @said;
        push @rows, "";
        if ($mode eq 'menu') {
            push @rows, " Do you want to create haiku.txt?", " ❯ 1. Yes", "   2. No", "",
                " Esc to cancel · Tab to amend";
        } elsif ($mode eq 'picker') {
            push @rows, " Select model", " ❯ Default", "   Opus", "", "  ? for shortcuts";
        } else {
            my $rule = "─" x 70;
            push @rows, map { ("❯ $_", "  ctrl+x ctrl+s to send now") } @queued;
            push @rows, "✻ Pondering… (3s)" if working($mode);
            my $first = shift(@box);
            $first = "\e[7mP\e[0;2mress up to edit queued messages\e[0m" if $composer eq "" && @queued;
            push @rows, $rule, "❯\x{a0}" . $first, (map { "  $_" } @box), $rule;
            # claude drops `esc to interrupt` while its box holds something.
            push @rows, working($mode) && $composer eq "" ? "  ⏸ manual mode on · esc to interrupt"
                : working($mode) ? "  ⏸ manual mode on" : "  ⏸ manual mode on · ? for shortcuts";
        }
    } else {
        push @rows, " OpenAI Codex stand-in", "";
        push @rows, map { "• $_" } @said;
        push @rows, "";
        push @rows, "• Working (3s • esc to interrupt)" if working($mode);
        if (@queued) {
            push @rows, "• Messages to be submitted after next tool call (press esc to interrupt and send immediately)";
            push @rows, map { "  ↳ $_" } @queued;
        }
        if ($composer eq "") {
            push @rows, "› \e[2mExplain this codebase\e[0m";
        } else {
            push @rows, "› " . shift(@box), map { "  $_" } @box;
        }
        push @rows, "", working($mode) && $composer ne "" ? "  \e[2mtab to queue message\e[0m" : "  gpt-5 high · ~/src";
    }
    print "\e[H\e[2J" . join("\r\n", @rows);
}

sub take {
    my ($text) = @_;
    $composer .= $text;
}

draw();
my $buf = "";
my $fired = 0;    # menu-on-paste's paste has arrived: the menu stays up
while (1) {
    my $now = mode();
    $fired = 0 if $now ne 'menu-on-paste';
    $now = 'menu' if $fired;
    if ($now ne $mode) {
        if (working($mode) && !working($now)) {
            for my $q (@queued) {
                logit("SUBMIT $q");
                record('{"type":"user","message":{"role":"user","content":' . json($q) . '},"promptSource":"queued"}')
                    if $mode eq 'working';
                push @said, $q;
            }
            @queued = ();
        }
        $mode = $now;
        $composer = $1 if $mode =~ /^draft:(.*)$/s;
        print($mode eq 'nobracket' ? "\e[?2004l" : "\e[?2004h");
        draw();
        logit("MODE $mode");
    }
    if ($late ne "" && time() >= $late_at) {
        take($late);
        $late = "";
        draw();
    }
    my $rin = "";
    vec($rin, fileno(STDIN), 1) = 1;
    next unless select(my $rout = $rin, undef, undef, 0.05);
    my $got = sysread(STDIN, my $chunk, 4096);
    last unless $got;
    $buf .= $chunk;
    while (length $buf) {
        if ($pasting) {
            my $end = index($buf, "\e[201~");
            if ($end < 0) { last }
            my $text = decode_utf8(substr($buf, 0, $end));
            $buf = substr($buf, $end + 6);
            $pasting = 0;
            if ($mode eq 'slow') {
                ($late, $late_at) = ($text, time() + 2);
            } else {
                take($mode eq 'mangle' ? "[Pasted text #1]" : $text);
            }
            logit("PASTE $text");
            ($fired, $mode) = (1, 'menu') if $mode eq 'menu-on-paste';
        } elsif (substr($buf, 0, 6) eq "\e[200~") {
            $buf = substr($buf, 6);
            $pasting = 1;
        } elsif (substr($buf, 0, 1) eq "\r") {
            $buf = substr($buf, 1);
            logit("ENTER");
            if ($composer ne "" && working($mode)) {
                logit("QUEUED $composer");
                record('{"type":"queue-operation","operation":"enqueue","content":' . json($composer) . '}')
                    if $mode eq 'working';
                push @queued, $composer;
                $composer = "";
            } elsif ($composer ne "") {
                logit("SUBMIT $composer");
                record('{"type":"user","message":{"role":"user","content":' . json($composer) . '}}');
                push @said, $composer;
                $composer = "";
            }
        } elsif (substr($buf, 0, 1) eq "\e") {
            # An escape that isn't a paste: drop it whole.
            $buf =~ s/^\e\[?[0-9;]*[A-Za-z~]?//;
        } else {
            $buf =~ s/^([^\e\r]+)//;
            take(decode_utf8($1));
            logit("TYPED " . decode_utf8($1));
        }
    }
    draw();
}
