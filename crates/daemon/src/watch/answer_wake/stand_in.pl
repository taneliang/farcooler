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
# shows: idle, working, menu (a permission prompt; working-menu, one the
# registry still calls busy; working-waiting, the registry's dialog not
# drawn yet), picker (a menu with no
# box), mangle (a paste shows as `[Pasted text #1]`), slow (a paste shows
# a second late), nobracket (bracketed paste off), draft:<text> (a box
# already holding <text>), or <mode>-on-paste (idle until a paste arrives,
# then <mode>: menu, working, working-hidden…, drawn in the same pass that
# logs PASTE, so a test never races it against the read-back).
#
# While working, Enter queues the box as the real agents do (ov-360): it
# logs `QUEUED <box>`, draws it above the box as claude 2.1.290 or codex
# 0.153.4 does, and submits it when the mode next turns idle. A claude
# stand-in with CLAUDE_CONFIG_DIR set keeps claude's session registry and
# transcript there, an `enqueue` record per queued message; working-quiet
# works the same and records nothing.
#
# A lone Esc logs `ESC`; working, it stops the turn as claude 2.1.290 does
# (ov-368): an `[Request interrupted by user]` record, the mode back to idle
# (the control file rewritten), and anything queued submitted at once. A
# ctrl+x ctrl+s logs `SENDNOW`; working, it sends every queued message as
# the next turn, a `dequeue` record for each, and stays working; idle, it
# submits a draft in the box, as claude does.
#
# A claude stand-in draws a paste as claude 2.1.290 does (ov-367): an image's
# path alone as `[Image #N]`, a paste past 800 UTF-16 units or with three
# line breaks as `[Pasted text #N +K lines]`, sent whole; a box starting
# with `/` opens a command popup above it, its best match highlighted (an
# alias too: `/cost` highlights `/usage`), and Enter on it runs that and logs
# `COMMAND /usage`. A submitted line break is logged as `\n`.
use strict;
use warnings;
use utf8;
use Encode qw(decode_utf8);

my ($agent, $control, $log) = @ARGV;
# STAND_IN_HOLD names a file to hold open, as codex holds its rollout.
my $held;
open($held, '<', $ENV{STAND_IN_HOLD}) or die "can't hold $ENV{STAND_IN_HOLD}" if $ENV{STAND_IN_HOLD};
binmode STDOUT, ':utf8';
$| = 1;
system("stty raw -echo 2>/dev/null");
print "\e[?2004h";

my ($composer, $mode, $pasting, $late, $late_at, @said, @queued) = ("", "", 0, "", 0);
my ($pasted, %whole) = (0);
my @commands = (["/init", "", "Initialize a new CLAUDE.md file"], ["/usage", "cost", "Show session cost"],
    ["/model", "", "Set the AI model"], ["/compact", "", "Free up context"]);
