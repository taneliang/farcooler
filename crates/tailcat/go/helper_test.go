package main

import (
	"strconv"
	"strings"
	"syscall"
	"testing"
)

// run drives the helper over a scripted stdin and returns the reply lines.
func run(t *testing.T, keyPath string, commands ...string) []string {
	t.Helper()
	in := strings.NewReader(strings.Join(commands, "\n") + "\n")
	var out strings.Builder
	if err := runHelper(keyPath, in, &out); err != nil {
		t.Fatalf("runHelper: %v", err)
	}
	lines := strings.Split(strings.TrimRight(out.String(), "\n"), "\n")
	if len(lines) != len(commands) {
		t.Fatalf("wanted one reply per command, got %d for %d: %q", len(lines), len(commands), lines)
	}
	return lines
}

// errLine is the reply the helper sends for one errno. The Rust side turns it
// straight back into `io::Error::from_raw_os_error`, so the number is the
// contract and not a detail.
func errLine(e syscall.Errno) string {
	return "err " + strconv.Itoa(int(e))
}

// The whole reason this program is spoken to over a pipe rather than argv: a
// command it does not know must be refused, not guessed at.
func TestAnUnknownCommandIsRefused(t *testing.T) {
	got := run(t, "/nonexistent/key", "nonsense", "", "serve")
	for i, line := range got {
		if line != errLine(syscall.EINVAL) {
			t.Errorf("reply %d: got %q, want %q", i, line, errLine(syscall.EINVAL))
		}
	}
}

// The guard this design is most likely to lose, at the last place it can be
// lost. Tailcat reads an empty AllowedClients as "admit everyone", so a
// `serve` carrying no node keys must never reach serve() at all — and serve()
// refuses it a second time regardless.
func TestServingNobodyIsRefusedBeforeAServerExists(t *testing.T) {
	got := run(t, "/nonexistent/key", "serve 22")
	if got[0] != errLine(syscall.EINVAL) {
		t.Fatalf("an allowlist admitting nobody was accepted: %q", got[0])
	}
	mu.Lock()
	defer mu.Unlock()
	if server != nil {
		t.Fatal("a server was started for an allowlist that admits nobody")
	}
}

// A port that is not a port is the caller's mistake, and it must not be
// silently truncated into one that is.
func TestAnUnusablePortIsRefused(t *testing.T) {
	for _, port := range []string{"-1", "65536", "twenty-two", ""} {
		got := run(t, "/nonexistent/key", "serve "+port+" "+keyA)
		if got[0] != errLine(syscall.EINVAL) {
			t.Errorf("port %q: got %q, want a refusal", port, got[0])
		}
	}
}

// `blob` and `allow` before `serve` have nothing to answer about. ENOTCONN is
// the same errno the C entry points return for the same state, so the Rust
// side sees one answer from both backends.
func TestAskingAboutATunnelThatIsNotServingSaysSo(t *testing.T) {
	got := run(t, "/nonexistent/key", "blob", "allow "+keyA)
	for i, line := range got {
		if line != errLine(syscall.ENOTCONN) {
			t.Errorf("reply %d: got %q, want %q", i, line, errLine(syscall.ENOTCONN))
		}
	}
}

// Configuration is recorded rather than refused, exactly as
// `fc_tailcat_set_derp_map_url` records it — and it is the one command that
// claims nothing about a tunnel.
func TestTheDerpMapIsASetting(t *testing.T) {
	t.Cleanup(func() { setDerpMapURL("") })
	got := run(t, "/nonexistent/key", "derpmap https://example.invalid/derpmap.json", "derpmap")
	if got[0] != "ok" {
		t.Errorf("setting the DERP map URL was refused: %q", got[0])
	}
	if got[1] != errLine(syscall.EINVAL) {
		t.Errorf("a DERP map command with no URL was accepted: %q", got[1])
	}
	mu.Lock()
	defer mu.Unlock()
	if derpMapURL != "https://example.invalid/derpmap.json" {
		t.Errorf("the DERP map URL was not recorded: %q", derpMapURL)
	}
}

// EOF on stdin is how the daemon says it is gone, and this program's whole
// supervision story: no PID to track, no socket to clean up. It must exit
// cleanly rather than reporting a failure nobody caused.
func TestClosingTheParentsPipeIsNotAnError(t *testing.T) {
	var out strings.Builder
	if err := runHelper("/nonexistent/key", strings.NewReader(""), &out); err != nil {
		t.Fatalf("EOF on stdin reported an error: %v", err)
	}
	if out.String() != "" {
		t.Errorf("something was written for no command: %q", out.String())
	}
}

// A final line with no newline is still a command. A parent that exits right
// after writing one would otherwise have it silently dropped.
func TestAnUnterminatedLastCommandIsStillRun(t *testing.T) {
	var out strings.Builder
	if err := runHelper("/nonexistent/key", strings.NewReader("blob"), &out); err != nil {
		t.Fatalf("runHelper: %v", err)
	}
	if strings.TrimRight(out.String(), "\n") != errLine(syscall.ENOTCONN) {
		t.Errorf("an unterminated command was dropped: %q", out.String())
	}
}
