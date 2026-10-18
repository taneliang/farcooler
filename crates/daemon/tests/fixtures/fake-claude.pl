#!/usr/bin/env perl
# Stand-in for `claude auth login|status|logout` (Claude Code 2.1.301), replaying the
# transcript measured in ov-584 spike 0. Login with a code matching TEST_CODE succeeds.
# Unmeasured (needs a real second account): the real success text and the signed-in
# status JSON; the shapes here are placeholders and are marked as such.
use strict; use warnings; use JSON::PP;
$| = 1;
my $TEST_CODE = qr/^ok-[A-Za-z0-9_-]+#[A-Za-z0-9_-]+$/;
my $dir = $ENV{CLAUDE_CONFIG_DIR} or die "fake-claude: CLAUDE_CONFIG_DIR must be set\n";
my $cfg = "$dir/.claude.json";
my $json = JSON::PP->new->canonical->pretty;

sub load { open(my $f, '<', $cfg) or return {}; local $/; my $t = <$f>; close $f; return $json->decode($t || '{}'); }
sub save { mkdir $dir; open(my $f, '>', "$cfg.tmp") or die "$cfg.tmp: $!"; print $f $json->encode($_[0]); close $f; rename("$cfg.tmp", $cfg) or die "rename: $!"; }

my ($cmd, $sub, @rest) = @ARGV;
$cmd //= ''; $sub //= '';
if ($cmd eq '--version') { print "2.1.301 (Claude Code)\n"; exit 0; }
die "fake-claude: only `auth login|status|logout` are faked\n" unless $cmd eq 'auth';

if ($sub eq 'status') {
    my $c = load();
    my $in = $c->{oauthAccount} ? JSON::PP::true : JSON::PP::false;
    if (grep { $_ eq '--text' } @rest) {
        if ($in) { print "Logged in as $c->{oauthAccount}{emailAddress}\n"; exit 0 }   # placeholder, unmeasured
        print "Not logged in. Run claude auth login to authenticate.\n"; exit 1;
    }
    my %s = (loggedIn => $in, authMethod => ($in ? 'claude.ai' : 'none'),        # signed-in shape is a placeholder
             apiProvider => 'firstParty', analyticsDisabled => JSON::PP::false,
             projectsDirectory => "$dir/projects", configDirectory => $dir);
    $s{email} = $c->{oauthAccount}{emailAddress} if $in;                          # placeholder
    my $e = JSON::PP->new->allow_nonref;   # measured key order, two-space indent
    my @k = (qw(loggedIn authMethod apiProvider analyticsDisabled projectsDirectory configDirectory), ($in ? "email" : ()));
    print "{\n", join(",\n", map { "  \"$_\": " . $e->encode($s{$_}) } @k), "\n}\n";
    exit($in ? 0 : 1);
}

if ($sub eq 'logout') {
    my $c = load(); delete $c->{oauthAccount}; save($c);
    print "Successfully logged out from your Anthropic account.\n";   # measured: exit 0 even when not signed in
    exit 0;
}

die "fake-claude: unknown auth subcommand `$sub`\n" unless $sub eq 'login';

my $console = grep { $_ eq '--console' } @rest;
my $host = $console ? 'https://platform.claude.com/oauth/authorize' : 'https://claude.com/cai/oauth/authorize';
my $q = 'code=true&client_id=9d1c250a-e61b-44d9-88ed-5944d1962f5e&response_type=code&redirect_uri=%s'
      . '&scope=org%%3Acreate_api_key+user%%3Aprofile+user%%3Ainference+user%%3Asessions%%3Aclaude_code+user%%3Amcp_servers+user%%3Afile_upload+user%%3Aplugins'
      . '&code_challenge=FAKEchallengeFAKEchallengeFAKEchallenge00000&code_challenge_method=S256&state=FAKEstateFAKEstateFAKEstateFAKEstate0000000';
my $manual = "$host?" . sprintf($q, 'https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback');
my $local  = "$host?" . sprintf($q, 'http%3A%2F%2Flocalhost%3A57533%2Fcallback');

# Measured: stdout carries everything below; the prompt has no trailing newline.
print "Opening browser to sign in\xe2\x80\xa6\n";
# Measured: the browser gets the localhost-callback URL via $BROWSER; a failing $BROWSER is ignored.
if (defined $ENV{BROWSER} && length $ENV{BROWSER}) { system($ENV{BROWSER}, $local); }
print "If the browser didn't open, visit: $manual\n";
print "Paste code here if prompted > ";

while (defined(my $line = <STDIN>)) {
    $line =~ s/[\r\n]+$//;
    if ($line !~ /#/) {            # measured: local check, stderr, keeps waiting, no new prompt
        print STDERR "Invalid code. Please make sure the full code was copied.\n";
        next;
    }
    if ($line =~ $TEST_CODE) {
        my $c = load();
        $c->{oauthAccount} = { emailAddress => 'member@example.com' };
        save($c);
        print "Login successful.\n";   # placeholder, unmeasured
        exit 0;
    }
    print STDERR "Login failed: Request failed with status code 400\n";   # measured, exit 1
    exit 1;
}
# Measured: on stdin EOF the real CLI does not exit; it keeps waiting for the browser
# callback until killed. Mirror that so callers must time out and kill.
sleep 1 while 1;
