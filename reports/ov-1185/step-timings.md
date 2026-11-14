| Job | Historical command | Before seconds | Landing seconds | Reason |
| --- | --- | ---: | ---: | --- |
| rust | cargo check -p farcooler-tailcat --features linked | 0.0 | 0.0 | retained |
| rust | cargo check -p farcooler-tailcat --features linked --target aarch64-unknown-linux-musl | 0.0 | 0.0 | retained |
| rust | cargo nextest run --profile ci -p farcooler-tailcat --features helper | 6.0 | 6.0 | retained |
| swift-tests | xcodebuild -version ... | 0.0 | 0.0 | retained |
| swift-tests | ./scripts/macos-test-wrapper-test.sh | 8.0 | 8.0 | retained |
| android | ./scripts/build-android-libs.sh | 54.0 | 54.0 | retained |
| rust-lints | cargo clippy --workspace --all-targets -- -D warnings | 37.0 | 37.0 | retained |
| rust | cargo nextest run --profile ci -p farcooler-daemon --features tailcat-helper --test an_empty_allowlist_starts_no_tunnel | 24.0 | 24.0 | retained |
| swift-tests | ./scripts/wait-for-unlock-test.sh | 4.0 | 4.0 | retained |
| swift-tests | ./scripts/swift-test-guard-test.sh | 32.0 | 32.0 | retained |
| rust | cargo nextest run --profile ci -p farcooler-daemon --features tailcat-helper --test revocation_closes_what_it_revoked | 5.0 | 5.0 | retained |
| rust | cargo nextest run --profile ci -p farcooler-daemon --features tailcat-helper --test a_pairing_gives_the_runner_a_tunnel | 1.0 | 1.0 | retained |
| rust-lints | cargo clippy -p farcooler-tailcat --features helper --all-targets -- -D warnings | 4.0 | 4.0 | retained |
| rust | cargo nextest run --profile ci -p farcooler-daemon --features tailcat-helper --test a_sweep_rebuilds_the_tunnel_once | 2.0 | 2.0 | retained |
| rust | cargo nextest run --profile ci -p farcooler-daemon --features tailcat-helper --test a_configured_derp_map_reaches_the_tunnel | 1.0 | 1.0 | retained |
| rust | cargo check -p farcooler-daemon --features tailcat --all-targets | 27.0 | 27.0 | retained |
| rust-lints | cargo clippy -p farcooler-tailcat --features linked --all-targets -- -D warnings | 0.0 | 0.0 | retained |
| rust-lints | cargo clippy -p farcooler-daemon --features tailcat-helper --all-targets -- -D warnings | 26.0 | 26.0 | retained |
| swift-tests | ./scripts/test-leak-check.py --self-test | 8.0 | 8.0 | retained |
| swift-tests | ./scripts/mac-capture-stop-test.sh | 1.0 | 1.0 | retained |
| android | cargo build -p farcooler-jni | 14.0 | 14.0 | retained |
| swift-tests | ./scripts/capture-duplicates-test.sh | 2.0 | 2.0 | retained |
| swift-tests | cargo build --release -p farcooler-vt -p farcooler-client | 150.0 | 150.0 | retained |
| rust-lints | cargo clippy -p farcooler-daemon --features tailcat --all-targets -- -D warnings | 26.0 | 26.0 | retained |
| rust | cargo build --workspace --bins ... | 21.0 | 21.0 | retained |
| android | ./gradlew testInstrumentedUnitTest "-Pfarcooler.nativeLibDir=$GITHUB_WORKSPACE/target/debug" | 30.0 | 30.0 | retained |
| rust | ./scripts/tmux-leak-check.py --self-test | 22.0 | 22.0 | retained |
| rust-lints | cargo clippy -p farcooler-client --features tailcat --all-targets -- -D warnings | 19.0 | 19.0 | retained |
| android | ./gradlew assembleDebug assembleAndroidTest | 36.0 | 36.0 | retained |
| rust | ./scripts/tmux-stale-sockets.py --self-test | 0.0 | 0.0 | retained |
| rust | ./scripts/test-leak-check.py --self-test | 8.0 | 8.0 | retained |
| rust-lints | cargo clippy -p farcooler-cli --features tailcat --all-targets -- -D warnings | 9.0 | 9.0 | retained |
| rust | ./scripts/test-leak-check.py -- ./scripts/tmux-leak-check.py -- cargo nextest run --profile ci --workspace | 665.0 | 665.0 | retained |
| rust-lints | cargo clippy -p farcooler-jni --features tailcat --all-targets -- -D warnings | 4.0 | 4.0 | retained |
| wire | ./scripts/proto-lint.py --self-test | 1.0 | 1.0 | retained |
| wire | ./scripts/proto-lint.py --channel preview | 0.0 | 0.0 | retained |
| wire | ./scripts/proto-lint.py --channel stable | 0.0 | 0.0 | retained |
| wire | ./scripts/proto-lint.py --channel canary | 0.0 | 0.0 | retained |
| wire | ./scripts/canary-baseline-test.sh | 10.0 | 10.0 | retained |
| android | ./gradlew assembleRelease | 106.0 | 106.0 | retained |
| wire | ./scripts/canary-shipped.py --self-test | 0.0 | 0.0 | retained |
| wire | ./scripts/canary-plan.py --self-test | 2.0 | 2.0 | retained |
| wire | ./scripts/device-keys-readiness.py --self-test | 0.0 | 0.0 | retained |
| wire | ./scripts/site-proof.py --self-test | 0.0 | 0.0 | retained |
| wire | ./scripts/ci-changes.py --self-test | 1.0 | 1.0 | retained |
| wire | HEAVY_LOCK_HELD=1 ./scripts/lane-gate.py --self-test | 8.0 | 8.0 | retained |
| wire | HEAVY_LOCK_HELD=1 ./scripts/heavy_lock.py --self-test | 17.0 | 17.0 | retained |
| wire | ./scripts/notarytool-retry.sh --self-test | 1.0 | 1.0 | retained |
| wire | ./scripts/version-test.sh | 5.0 | 5.0 | retained |
| wire | ./scripts/workos-test.sh | 1.0 | 1.0 | retained |
| wire | ./scripts/appcast-test.sh | 0.0 | 0.0 | retained |
| wire | ./scripts/ci-flake-tally-test.sh | 1.0 | 1.0 | retained |
| wire | ./scripts/ios-ui-retry-test.sh | 56.0 | 56.0 | retained |
| swift-tests | ./scripts/swift-test-guard.sh apps/shared/AgentKit/test-floor.txt -- ./scripts/test-leak-check.py -- swift test --package-path apps/shared/AgentKit | 88.0 | 88.0 | retained |
| wire | ./scripts/ios-ui-lease-test.sh | 63.0 | 63.0 | retained |
| android | ./gradlew testInstrumentedUnitTest \ ... | 324.0 | 324.0 | retained |
| swift-tests | ./apps/macos/test.sh | 429.0 | 429.0 | retained |
| wire | ./scripts/copy-lint.py --self-test | 0.0 | 0.0 | retained |
| wire | ./scripts/test-leak-check.py --lint | 1.0 | 1.0 | retained |
| wire | ./scripts/copy-lint.py | 1.0 | 1.0 | retained |
| wire | ./scripts/shared-core-lint.py --self-test | 0.0 | 0.0 | retained |
| wire | ./scripts/shared-core-lint.py | 1.0 | 1.0 | retained |
| wire | git fetch --no-tags origin main ... | 3.0 | 3.0 | retained |
| wire | ./scripts/capability-names.py --self-test ... | 0.0 | 0.0 | retained |
| wire | ./scripts/claude-prices.py --self-test | 0.0 | 0.0 | retained |
| wire | ./scripts/openai-prices.py --self-test | 0.0 | 0.0 | retained |
| wire | ./scripts/derp-hosts-check.py --self-test | 0.0 | 0.0 | retained |
| wire | ./scripts/file-size-budget.py --self-test ... | 0.0 | 0.0 | retained |
| wire | ./scripts/plan-layer-lint.py --self-test ... | 1.0 | 1.0 | retained |
| wire | ./scripts/page-layer-lint.py --self-test ... | 16.0 | 16.0 | retained |
| wire | ./scripts/swallow-lint.py --self-test | 0.0 | 0.0 | retained |
| wire | ./scripts/swallow-lint.py | 0.0 | 0.0 | retained |
| wire | ./scripts/visual-tokens-lint.py --self-test | 0.0 | 0.0 | retained |
| wire | ./scripts/visual-tokens-lint.py | 1.0 | 1.0 | retained |
| wire | ./scripts/pixel-scale-lint.py --self-test | 0.0 | 0.0 | retained |
| wire | ./scripts/pixel-scale-lint.py | 0.0 | 0.0 | retained |
| wire | ./scripts/test-window-lint.py --self-test | 0.0 | 0.0 | retained |
| wire | ./scripts/test-window-lint.py | 0.0 | 0.0 | retained |
| wire | ./scripts/runloop-spin-lint.py --self-test | 0.0 | 0.0 | retained |
| wire | ./scripts/runloop-spin-lint.py | 0.0 | 0.0 | retained |
| wire | ./scripts/capture-window-lint.py --self-test | 0.0 | 0.0 | retained |
| wire | ./scripts/capture-window-lint.py | 0.0 | 0.0 | retained |
| wire | ./scripts/fresh-script-lint.py --self-test | 0.0 | 0.0 | retained |
| wire | ./scripts/fresh-script-lint.py | 0.0 | 0.0 | retained |
| wire | ./scripts/charter-intercept-lint.py --self-test | 0.0 | 0.0 | retained |
| wire | ./scripts/charter-intercept-lint.py | 0.0 | 0.0 | retained |
| wire | ./scripts/ios-agentkit-subset-lint.py --self-test | 0.0 | 0.0 | retained |
| wire | ./scripts/ios-agentkit-subset-lint.py | 2.0 | 2.0 | retained |
| wire | ./scripts/migration-lint.py --self-test | 1.0 | 1.0 | retained |
| wire | ./scripts/migration-lint.py | 0.0 | 0.0 | retained |
| wire | if [ "$(typeshare --version 2>/dev/null)" != "typeshare 1.13.4" ]; then ... | 0.0 | 0.0 | retained |
| wire | ./scripts/gen-wire-types.sh ... | 0.0 | 0.0 | retained |
| doc-comments | ./scripts/doc-comment-check.py --self-test | 0.0 | 0.0 | retained |
| doc-comments | ./scripts/doc-comment-check.py --base origin/main --head HEAD | 0.0 | 0.0 | retained |
| relay | npm ci | 6.0 | 6.0 | retained |
| relay | npx tsc --noEmit | 1.0 | 1.0 | retained |
| relay | npm test | 14.0 | 14.0 | retained |
| relay | npx wrangler deploy --dry-run --outdir /tmp/relay-build | 1.0 | 1.0 | retained |
| web | cargo check --locked -p farcooler-client --no-default-features --features web --target wasm32-unknown-unknown | 3.0 | 3.0 | retained |
| web | ./scripts/web-client-tree-check.py --self-test | 0.0 | 0.0 | retained |
| web | ./scripts/web-client-tree-check.py | 0.0 | 0.0 | retained |
| web | npm ci | 5.0 | 5.0 | retained |
| web | npx tsc --noEmit -p tsconfig.json | 0.0 | 0.0 | retained |
| web | npm test | 2.0 | 2.0 | retained |
| web | npm run test:workflow | 0.0 | 0.0 | retained |
| web | cargo test --locked --manifest-path ../../crates/web/Cargo.toml | 36.0 | 36.0 | retained |
| web | GOOS=js GOARCH=wasm go test -exec "$(go env GOROOT)/lib/wasm/go_js_wasm_exec" . | 5.0 | 5.0 | retained |
| web | node scripts/size-gate.mjs --self-test | 0.0 | 0.0 | retained |
| web | npx tsc --noEmit -p tsconfig.client.json | 1.0 | 1.0 | retained |
| web | node test/browser/plan-render-check.mjs | 1.0 | 1.0 | retained |
| android | python3 - <<'PY' ... | 0.0 | 0.0 | retained |
| swift | xcodebuild -version ... | 0.0 | 0.0 | retained |
| swift | name="$(scripts/version.sh app-name)" ... | 0.0 | 0 | local landing: release bundle and its consumers run in CI |
| swift | apps/macos/build-app.sh | 334.0 | 0 | local landing: release bundle and its consumers run in CI |
| swift | scripts/tunnel-smoke.sh "apps/macos/build/$APP_NAME.app/Contents/Resources/farcoolerd" | 1.0 | 0 | local landing: release bundle and its consumers run in CI |
| swift | out="$("apps/macos/build/$APP_NAME.app/Contents/Resources/farcooler" runner mint-node-key)" ... | 0.0 | 0 | local landing: release bundle and its consumers run in CI |
| swift | app="apps/macos/build/$APP_NAME.app" ... | 0.0 | 0 | local landing: release bundle and its consumers run in CI |
| swift | app="apps/macos/build/$APP_NAME.app" ... | 0.0 | 0 | local landing: release bundle and its consumers run in CI |
| swift | ./scripts/icon-test.sh | 2.0 | 2.0 | retained |
| swift | ./scripts/sparkle-key-test.sh | 1.0 | 1.0 | retained |
| swift | ./scripts/gh-download-test.sh | 3.0 | 3.0 | retained |
| swift | ./scripts/macos-entitlements-test.sh | 0.0 | 0.0 | retained |
| rust | cargo test --workspace --doc | 4.0 | 4.0 | retained |
