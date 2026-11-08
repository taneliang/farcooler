import subprocess, sys, shutil, re
root = '/Users/e-liang/Library/Application Support/com.farcooler.FarCoolerCanary/worktrees/overnight/ov-984-recovery-button/apps/macos/'
S = root + 'Sources/FarCooler/'
mutants = [
  ('upgrade-phase-dropped', S+'FleetPlaceholder.swift', 'if runner?.isEmpty ?? true, isMidUpgrade(localError) { return .upgrade }', '', 'aRunnerMidUpgradeDrawsTheRecoveryScreen'),
  ('no-recover-on-failure', S+'RunnerUpgradeView.swift', 'case .failed: [.recover]', 'case .failed: []', 'aFailureAtAnyStep'),
  ('committed-rolled-back', S+'RunnerUpgrade.swift', 'case .at(let step, _) where step.isCommitted:', 'case .at(let step, _) where step.isCommitted && false:', 'recoverAfterCommitFinishesTheStepsLeft'),
  ('no-touch-id', S+'RunnerUpgrade.swift', 'guard await confirmed(Self.finishReason) else { return }', '', 'noTouchIDRunsNothing'),
  ('held-attempt-ignored', S+'RunnerUpgrade.swift', 'attempt = held', 'attempt = nil', 'aHeldRunnerIsPreparedUnderItsOwnAttempt'),
  ('failure-runs-on', S+'RunnerUpgrade.swift', '''                phase = .failed(.step(step), detail: ran.said)
                return''', '''                phase = .failed(.step(step), detail: ran.said)
                continue''', 'aFailureAtAnyStep'),
]
results = []
for name, path, old, new, test in mutants:
    src = open(path).read()
    assert src.count(old) == 1, name
    try:
        open(path, 'w').write(src.replace(old, new))
        r = subprocess.run(['./test.sh', '--filter', 'RunnerUpgradeTests'], cwd=root, capture_output=True, text=True, env={**__import__('os').environ, 'CARGO_BUILD_JOBS': '6'})
        out = r.stdout + r.stderr
        red = re.search(r'✘ Test %s' % re.escape(test), out) is not None
        built = 'Test run started' in out
        results.append((name, 'RED' if red else ('NOT BUILT' if not built else 'green!'), test))
        print(name, results[-1][1], flush=True)
    finally:
        open(path, 'w').write(src)
print(results)
