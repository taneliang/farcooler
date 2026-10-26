// Command webtags prints the -tags a browser build of the tunnel takes:
// upstream Tailcat's own js/wasm set, which keeps netstack and what it needs
// and omits every other optional tailscale feature (ov-764).
//
// Computed from the tailscale module this go.mod pins rather than written
// down, so a tailscale bump that adds a feature cannot quietly grow the
// browser's download: `crates/web/build.sh` runs it on every build.
// Upstream computes the set the same way (tailcat's internal/buildtags).
package main

import (
	"fmt"
	"slices"
	"strings"

	"tailscale.com/feature/featuretags"
)

func main() {
	keep := map[featuretags.FeatureTag]bool{}
	for dep := range featuretags.Requires("netstack") {
		keep[dep] = true
	}
	tags := []string{"osusergo", "netgo", "omitidna", "omitpemdecrypt"}
	var omitted []string
	for ft := range featuretags.Features {
		if ft == "" || !ft.IsOmittable() || keep[ft] {
			continue
		}
		omitted = append(omitted, ft.OmitTag())
	}
	// Sorted, so two builds of one tree pass the same flag and a build log
	// can be compared with another.
	slices.Sort(omitted)
	fmt.Println(strings.Join(append(tags, omitted...), ","))
}
