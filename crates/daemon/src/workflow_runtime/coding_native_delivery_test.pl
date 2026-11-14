# Scratch Claude: runs the actual commands from the pinned settings supplied to the native launch.
# No direct socket writes: the real CLI must advertise context support and print Claude's JSON.
use strict;
use warnings;
use JSON::PP;
use IPC::Open2;
use Cwd qw(getcwd);
use Time::HiRes qw(sleep);

open my $settings_file, '<', $ARGV[0] or die $!;
my $settings = decode_json(do { local $/; <$settings_file> });
close $settings_file;
my $session = $ENV{FARCOOLER_NATIVE_SESSION};
my $cwd = getcwd();
sub hook {
    my ($event, %extra) = @_;
    my $command = $settings->{hooks}{$event}[0]{hooks}[0]{command};
    my $pid = open2(my $out, my $in, '/bin/sh', '-c', $command);
    print $in encode_json({ session_id => $session, cwd => getcwd(), %extra });
    close $in;
    my $reply = do { local $/; <$out> };
    close $out;
    waitpid($pid, 0);
    die "hook failed: $?" if $?;
    if (length $reply) {
        my $value = decode_json($reply);
        die "context escaped validation" if $extra{agent_id} || $extra{session_id} || $extra{cwd};
        open my $log, '>>', $ENV{CONTEXT_LOG} or die $!;
        print $log encode_json($value) . "\n";
        close $log;
    }
}
die 'native agent needs a PTY' unless -t STDIN && -t STDOUT;
hook('UserPromptSubmit', prompt => $ENV{TEST_LAUNCHED_PROMPT});
open my $started, '>', $ENV{STARTED} or die $!;
print $started $$;
close $started;
# Hooks may report a subdirectory of the worktree, as a real builder does.
mkdir 'subdir' or die $!;
chdir 'subdir' or die $!;
while (!-e $ENV{FINISH}) {
    hook('PreToolUse', session_id => 'foreign');
    hook('PreToolUse', cwd => '/foreign');
    hook('PreToolUse', agent_id => 'child');
    hook('PreToolUse', tool_name => 'Bash', tool_input => {command => 'true'});
    sleep 0.05;
}
chdir $cwd or die $!;
rmdir 'subdir' or die $!;
open my $result, '>', 'result.txt' or die $!;
print $result 'candidate native PTY';
close $result;
system('/usr/bin/git', 'add', '--', 'result.txt') == 0 or die 'git add';
system('/usr/bin/git', '-c', 'commit.gpgsign=false', 'commit', '--quiet', '-m', 'native delivery') == 0 or die 'git commit';
hook('Stop');
sleep 30; # The native host owns teardown after the authenticated Stop.