my ($transcript, $registry, $registry_cwd);
# This process's start in UTC, as claude writes `procStart`.
my $started = `TZ=UTC ps -o lstart= -p $$`;
$started =~ s/^\s+|\s+$//g;
if ($agent eq 'claude' && ($ENV{CLAUDE_CONFIG_DIR} // '') ne '') {
    use Cwd qw(getcwd);
    my ($config, $cwd) = ($ENV{CLAUDE_CONFIG_DIR}, getcwd());
    (my $project = $cwd) =~ s/[^A-Za-z0-9]/-/g;
    mkdir "$config/sessions";
    mkdir "$config/projects";
    mkdir "$config/projects/$project";
    $transcript = "$config/projects/$project/stand-in.jsonl";
    $registry = "$config/sessions/$$.json";
    $registry_cwd = $cwd;
    registry("idle");
}

# claude's session registry, with its status: `busy` while a turn runs.
sub registry {
    return unless defined $registry;
    open(my $f, '>:utf8', $registry) or die;
    print $f '{"pid":' . $$ . ',"sessionId":"stand-in","cwd":' . json($registry_cwd) . ',"procStart":' . json($started)
        . ',"status":"' . $_[0] . '"}';
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

# What the registry says in `mode`: busy while working, but idle for
# working-lagging (the screen works, the registry still says idle);
# `waiting` under a dialog, as claude 2.1.290 writes it (ov-368), but busy
# for working-menu (a dialog drawn before the registry says so).
sub status_of {
    my ($m) = @_;
    return "waiting" if $m eq 'working-waiting';
    return "busy" if $m eq 'working-menu' || (working($m) && $m ne 'working-lagging');
    return $m eq 'menu' ? "waiting" : "idle";
}

# working-lagging: the screen works, the registry still says idle.
# working-waiting: the screen works, the registry says a dialog is up.
sub working { return $_[0] =~ /^working(-quiet|-hidden|-long|-lagging|-waiting)?$/ }

# The command claude's popup highlights for the box, or undef with no popup.
sub highlighted {
    return undef unless $agent eq 'claude' && $composer =~ m{^/([^\s]*)$};
    my $typed = $1;
    for my $c (@commands) {
        return $c->[0] if substr($c->[0], 1, length $typed) eq $typed;
    }
    for my $c (@commands) {
        return $c->[0] if $c->[1] ne '' && substr($c->[1], 0, length $typed) eq $typed;
    }
    return undef;
}

# The box with each placeholder's whole paste back in it, as claude sends it.
sub whole {
    my ($text) = @_;
    $text =~ s/(\[Pasted text #\d+(?: \+\d+ lines)?\])/exists $whole{$1} ? $whole{$1} : $1/ge;
    return $text;
}

sub logged { my ($t) = @_; $t =~ s/\n/\\n/g; return $t }

sub units { my ($t) = @_; my $n = length $t; $n++ while $t =~ /[^\x{0}-\x{ffff}]/g; return $n }

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
    my @box = map { wrap($_, 60) } split(/\n/, $composer, -1);
    @box = ("") unless @box;
    if ($agent eq 'claude') {
        push @rows, " Claude Code stand-in", "";
        push @rows, map { "⏺ $_" } @said;
        push @rows, "";
        if ($mode eq 'menu' || $mode eq 'working-menu') {
            push @rows, " Do you want to create haiku.txt?", " ❯ 1. Yes", "   2. No", "",
                " Esc to cancel · Tab to amend";
        } elsif ($mode eq 'picker') {
            push @rows, " Select model", " ❯ Default", "   Opus", "", "  ? for shortcuts";
        } else {
            my $rule = "─" x 70;
            push @rows, map { my $q = $_; $q =~ s/\n/ /g; ("❯ $q", "  ctrl+x ctrl+s to send now") } @queued;
            push @rows, "✻ Pondering… (3s)" if working($mode) && $mode ne 'working-hidden';
            if (defined(my $hl = highlighted())) {
                for my $c (@commands) {
                    my $name = $c->[1] ne '' ? "$c->[0] ($c->[1])" : $c->[0];
                    push @rows, ($c->[0] eq $hl ? "  ❯ " : "    ") . sprintf("%-28s%s", $name, $c->[2]);
                }
            }
            my $first = shift(@box);
            $first = "\e[7mP\e[0;2mress up to edit queued messages\e[0m" if $composer eq "" && @queued;
            push @rows, $rule, "❯\x{a0}" . $first, (map { "  $_" } @box), $rule;
            # claude drops `esc to interrupt` while its box holds something.
            # After a long paste claude 2.1.290 keeps `paste again to expand`
            # there, working or not: `working-long`, with its spinner row
            # above the box, as claude draws it; `working-hidden`, with none.
            push @rows, $mode =~ /^working-(hidden|long)$/ ? "  paste again to expand"
                : working($mode) && $composer eq "" ? "  ⏸ manual mode on · esc to interrupt"
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

# Each queued message as the next turn's prompt, as claude runs its queue.
sub run_queue {
    for my $q (@queued) {
        logit("SUBMIT " . logged($q));
        record('{"type":"user","message":{"role":"user","content":' . json($q) . '},"promptSource":"queued"}')
            if $mode ne 'working-quiet';
        (my $shown = $q) =~ s/\n/ /g;
        push @said, $shown;
    }
    @queued = ();
}

sub take {
    my ($text) = @_;
    $composer .= $text;
}

draw();
my $buf = "";
my $fired = "";   # an X-on-paste's paste has arrived: X stays up
while (1) {
    my $now = mode();
    $fired = "" if $now !~ /-on-paste$/;
    $now = $fired if $fired ne "";
    if ($now ne $mode) {
        run_queue() if working($mode) && !working($now);
        $mode = $now;
        # Not when a test took the registry away.
        registry(status_of($mode)) if defined $registry && -e $registry;
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
            } elsif ($agent eq 'claude' && $mode ne 'mangle' && $text =~ m{^'?(/.*\.(?:png|jpe?g|gif|webp))'?$} && -f $1) {
                $pasted++;
                take("[Image #$pasted]");
            } elsif ($agent eq 'claude' && $mode ne 'mangle' && ((() = $text =~ /\n/g) >= 3 || units($text) > 800)) {
                $pasted++;
                my $breaks = () = $text =~ /\n/g;
                my $shown = $breaks ? "[Pasted text #$pasted +$breaks lines]" : "[Pasted text #$pasted]";
                $whole{$shown} = $text;
                take($shown);
            } else {
                take($mode eq 'mangle' ? "[Pasted text #1]" : $text);
            }
            logit("PASTE " . logged($text));
            if ($mode =~ /^(.+)-on-paste$/) {
                ($fired, $mode) = ($1, $1);
                registry(status_of($mode)) if defined $registry && -e $registry;
            }
        } elsif (substr($buf, 0, 6) eq "\e[200~") {
            $buf = substr($buf, 6);
            $pasting = 1;
        } elsif (substr($buf, 0, 1) eq "\r") {
            $buf = substr($buf, 1);
            logit("ENTER");
            my $sent = whole($composer);
            if (defined(my $hl = highlighted())) {
                logit("COMMAND $hl");
                $composer = "";
            } elsif ($composer ne "" && working($mode)) {
                logit("QUEUED " . logged($sent));
                record('{"type":"queue-operation","operation":"enqueue","content":' . json($sent) . '}')
                    if $mode ne 'working-quiet';
                push @queued, $sent;
                $composer = "";
            } elsif ($composer ne "") {
                logit("SUBMIT " . logged($sent));
                record('{"type":"user","message":{"role":"user","content":' . json($sent) . '}}');
                (my $shown = $composer) =~ s/\n/ /g;
                push @said, $shown;
                $composer = "";
            }
        } elsif ($buf eq "\e") {
            $buf = "";
            logit("ESC");
            if (working($mode)) {
                record('{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}');
                open(my $f, '>', $control) or die;
                print $f "idle";
                close $f;
            }
        } elsif (substr($buf, 0, 2) eq "\x18\x13") {
            $buf = substr($buf, 2);
            logit("SENDNOW");
            if (working($mode) && @queued) {
                record('{"type":"queue-operation","operation":"dequeue"}') for @queued;
                run_queue();
            } elsif (!working($mode) && $composer ne "") {
                logit("SUBMIT " . logged($composer));
                $composer = "";
            }
        } elsif (substr($buf, 0, 1) eq "\e") {
            # An escape that isn't a paste: drop it whole, a mouse report
            # (`ESC [ < 0 ; 41 ; 13 M`) or a reply (`ESC [ ? 6 c`) too.
            $buf =~ s/^\e\[?[<?>]?[0-9;]*\$?[A-Za-z~]?//;
        } else {
            $buf =~ s/^([^\e\r]+)//;
            take(decode_utf8($1));
            logit("TYPED " . decode_utf8($1));
        }
    }
    draw();
}
