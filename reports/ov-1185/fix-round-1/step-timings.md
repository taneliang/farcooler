| Job | Command | Historical seconds | Earlier green | Earlier diagnostic | Blocked probe |
| --- | --- | ---: | ---: | ---: | ---: |
| rust | if ! gh api "repos/$GITHUB_REPOSITORY/actions/workflows/ci.yml/runs?branch=main&event=push&status=success&per_page=100" \ ... | not in capture | 0.0 (skipped: CI lists verified green bases; local shadow safely proposes full without that evidence) | 0.0 (skipped: CI lists verified green bases; local shadow safely proposes full without that evidence) | not reached |
| rust | ./scripts/ci-install-tmux.sh | None | 0.0 (skipped: installs a toolchain on a CI runner) | 0.0 (skipped: installs a toolchain on a CI runner) | not reached |
| rust | if command -v git-lfs >/dev/null 2>&1; then ... | None | 0.0 (skipped: installs a toolchain on a CI runner) | 0.0 (skipped: installs a toolchain on a CI runner) | not reached |
| rust | if ! command -v fish >/dev/null 2>&1; then ... | None | 0.0 (skipped: installs a toolchain on a CI runner) | 0.0 (skipped: installs a toolchain on a CI runner) | not reached |
| rust | if [ ! -x /usr/sbin/sshd ]; then ... | None | 0.0 (skipped: installs a toolchain on a CI runner) | 0.0 (skipped: installs a toolchain on a CI runner) | not reached |
| rust | cargo check -p farcooler-tailcat --features linked | 0.0 | 0.11114695901051164 (pass) | 0.10279770797933452 (pass) | not reached |
| rust-lints | cargo clippy --workspace --all-targets -- -D warnings | 37.0 | 69.42637033297797 (pass) | 47.12029479199555 (pass) | not reached |
| swift-tests | xcodebuild -version ... | 0.0 | 0.35526549999485724 (pass) | 0.24624791598762386 (pass) | not reached |
| android | echo "y" \| "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" \ ... | None | 0.0 (skipped: installs a toolchain on a CI runner) | 0.0 (skipped: installs a toolchain on a CI runner) | not reached |
| android | ./scripts/build-android-libs.sh | 54.0 | 137.44729645800544 (pass) | 102.74768524998217 (pass) | not reached |
| rust | cargo check -p farcooler-tailcat --features linked --target aarch64-unknown-linux-musl | 0.0 | 0.26079004199709743 (pass) | 0.17056554200826213 (pass) | not reached |
| swift-tests | ./scripts/macos-test-wrapper-test.sh | 8.0 | 6.695165916986298 (pass) | 5.973162250011228 (pass) | not reached |
| rust | cargo nextest run --profile ci -p farcooler-tailcat --features helper | 6.0 | 76.90286570900935 (pass) | 52.0079217920138 (pass) | not reached |
| swift-tests | ./scripts/wait-for-unlock-test.sh | 4.0 | 4.202833249990363 (pass) | 4.140677000017604 (pass) | not reached |
| swift-tests | ./scripts/swift-test-guard-test.sh | 32.0 | 45.35509420800372 (pass) | 44.25663379099569 (pass) | not reached |
| swift-tests | ./scripts/macos-test-partitions.py --self-test | not in capture | 15.5762867500016 (pass) | 13.798447707988089 (pass) | not reached |
| rust-lints | cargo clippy -p farcooler-tailcat --features helper --all-targets -- -D warnings | 4.0 | 11.126575000002049 (pass) | 7.079639500007033 (pass) | not reached |
| swift-tests | ./scripts/test-leak-check.py --self-test | 8.0 | 7.835047707980266 (pass) | 7.895826791995205 (pass) | not reached |
| rust | cargo nextest run --profile ci -p farcooler-daemon --features tailcat-helper --test an_empty_allowlist_starts_no_tunnel | 24.0 | 53.76149804200395 (pass) | 42.78069995800615 (pass) | not reached |
| swift-tests | ./scripts/mac-capture-stop-test.sh | 1.0 | 0.9399047500046436 (pass) | 0.9198141660017427 (pass) | not reached |
| rust-lints | cargo clippy -p farcooler-tailcat --features linked --all-targets -- -D warnings | 0.0 | 50.2959429170005 (pass) | 40.7134135410015 (pass) | not reached |
| swift-tests | ./scripts/capture-duplicates-test.sh | 2.0 | 2.2630781670159195 (pass) | 2.3288412499823608 (pass) | not reached |
| swift-tests | cargo build --release -p farcooler-vt -p farcooler-client | 150.0 | 165.55779941700166 (pass) | 106.93696899997303 (pass) | not reached |
| rust-lints | cargo clippy -p farcooler-daemon --features tailcat-helper --all-targets -- -D warnings | 26.0 | 70.89937025000108 (pass) | 36.54914462499437 (pass) | not reached |
| rust | cargo nextest run --profile ci -p farcooler-daemon --features tailcat-helper --test revocation_closes_what_it_revoked | 5.0 | 76.7835455420136 (pass) | 42.04739887500182 (pass) | not reached |
| android | cargo build -p farcooler-jni | 14.0 | 91.13818062501377 (pass) | 41.46331804100191 (pass) | not reached |
| rust-lints | cargo clippy -p farcooler-daemon --features tailcat --all-targets -- -D warnings | 26.0 | 57.308474999998 (pass) | 44.965035083994735 (pass) | not reached |
| rust | cargo nextest run --profile ci -p farcooler-daemon --features tailcat-helper --test a_pairing_gives_the_runner_a_tunnel | 1.0 | 52.57952433399623 (pass) | 40.5609843749844 (pass) | not reached |
| android | ./gradlew testInstrumentedUnitTest "-Pfarcooler.nativeLibDir=$GITHUB_WORKSPACE/target/debug" | 30.0 | 1.5159500830050092 (pass) | 1.50419658300234 (pass) | not reached |
| android | ./gradlew assembleDebug assembleAndroidTest | 36.0 | 1.3957918749947567 (pass) | 1.2770297499955632 (pass) | not reached |
| android | ./gradlew assembleRelease | 106.0 | 1.1200834170158487 (pass) | 1.024708458018722 (pass) | not reached |
| android | ./gradlew testInstrumentedUnitTest \ ... | 324.0 | 469.2801519999921 (pass) | 733.2555680410005 (pass) | not reached |
| swift-tests | ./scripts/swift-test-guard.sh apps/shared/AgentKit/test-floor.txt -- ./scripts/test-leak-check.py -- swift test --package-path apps/shared/AgentKit | 88.0 | 24.12883062500623 (pass) | 25.82823629199993 (pass) | not reached |
| rust-lints | cargo clippy -p farcooler-client --features tailcat --all-targets -- -D warnings | 19.0 | 23.113573166978313 (pass) | 22.23555154100177 (pass) | not reached |
| rust | cargo nextest run --profile ci -p farcooler-daemon --features tailcat-helper --test a_sweep_rebuilds_the_tunnel_once | 2.0 | 23.92906291698455 (pass) | 23.533765583008062 (pass) | not reached |
| swift-tests | ./scripts/ci-install-tmux.sh | None | 0.0 (skipped: installs a toolchain on a CI runner) | 0.0 (skipped: installs a toolchain on a CI runner) | not reached |
| swift-tests | ./apps/macos/test.sh | 429.0 | 500.52773304100265 (pass) | 668.732786040986 (pass) | not reached |
| rust-lints | cargo clippy -p farcooler-cli --features tailcat --all-targets -- -D warnings | 9.0 | 29.308974292012863 (pass) | 25.69206133298576 (pass) | not reached |
| rust | cargo nextest run --profile ci -p farcooler-daemon --features tailcat-helper --test a_configured_derp_map_reaches_the_tunnel | 1.0 | 28.659757917019306 (pass) | 24.515449875005288 (pass) | not reached |
| rust-lints | cargo clippy -p farcooler-jni --features tailcat --all-targets -- -D warnings | 4.0 | 6.792107541987207 (pass) | 5.944181957980618 (pass) | not reached |
| rust | cargo check -p farcooler-daemon --features tailcat --all-targets | 27.0 | 52.27789262501756 (pass) | 112.79012316599255 (pass) | not reached |
| wire | ./scripts/proto-lint.py --self-test | 1.0 | 1.1638124999881256 (pass) | 0.8142004999972414 (pass) | not reached |
| wire | ./scripts/proto-lint.py --channel preview | 0.0 | 0.09834479101118632 (pass) | 0.07004591598524712 (pass) | not reached |
| wire | ./scripts/proto-lint.py --channel stable | 0.0 | 0.11331683400203474 (pass) | 0.08907941600773484 (pass) | not reached |
| wire | ./scripts/proto-lint.py --channel canary | 0.0 | 0.11240720798377879 (pass) | 0.09367041700170375 (pass) | not reached |
| wire | ./scripts/canary-baseline-test.sh | 10.0 | 7.924356916977558 (pass) | 10.039512917021057 (pass) | not reached |
| wire | ./scripts/canary-shipped.py --self-test | 0.0 | 0.09221945801982656 (pass) | 0.12113666700315662 (pass) | not reached |
| wire | ./scripts/canary-plan.py --self-test | 2.0 | 2.0070525830087718 (pass) | 2.4259460419998504 (pass) | not reached |
| wire | ./scripts/device-keys-readiness.py --self-test | 0.0 | 0.0711844579782337 (pass) | 0.09523104201070964 (pass) | not reached |
| wire | ./scripts/site-proof.py --self-test | 0.0 | 0.29155004100175574 (pass) | 0.4050010830105748 (pass) | not reached |
| wire | ./scripts/ci-changes.py --self-test | 1.0 | 0.9301467089971993 (pass) | 1.267965875013033 (pass) | not reached |
| wire | python3 scripts/rust-shadow-test.py | not in capture | 2.8941408329992555 (pass) | 3.0047867909888737 (pass) | not reached |
| wire | HEAVY_LOCK_HELD=1 ./scripts/lane-gate.py --self-test | 8.0 | 10.689955916983308 (pass) | 10.681704874994466 (pass) | not reached |
| wire | python3 scripts/gate-evidence-test.py | not in capture | 57.834503916004905 (pass) | 80.24374466697918 (pass) | not reached |
| rust | cargo build --workspace --bins ... | 21.0 | 39.92592041697935 (pass) | 17.980973708996316 (pass) | not reached |
| wire | python3 scripts/engine-gate-test.py | not in capture | 56.290040958003374 (pass) | 61.2992965420126 (pass) | not reached |
| rust | ./scripts/tmux-leak-check.py --self-test | 22.0 | 24.847598583000945 (pass) | 25.456027249980252 (pass) | not reached |
| rust | ./scripts/tmux-stale-sockets.py --self-test | 0.0 | 0.40342554199742153 (pass) | 0.42176412499975413 (pass) | not reached |
| rust | ./scripts/test-leak-check.py --self-test | 8.0 | 7.863905917009106 (pass) | 7.891015250002965 (pass) | not reached |
| rust | ./scripts/test-leak-check.py -- ./scripts/tmux-leak-check.py -- cargo nextest run --profile ci --workspace --no-fail-fast | 665.0 | 1003.7268902499927 (pass) | 970.4236254999996 (FAIL (exit 100)) | not reached |
| wire | HEAVY_LOCK_HELD=1 ./scripts/heavy_lock.py --self-test | 17.0 | 16.94274700002279 (pass) | 16.906070291996002 (pass) | not reached |
| wire | ./scripts/notarytool-retry.sh --self-test | 1.0 | 1.096775207988685 (pass) | 1.17255474999547 (pass) | not reached |
| wire | ./scripts/version-test.sh | 5.0 | 4.769937334000133 (pass) | 5.023434000002453 (pass) | not reached |
| wire | ./scripts/workos-test.sh | 1.0 | 0.6987785419914871 (pass) | 0.6868541669973638 (pass) | not reached |
| wire | ./scripts/appcast-test.sh | 0.0 | 0.34881545798270963 (pass) | 0.36070495800231583 (pass) | not reached |
| wire | ./scripts/ci-flake-tally-test.sh | 1.0 | 1.0177154580014758 (pass) | 1.0984987079864368 (pass) | not reached |
| wire | ./scripts/ios-ui-retry-test.sh | 56.0 | 76.98862162500154 (pass) | 77.4599929160031 (pass) | not reached |
| wire | ./scripts/ios-ui-lease-test.sh | 63.0 | 65.12221679199138 (pass) | 65.10455491702305 (pass) | not reached |
| wire | ./scripts/copy-lint.py --self-test | 0.0 | 0.06608966601197608 (pass) | 0.09450320899486542 (pass) | not reached |
| wire | ./scripts/test-leak-check.py --lint | 1.0 | 0.8898039170017 (pass) | 1.1781598330126144 (pass) | not reached |
| wire | ./scripts/copy-lint.py | 1.0 | 1.312588250002591 (pass) | 1.4754357089987025 (pass) | not reached |
| wire | ./scripts/shared-core-lint.py --self-test | 0.0 | 0.09485550000681542 (pass) | 0.0752407910185866 (pass) | not reached |
| wire | ./scripts/shared-core-lint.py | 1.0 | 0.822961750003742 (pass) | 1.0993312910140958 (pass) | not reached |
| wire | git fetch --no-tags origin main ... | 3.0 | 2.754804708994925 (pass) | 2.7877199169888627 (pass) | not reached |
| wire | ./scripts/capability-names.py --self-test ... | 0.0 | 0.08876866698847152 (pass) | 0.10399691699421965 (pass) | not reached |
| wire | ./scripts/claude-prices.py --self-test | 0.0 | 0.1170319999800995 (pass) | 0.15011733301798813 (pass) | not reached |
| wire | ./scripts/openai-prices.py --self-test | 0.0 | 0.08991354101453908 (pass) | 0.10280833399156108 (pass) | not reached |
| wire | ./scripts/derp-hosts-check.py --self-test | 0.0 | 0.10060358297778293 (pass) | 0.11750875000143424 (pass) | not reached |
| wire | ./scripts/file-size-budget.py --self-test ... | 0.0 | 0.23623416700866073 (pass) | 0.3053876659832895 (pass) | not reached |
| wire | ./scripts/plan-layer-lint.py --self-test ... | 1.0 | 0.2956107080099173 (pass) | 0.3809092910087202 (pass) | not reached |
| wire | ./scripts/page-layer-lint.py --self-test ... | 16.0 | 4.692827166989446 (pass) | 4.615582541999174 (pass) | not reached |
| wire | ./scripts/swallow-lint.py --self-test | 0.0 | 0.10330112499650568 (pass) | 0.05332050001015887 (pass) | not reached |
| wire | ./scripts/swallow-lint.py | 0.0 | 0.293739959015511 (pass) | 0.2129327500006184 (pass) | not reached |
| wire | ./scripts/visual-tokens-lint.py --self-test | 0.0 | 0.10193845900357701 (pass) | 0.08571012498578057 (pass) | not reached |
| wire | ./scripts/visual-tokens-lint.py | 1.0 | 1.2734589169849642 (pass) | 1.3676051250076853 (pass) | not reached |
| wire | ./scripts/pixel-scale-lint.py --self-test | 0.0 | 0.08006541701615788 (pass) | 0.08859820899670012 (pass) | not reached |
| wire | ./scripts/pixel-scale-lint.py | 0.0 | 0.22636149998288602 (pass) | 0.23551633299211971 (pass) | not reached |
| wire | ./scripts/test-window-lint.py --self-test | 0.0 | 0.13685483299195766 (pass) | 0.07776400001603179 (pass) | not reached |
| wire | ./scripts/test-window-lint.py | 0.0 | 0.24805875000311062 (pass) | 0.18439141698763706 (pass) | not reached |
| wire | ./scripts/runloop-spin-lint.py --self-test | 0.0 | 0.08568141600699164 (pass) | 0.07053258299129084 (pass) | not reached |
| wire | ./scripts/runloop-spin-lint.py | 0.0 | 0.15430416699382477 (pass) | 0.14307537500280887 (pass) | not reached |
| wire | ./scripts/capture-window-lint.py --self-test | 0.0 | 0.07547549999435432 (pass) | 0.060007583000697196 (pass) | not reached |
| wire | ./scripts/capture-window-lint.py | 0.0 | 0.11536479200003669 (pass) | 0.11592487499001436 (pass) | not reached |
| wire | ./scripts/fresh-script-lint.py --self-test | 0.0 | 0.07146179201663472 (pass) | 0.10312854201765731 (pass) | not reached |
| wire | ./scripts/fresh-script-lint.py | 0.0 | 0.2249151249998249 (pass) | 0.5828380419989116 (pass) | not reached |
| wire | ./scripts/charter-intercept-lint.py --self-test | 0.0 | 0.06660466600442305 (pass) | 0.1136847079906147 (pass) | not reached |
| wire | ./scripts/charter-intercept-lint.py | 0.0 | 0.12264762498671189 (pass) | 0.15936679200967774 (pass) | not reached |
| wire | ./scripts/ios-agentkit-subset-lint.py --self-test | 0.0 | 0.14475995901739225 (pass) | 0.17394491701270454 (pass) | not reached |
| wire | ./scripts/ios-agentkit-subset-lint.py | 2.0 | 1.7577334589732345 (pass) | 2.197386000014376 (pass) | not reached |
| wire | ./scripts/migration-lint.py --self-test | 1.0 | 1.3745122500113212 (pass) | 2.024440874985885 (pass) | not reached |
| wire | ./scripts/migration-lint.py | 0.0 | 0.16182924999156967 (pass) | 0.2974375830090139 (pass) | not reached |
| wire | if [ "$(typeshare --version 2>/dev/null)" != "typeshare 1.13.4" ]; then ... | 0.0 | 0.0410220829944592 (pass) | 0.05402399998274632 (pass) | not reached |
| wire | ./scripts/gen-wire-types.sh ... | 0.0 | 0.18468362500425428 (pass) | 0.35627991598448716 (pass) | not reached |
| doc-comments | ./scripts/doc-comment-check.py --self-test | 0.0 | 0.07891458299127407 (pass) | 0.12059708399465308 (pass) | not reached |
| doc-comments | ./scripts/doc-comment-check.py --base 54aa2ec89382e729e592e8fe12320d52e35fb853 --head HEAD | not in capture | 0.09427408300689422 (pass) | 0.145709792006528 (pass) | not reached |
| relay | npm ci | 6.0 | 4.140448999998625 (pass) | 4.925801458011847 (pass) | not reached |
| relay | npx tsc --noEmit | 1.0 | 1.5107088749937247 (pass) | 1.452995291998377 (pass) | not reached |
| relay | npm test | 14.0 | 20.929859083000338 (pass) | 35.43901775000268 (pass) | not reached |
| relay | npx wrangler deploy --dry-run --outdir /tmp/relay-build | 1.0 | 0.9074386249994859 (pass) | 2.1501357090019155 (pass) | not reached |
| tunnel | unformatted="$(gofmt -l .)" ... | not in capture | 0.05170979199465364 (pass) | 0.07031324997660704 (pass) | not reached |
| tunnel | CGO_ENABLED=1 go vet ./... ... | not in capture | 13.221621584001696 (pass) | 14.845477374998154 (pass) | not reached |
| tunnel | CGO_ENABLED=0 go vet ./... ... | not in capture | 11.155886207998265 (pass) | 13.929391832993133 (pass) | not reached |
| web | mkdir -p wasm-tools ... | None | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | not reached |
| web | echo "WASM_BINDGEN=$RUNNER_TEMP/wasm-tools/wasm-bindgen-$WASM_BINDGEN_VERSION-x86_64-unknown-linux-musl/wasm-bindgen" >> "$GITHUB_ENV" ... | None | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | not reached |
| web | cargo check --locked -p farcooler-client --no-default-features --features web --target wasm32-unknown-unknown | 3.0 | 34.57035754201934 (pass) | 17.33373166702222 (pass) | not reached |
| android | python3 - <<'PY' ... | 0.0 | 0.09409137500915676 (pass) | 0.060492707998491824 (pass) | not reached |
| web | ./scripts/web-client-tree-check.py --self-test | 0.0 | 0.051265957998111844 (pass) | 0.11107466698740609 (pass) | not reached |
| web | ./scripts/web-client-tree-check.py | 0.0 | 0.1331442089867778 (pass) | 0.3737139579898212 (pass) | not reached |
| web | npm ci | 5.0 | 2.926649291999638 (pass) | 6.511336625000695 (pass) | not reached |
| web | npx tsc --noEmit -p tsconfig.json | 0.0 | 0.5057264579809271 (pass) | 1.3034157500078436 (pass) | not reached |
| web | npm test | 2.0 | 2.5951601669949014 (pass) | 6.070223791000899 (pass) | not reached |
| web | npm run test:workflow | 0.0 | 0.5633756669994909 (pass) | 0.7338717920065392 (pass) | not reached |
| web | if [ ! -x /usr/sbin/sshd ]; then ... | None | 0.0 (skipped: installs a toolchain on a CI runner) | 0.0 (skipped: installs a toolchain on a CI runner) | not reached |
| web | cargo test --locked --manifest-path ../../crates/web/Cargo.toml | 36.0 | 128.16345058300067 (pass) | 16.63532312499592 (pass) | not reached |
| swift | xcodebuild -version ... | 0.0 | 0.30051083301077597 (pass) | 0.23929954101913609 (pass) | not reached |
| swift | name="$(scripts/version.sh app-name)" ... | 0.0 | 0.0 (skipped: local landing: release bundle and its consumers run in CI) | 0.0 (skipped: local landing: release bundle and its consumers run in CI) | not reached |
| swift | apps/macos/build-app.sh | 334.0 | 0.0 (skipped: local landing: release bundle and its consumers run in CI) | 0.0 (skipped: local landing: release bundle and its consumers run in CI) | not reached |
| swift | scripts/tunnel-smoke.sh "apps/macos/build/$APP_NAME.app/Contents/Resources/farcoolerd" | 1.0 | 0.0 (skipped: local landing: release bundle and its consumers run in CI) | 0.0 (skipped: local landing: release bundle and its consumers run in CI) | not reached |
| swift | out="$("apps/macos/build/$APP_NAME.app/Contents/Resources/farcooler" runner mint-node-key)" ... | 0.0 | 0.0 (skipped: local landing: release bundle and its consumers run in CI) | 0.0 (skipped: local landing: release bundle and its consumers run in CI) | not reached |
| swift | app="apps/macos/build/$APP_NAME.app" ... | 0.0 | 0.0 (skipped: local landing: release bundle and its consumers run in CI) | 0.0 (skipped: local landing: release bundle and its consumers run in CI) | not reached |
| swift | app="apps/macos/build/$APP_NAME.app" ... | 0.0 | 0.0 (skipped: local landing: release bundle and its consumers run in CI) | 0.0 (skipped: local landing: release bundle and its consumers run in CI) | not reached |
| swift | ./scripts/icon-test.sh | 2.0 | 10.136447875003796 (pass) | 3.1079666670120787 (pass) | not reached |
| swift | ./scripts/sparkle-key-test.sh | 1.0 | 3.671909750002669 (pass) | 1.6256457080016844 (pass) | not reached |
| swift | ./scripts/gh-download-test.sh | 3.0 | 3.8136322500067763 (pass) | 3.477600749989506 (pass) | not reached |
| swift | ./scripts/macos-entitlements-test.sh | not in capture | 0.2019940000027418 (pass) | 0.16085387498605996 (pass) | not reached |
| ios | ./scripts/ios-ui-shards.py --check | not in capture | 0.09699316701153293 (pass) | 0.09132091599167325 (pass) | not reached |
| ios | ./scripts/ios-ui-shards.py --self-test | not in capture | 0.5812204999965616 (pass) | 0.3476955420046579 (pass) | not reached |
| ios | ./scripts/ios-ui-pick-simulator.py --self-test | not in capture | 0.07445162499789149 (pass) | 0.07217112500802614 (pass) | not reached |
| ios | if gh api "repos/$GITHUB_REPOSITORY/actions/workflows/ci.yml/runs?branch=main&event=push&status=success&per_page=100" \ ... | not in capture | 0.0 (skipped: decides what CI runs, or builds the iOS archive; see scripts/ios-ui-tests.sh) | 0.0 (skipped: decides what CI runs, or builds the iOS archive; see scripts/ios-ui-tests.sh) | not reached |
| ios | ./scripts/ios-ui-plan.py | not in capture | 0.0 (skipped: decides what CI runs, or builds the iOS archive; see scripts/ios-ui-tests.sh) | 0.0 (skipped: decides what CI runs, or builds the iOS archive; see scripts/ios-ui-tests.sh) | not reached |
| ios | ./scripts/ios-ui-plan.py --self-test | not in capture | 1.212034374999348 (pass) | 1.017774959007511 (pass) | not reached |
| ios | xcodebuild -version ... | not in capture | 0.0 (skipped: checks the runner, installs the watchOS platform or packages products for CI's UI shards) | 0.0 (skipped: checks the runner, installs the watchOS platform or packages products for CI's UI shards) | not reached |
| ios | ./scripts/build-ios-frameworks.sh | not in capture | 146.2663660410035 (pass) | 148.5416335000191 (pass) | not reached |
| web | GOOS=js GOARCH=wasm go test -exec "$(go env GOROOT)/lib/wasm/go_js_wasm_exec" . | 5.0 | 7.160725124995224 (pass) | 7.251726291986415 (pass) | not reached |
| web | node scripts/size-gate.mjs --self-test | 0.0 | 0.233370375004597 (pass) | 0.16692183399572968 (pass) | not reached |
| web | ../../crates/web/build.sh --channel stable --out "$RUNNER_TEMP/wasm-stable" | None | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | not reached |
| web | npx tsc --noEmit -p tsconfig.client.json | 1.0 | 4.062097791000269 (pass) | 1.5376483330037445 (pass) | not reached |
| web | WEB_WASM_DIR="$RUNNER_TEMP/wasm-stable" ./build.sh --channel stable --build "$GITHUB_SHA" | None | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | not reached |
| web | node scripts/size-gate.mjs dist | None | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | not reached |
| web | npx wrangler deploy --dry-run --outdir /tmp/web-build | None | 0.0 (skipped: stable web asset build skipped (needs the Linux x86 wasm toolchain CI downloads; left to CI); dry-run left to CI) | 0.0 (skipped: stable web asset build skipped (needs the Linux x86 wasm toolchain CI downloads; left to CI); dry-run left to CI) | not reached |
| web | ../../crates/web/build.sh --channel canary --out "$RUNNER_TEMP/wasm-canary" | None | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | not reached |
| web | WEB_WASM_DIR="$RUNNER_TEMP/wasm-canary" ./build.sh --channel canary --build "$GITHUB_SHA" | None | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | not reached |
| web | node scripts/size-gate.mjs dist | None | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | not reached |
| web | npx wrangler deploy --dry-run --env canary --outdir /tmp/web-build-canary | None | 0.0 (skipped: canary web asset build skipped (needs the Linux x86 wasm toolchain CI downloads; left to CI); dry-run left to CI) | 0.0 (skipped: canary web asset build skipped (needs the Linux x86 wasm toolchain CI downloads; left to CI); dry-run left to CI) | not reached |
| web | cargo build --locked --manifest-path ../../crates/web/Cargo.toml --example fake_daemon | None | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | not reached |
| web | npx playwright-core install --with-deps --only-shell chromium | None | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | not reached |
| web | node test/browser/plan-render-check.mjs | 1.0 | 1.990328915999271 (pass) | 1.1053888750029728 (pass) | not reached |
| web | node test/browser/csp-check.mjs --fake-daemon ../../crates/web/target/debug/examples/fake_daemon --tunnel-delay 100 | None | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | not reached |
| web | node test/browser/sleep-check.mjs --fake-daemon ../../crates/web/target/debug/examples/fake_daemon --engines chromium | None | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | 0.0 (skipped: needs the Linux x86 wasm toolchain CI downloads; left to CI) | not reached |
| ios | apps/ios/generate-project.py | not in capture | 4.228618249995634 (pass) | 5.54319175001001 (pass) | not reached |
| ios | xcodebuild -downloadPlatform watchOS | not in capture | 0.0 (skipped: checks the runner, installs the watchOS platform or packages products for CI's UI shards) | 0.0 (skipped: checks the runner, installs the watchOS platform or packages products for CI's UI shards) | not reached |
| ios | xcodebuild -project apps/ios/FarCooler.xcodeproj \ ... | not in capture | 105.99956645900966 (pass) | 164.36851895801374 (pass) | not reached |
| ios | runs=("$RUNNER_TEMP"/ios-dd/Build/Products/*.xctestrun) ... | not in capture | 0.0 (skipped: checks the runner, installs the watchOS platform or packages products for CI's UI shards) | 0.0 (skipped: checks the runner, installs the watchOS platform or packages products for CI's UI shards) | not reached |
| ios | ./scripts/import-ios-certificate-test.sh | not in capture | 0.03885679098311812 (pass) | 0.05492066699662246 (pass) | not reached |
| ios | ./scripts/generator-test.sh | not in capture | 3.4582233330002055 (pass) | 3.752395499992417 (pass) | not reached |
| rust | cargo test --workspace --doc | 4.0 | 11.716854750004131 (pass) | 55.29234250000445 (pass) | not reached |
