// The workflow card's two standalone Node tests (test/workflow.test.mjs and
// test/workflow-session.test.mjs). Each takes a directory of the compiled
// client modules as its one argument, so vitest, which would run any
// test/*.test.mjs bare, leaves them out (vitest.config.ts) and this runs them:
// esbuild compiles the modules to CommonJS in a scratch directory, then each
// test runs from the repo root, where the session test reads its fixture.
import { spawnSync } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { buildSync } from 'esbuild'

const web = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const root = path.resolve(web, '../..')
const out = mkdtempSync(path.join(tmpdir(), 'workflow-test-'))
let failed = false
try {
  buildSync({
    entryPoints: ['workflow', 'workflow-view', 'workflow-session'].map(n => path.join(web, 'src/client', n + '.ts')),
    outdir: out,
    format: 'cjs',
    target: 'es2022',
    platform: 'node',
    logLevel: 'error',
  })
  for (const test of ['workflow.test.mjs', 'workflow-session.test.mjs']) {
    const run = spawnSync(process.execPath, [path.join(web, 'test', test), out], { cwd: root, stdio: 'inherit' })
    if (run.status !== 0) {
      console.error(`${test} failed (${run.status ?? run.signal})`)
      failed = true
    }
  }
} finally {
  rmSync(out, { recursive: true, force: true })
}
process.exit(failed ? 1 : 0)
