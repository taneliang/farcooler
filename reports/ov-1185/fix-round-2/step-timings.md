| Job | Command | Historical seconds | Real 1 | Real 2 | Real 3 |
| --- | --- | ---: | ---: | ---: | ---: |
| wire | ./scripts/proto-lint.py --self-test | 1.0 | not reached | not run | not run |
| wire | ./scripts/proto-lint.py --channel preview | 0.0 | not reached | not run | not run |
| wire | ./scripts/proto-lint.py --channel stable | 0.0 | not reached | not run | not run |
| wire | ./scripts/proto-lint.py --channel canary | 0.0 | not reached | not run | not run |
| wire | ./scripts/canary-baseline-test.sh | 10.0 | not reached | not run | not run |
| wire | ./scripts/canary-shipped.py --self-test | 0.0 | not reached | not run | not run |
| wire | ./scripts/canary-plan.py --self-test | 2.0 | not reached | not run | not run |
| wire | ./scripts/device-keys-readiness.py --self-test | 0.0 | not reached | not run | not run |
| wire | ./scripts/site-proof.py --self-test | 0.0 | not reached | not run | not run |
| wire | ./scripts/ci-changes.py --self-test | 1.0 | not reached | not run | not run |
| wire | HEAVY_LOCK_HELD=1 ./scripts/lane-gate.py --self-test | 8.0 | not reached | not run | not run |
| wire | HEAVY_LOCK_HELD=1 ./scripts/heavy_lock.py --self-test | 17.0 | not reached | not run | not run |
| wire | ./scripts/notarytool-retry.sh --self-test | 1.0 | not reached | not run | not run |
| wire | ./scripts/version-test.sh | 5.0 | not reached | not run | not run |
| wire | ./scripts/workos-test.sh | 1.0 | not reached | not run | not run |
| wire | ./scripts/appcast-test.sh | 0.0 | not reached | not run | not run |
| wire | ./scripts/ci-flake-tally-test.sh | 1.0 | not reached | not run | not run |
| wire | ./scripts/ios-ui-retry-test.sh | 56.0 | not reached | not run | not run |
| wire | ./scripts/ios-ui-lease-test.sh | 63.0 | not reached | not run | not run |
| wire | ./scripts/copy-lint.py --self-test | 0.0 | not reached | not run | not run |
| wire | ./scripts/test-leak-check.py --lint | 1.0 | not reached | not run | not run |
| wire | ./scripts/copy-lint.py | 1.0 | not reached | not run | not run |
| wire | ./scripts/shared-core-lint.py --self-test | 0.0 | not reached | not run | not run |
| wire | ./scripts/shared-core-lint.py | 1.0 | not reached | not run | not run |
| wire | git fetch --no-tags origin main ... | 3.0 | not reached | not run | not run |
| wire | ./scripts/capability-names.py --self-test ... | 0.0 | not reached | not run | not run |
| wire | ./scripts/claude-prices.py --self-test | 0.0 | not reached | not run | not run |
| wire | ./scripts/openai-prices.py --self-test | 0.0 | not reached | not run | not run |
| wire | ./scripts/derp-hosts-check.py --self-test | 0.0 | not reached | not run | not run |
| wire | ./scripts/file-size-budget.py --self-test ... | 0.0 | not reached | not run | not run |
| wire | ./scripts/plan-layer-lint.py --self-test ... | 1.0 | not reached | not run | not run |
| wire | ./scripts/page-layer-lint.py --self-test ... | 16.0 | not reached | not run | not run |
| wire | ./scripts/swallow-lint.py --self-test | 0.0 | not reached | not run | not run |
| wire | ./scripts/swallow-lint.py | 0.0 | not reached | not run | not run |
| wire | ./scripts/visual-tokens-lint.py --self-test | 0.0 | not reached | not run | not run |
| wire | ./scripts/visual-tokens-lint.py | 1.0 | not reached | not run | not run |
| wire | ./scripts/pixel-scale-lint.py --self-test | 0.0 | not reached | not run | not run |
| wire | ./scripts/pixel-scale-lint.py | 0.0 | not reached | not run | not run |
| wire | ./scripts/test-window-lint.py --self-test | 0.0 | not reached | not run | not run |
| wire | ./scripts/test-window-lint.py | 0.0 | not reached | not run | not run |
| wire | ./scripts/runloop-spin-lint.py --self-test | 0.0 | not reached | not run | not run |
| wire | ./scripts/runloop-spin-lint.py | 0.0 | not reached | not run | not run |
| wire | ./scripts/capture-window-lint.py --self-test | 0.0 | not reached | not run | not run |
| wire | ./scripts/capture-window-lint.py | 0.0 | not reached | not run | not run |
| wire | ./scripts/fresh-script-lint.py --self-test | 0.0 | not reached | not run | not run |
| wire | ./scripts/fresh-script-lint.py | 0.0 | not reached | not run | not run |
| wire | ./scripts/charter-intercept-lint.py --self-test | 0.0 | not reached | not run | not run |
| wire | ./scripts/charter-intercept-lint.py | 0.0 | not reached | not run | not run |
| wire | ./scripts/ios-agentkit-subset-lint.py --self-test | 0.0 | not reached | not run | not run |
| wire | ./scripts/ios-agentkit-subset-lint.py | 2.0 | not reached | not run | not run |
| wire | ./scripts/migration-lint.py --self-test | 1.0 | not reached | not run | not run |
| wire | ./scripts/migration-lint.py | 0.0 | not reached | not run | not run |
| wire | if [ "$(typeshare --version 2>/dev/null)" != "typeshare 1.13.4" ]; then ... | 0.0 | not reached | not run | not run |
| wire | ./scripts/gen-wire-types.sh ... | 0.0 | not reached | not run | not run |
| doc-comments | ./scripts/doc-comment-check.py --self-test | 0.0 | not reached | not run | not run |
| doc-comments | ./scripts/doc-comment-check.py --base origin/main --head HEAD | 0.0 | not reached | not run | not run |
| relay | npm ci | 6.0 | not reached | not run | not run |
| relay | npx tsc --noEmit | 1.0 | not reached | not run | not run |
| relay | npm test | 14.0 | not reached | not run | not run |
| relay | npx wrangler deploy --dry-run --outdir /tmp/relay-build | 1.0 | not reached | not run | not run |
| rust | ./scripts/ci-install-tmux.sh | None | not reached | not run | not run |
| rust | if command -v git-lfs >/dev/null 2>&1; then ... | None | not reached | not run | not run |
| rust | if ! command -v fish >/dev/null 2>&1; then ... | None | not reached | not run | not run |
| rust | if [ ! -x /usr/sbin/sshd ]; then ... | None | not reached | not run | not run |
| rust | cargo clippy --workspace --all-targets -- -D warnings | 37.0 | not reached | not run | not run |
| rust | cargo clippy -p farcooler-tailcat --features helper --all-targets -- -D warnings | 4.0 | not reached | not run | not run |
| rust | cargo clippy -p farcooler-tailcat --features linked --all-targets -- -D warnings | 0.0 | not reached | not run | not run |
| rust | cargo clippy -p farcooler-daemon --features tailcat-helper --all-targets -- -D warnings | 26.0 | not reached | not run | not run |
| rust | cargo clippy -p farcooler-daemon --features tailcat --all-targets -- -D warnings | 26.0 | not reached | not run | not run |
| rust | cargo clippy -p farcooler-client --features tailcat --all-targets -- -D warnings | 19.0 | not reached | not run | not run |
| rust | cargo clippy -p farcooler-cli --features tailcat --all-targets -- -D warnings | 9.0 | not reached | not run | not run |
| rust | cargo clippy -p farcooler-jni --features tailcat --all-targets -- -D warnings | 4.0 | not reached | not run | not run |
| rust | cargo check -p farcooler-tailcat --features linked | 0.0 | not reached | not run | not run |
| rust | cargo check -p farcooler-tailcat --features linked --target aarch64-unknown-linux-musl | 0.0 | not reached | not run | not run |
| rust | cargo nextest run --profile ci -p farcooler-tailcat --features helper | 6.0 | not reached | not run | not run |
| rust | cargo nextest run --profile ci -p farcooler-daemon --features tailcat-helper --test an_empty_allowlist_starts_no_tunnel | 24.0 | not reached | not run | not run |
| rust | cargo nextest run --profile ci -p farcooler-daemon --features tailcat-helper --test revocation_closes_what_it_revoked | 5.0 | not reached | not run | not run |
| rust | cargo nextest run --profile ci -p farcooler-daemon --features tailcat-helper --test a_pairing_gives_the_runner_a_tunnel | 1.0 | not reached | not run | not run |
| rust | cargo nextest run --profile ci -p farcooler-daemon --features tailcat-helper --test a_sweep_rebuilds_the_tunnel_once | 2.0 | not reached | not run | not run |
| rust | cargo nextest run --profile ci -p farcooler-daemon --features tailcat-helper --test a_configured_derp_map_reaches_the_tunnel | 1.0 | not reached | not run | not run |
| rust | cargo check -p farcooler-daemon --features tailcat --all-targets | 27.0 | not reached | not run | not run |
| rust | cargo build --workspace --bins ... | 21.0 | not reached | not run | not run |
| rust | ./scripts/tmux-leak-check.py --self-test | 22.0 | not reached | not run | not run |
| rust | ./scripts/tmux-stale-sockets.py --self-test | 0.0 | not reached | not run | not run |
| rust | ./scripts/test-leak-check.py --self-test | 8.0 | not reached | not run | not run |
| rust | ./scripts/test-leak-check.py -- ./scripts/tmux-leak-check.py -- cargo nextest run --profile ci --workspace | 665.0 | not reached | not run | not run |
| rust | cargo test --workspace --doc | 4.0 | not reached | not run | not run |
| swift-tests | xcodebuild -version ... | 0.0 | not reached | not run | not run |
| swift-tests | ./scripts/macos-test-wrapper-test.sh | 8.0 | not reached | not run | not run |
| swift-tests | ./scripts/wait-for-unlock-test.sh | 4.0 | not reached | not run | not run |
| swift-tests | ./scripts/swift-test-guard-test.sh | 32.0 | not reached | not run | not run |
| swift-tests | ./scripts/test-leak-check.py --self-test | 8.0 | not reached | not run | not run |
| swift-tests | ./scripts/mac-capture-stop-test.sh | 1.0 | not reached | not run | not run |
| swift-tests | ./scripts/capture-duplicates-test.sh | 2.0 | not reached | not run | not run |
| swift-tests | cargo build --release -p farcooler-vt -p farcooler-client | 150.0 | not reached | not run | not run |
| swift-tests | ./scripts/swift-test-guard.sh apps/shared/AgentKit/test-floor.txt -- ./scripts/test-leak-check.py -- swift test --package-path apps/shared/AgentKit | 88.0 | not reached | not run | not run |
| swift-tests | ./scripts/ci-install-tmux.sh | None | not reached | not run | not run |
| swift-tests | ./apps/macos/test.sh | 429.0 | not reached | not run | not run |
| swift | xcodebuild -version ... | 0.0 | not reached | not run | not run |
| swift | name="$(scripts/version.sh app-name)" ... | 0.0 | not reached | not run | not run |
| swift | apps/macos/build-app.sh | 334.0 | not reached | not run | not run |
| swift | scripts/tunnel-smoke.sh "apps/macos/build/$APP_NAME.app/Contents/Resources/farcoolerd" | 1.0 | not reached | not run | not run |
| swift | out="$("apps/macos/build/$APP_NAME.app/Contents/Resources/farcooler" runner mint-node-key)" ... | 0.0 | not reached | not run | not run |
| swift | app="apps/macos/build/$APP_NAME.app" ... | 0.0 | not reached | not run | not run |
| swift | app="apps/macos/build/$APP_NAME.app" ... | 0.0 | not reached | not run | not run |
| swift | ./scripts/icon-test.sh | 2.0 | not reached | not run | not run |
| swift | ./scripts/sparkle-key-test.sh | 1.0 | not reached | not run | not run |
| swift | ./scripts/gh-download-test.sh | 3.0 | not reached | not run | not run |
| swift | ./scripts/macos-entitlements-test.sh "apps/macos/build/$APP_NAME.app" | 0.0 | not reached | not run | not run |
| android | echo "y" \| "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" \ ... | None | not reached | not run | not run |
| android | ./scripts/build-android-libs.sh | 54.0 | not reached | not run | not run |
| android | cargo build -p farcooler-jni | 14.0 | not reached | not run | not run |
| android | ./gradlew testInstrumentedUnitTest "-Pfarcooler.nativeLibDir=$GITHUB_WORKSPACE/target/debug" | 30.0 | not reached | not run | not run |
| android | ./gradlew assembleDebug assembleAndroidTest | 36.0 | not reached | not run | not run |
| android | ./gradlew assembleRelease | 106.0 | not reached | not run | not run |
| android | ./gradlew testInstrumentedUnitTest \ ... | 324.0 | not reached | not run | not run |
| android | python3 - <<'PY' ... | 0.0 | not reached | not run | not run |
| web | mkdir -p wasm-tools ... | None | not reached | not run | not run |
| web | echo "WASM_BINDGEN=$RUNNER_TEMP/wasm-tools/wasm-bindgen-$WASM_BINDGEN_VERSION-x86_64-unknown-linux-musl/wasm-bindgen" >> "$GITHUB_ENV" ... | None | not reached | not run | not run |
| web | cargo check --locked -p farcooler-client --no-default-features --features web --target wasm32-unknown-unknown | 3.0 | not reached | not run | not run |
| web | ./scripts/web-client-tree-check.py --self-test | 0.0 | not reached | not run | not run |
| web | ./scripts/web-client-tree-check.py | 0.0 | not reached | not run | not run |
| web | npm ci | 5.0 | not reached | not run | not run |
| web | npx tsc --noEmit -p tsconfig.json | 0.0 | not reached | not run | not run |
| web | npm test | 2.0 | not reached | not run | not run |
| web | npm run test:workflow | 0.0 | not reached | not run | not run |
| web | if [ ! -x /usr/sbin/sshd ]; then ... | None | not reached | not run | not run |
| web | cargo test --locked --manifest-path ../../crates/web/Cargo.toml | 36.0 | not reached | not run | not run |
| web | GOOS=js GOARCH=wasm go test -exec "$(go env GOROOT)/lib/wasm/go_js_wasm_exec" . | 5.0 | not reached | not run | not run |
| web | node scripts/size-gate.mjs --self-test | 0.0 | not reached | not run | not run |
| web | ../../crates/web/build.sh --channel stable --out "$RUNNER_TEMP/wasm-stable" | None | not reached | not run | not run |
| web | npx tsc --noEmit -p tsconfig.client.json | 1.0 | not reached | not run | not run |
| web | WEB_WASM_DIR="$RUNNER_TEMP/wasm-stable" ./build.sh --channel stable --build "$GITHUB_SHA" | None | not reached | not run | not run |
| web | node scripts/size-gate.mjs dist | None | not reached | not run | not run |
| web | npx wrangler deploy --dry-run --outdir /tmp/web-build | None | not reached | not run | not run |
| web | ../../crates/web/build.sh --channel canary --out "$RUNNER_TEMP/wasm-canary" | None | not reached | not run | not run |
| web | WEB_WASM_DIR="$RUNNER_TEMP/wasm-canary" ./build.sh --channel canary --build "$GITHUB_SHA" | None | not reached | not run | not run |
| web | node scripts/size-gate.mjs dist | None | not reached | not run | not run |
| web | npx wrangler deploy --dry-run --env canary --outdir /tmp/web-build-canary | None | not reached | not run | not run |
| web | cargo build --locked --manifest-path ../../crates/web/Cargo.toml --example fake_daemon | None | not reached | not run | not run |
| web | npx playwright-core install --with-deps --only-shell chromium | None | not reached | not run | not run |
| web | node test/browser/plan-render-check.mjs | 1.0 | not reached | not run | not run |
| web | node test/browser/csp-check.mjs --fake-daemon ../../crates/web/target/debug/examples/fake_daemon --tunnel-delay 100 | None | not reached | not run | not run |
| web | node test/browser/sleep-check.mjs --fake-daemon ../../crates/web/target/debug/examples/fake_daemon --engines chromium | None | not reached | not run | not run |
