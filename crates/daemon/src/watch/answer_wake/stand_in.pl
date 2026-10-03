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
# box), mangle (a paste shows as `[Pasted text #1]`), or draft:<text> (a box
# already holding <text>).
use strict;
use warnings;
use utf8;
use Encode qw(decode_utf8);

my ($agent, $control, $log) = @ARGV;
binmode STDOUT, ':utf8';
$| = 1;
system("stty raw -echo 2>/dev/null");
print "\e[?2004h";

my ($composer, $mode, $pasting, @said) = ("", "", 0);

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
            push @rows, $rule, "❯\x{a0}" . shift(@box), (map { "  $_" } @box), $rule;
            push @rows, $mode eq 'working' ? "  ⏸ manual mode on · esc to interrupt" : "  ⏸ manual mode on · ? for shortcuts";
        }
    } else {
        push @rows, " OpenAI Codex stand-in", "";
        push @rows, map { "• $_" } @said;
        push @rows, "";
        push @rows, "• Working (3s • esc to interrupt)" if $mode eq 'working';
        if ($composer eq "") {
            push @rows, "› \e[2mExplain this codebase\e[0m";
        } else {
            push @rows, "› " . shift(@box), map { "  $_" } @box;
        }
        push @rows, "", "  gpt-5 high · ~/src";
    }
    print "\e[H\e[2J" . join("\r\n", @rows);
}

sub take {
    my ($text) = @_;
    $composer .= $text;
}

draw();
my $buf = "";
while (1) {
    my $now = mode();
    if ($now ne $mode) {
        $mode = $now;
        $composer = $1 if $mode =~ /^draft:(.*)$/s;
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
            take($mode eq 'mangle' ? "[Pasted text #1]" : $text);
            logit("PASTE $text");
        } elsif (substr($buf, 0, 6) eq "\e[200~") {
            $buf = substr($buf, 6);
            $pasting = 1;
        } elsif (substr($buf, 0, 1) eq "\r") {
            $buf = substr($buf, 1);
            logit("ENTER");
            if ($composer ne "") {
                logit("SUBMIT $composer");
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
