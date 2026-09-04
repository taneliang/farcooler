//go:build !cgo

// The entry point for the standalone helper. See `helper.go` for what it is
// and why Linux runs one.
//
// `//go:build !cgo` rather than a second package: everything this program does
// is already in `tailcat.go`, and a copy of the DERP pinning, identity and
// allowlist logic would be a second place for the two to disagree about who a
// runner admits. The c-archive build gets `exports_cgo.go`'s empty `main`
// instead, so exactly one of the two is ever compiled and neither platform
// carries the other's entry point.
package main

import (
	"flag"
	"fmt"
	"os"
)

func main() {
	keyPath := flag.String("key", "", "path to this runner's tailcat identity")
	flag.Parse()
	if *keyPath == "" {
		fmt.Fprintln(os.Stderr, "farcooler-tunnel: --key is required")
		os.Exit(2)
	}
	// Commands in, replies out, logs to stderr — which is the daemon's own
	// stderr, inherited, so everything tailcat has to say lands in the same
	// log as everything the daemon has to say. Nothing here reaches a screen.
	if err := runHelper(*keyPath, os.Stdin, os.Stdout); err != nil {
		fmt.Fprintf(os.Stderr, "farcooler-tunnel: %v\n", err)
		os.Exit(1)
	}
}
