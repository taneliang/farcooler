// Fails a web client build that is too big to ship (plan-763 §1, "CI").
//
//   node scripts/size-gate.mjs dist [--max-total BYTES] [--max-file BYTES]
//   node scripts/size-gate.mjs --self-test
//
// Two limits:
// - What a browser downloads, as brotli, summed over every file in dist/: at
//   most 5.5 MB. A `.br` file counts as it is; anything else is compressed
//   here at quality 11 to measure it. The Go Tailcat module alone is about
//   4.4 MB of that (spike-761).
// - Any one file as stored: at most 24 MiB, under Workers Static Assets' 25 MiB
//   per file. That is why a wasm module is stored only as its .br.

import { mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, relative } from 'node:path'
import { brotliCompressSync, constants } from 'node:zlib'

export const MAX_TOTAL = 5_500_000
export const MAX_FILE = 24 * 1024 * 1024

function* walk(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name)
    if (entry.isDirectory()) yield* walk(path)
    else yield path
  }
}

// The problems with `dir`, and its brotli total.
export function check(dir, maxTotal = MAX_TOTAL, maxFile = MAX_FILE) {
  const problems = []
  let total = 0
  for (const path of walk(dir)) {
    const size = statSync(path).size
    if (size > maxFile) problems.push(`${relative(dir, path)} is ${size} bytes, over the ${maxFile}-byte limit for one asset`)
    if (path.endsWith('.br')) total += size
    else total += brotliCompressSync(readFileSync(path), { params: { [constants.BROTLI_PARAM_QUALITY]: 11 } }).length
  }
  if (total > maxTotal) problems.push(`the client is ${total} bytes as brotli, over the ${maxTotal}-byte limit`)
  return { total, problems }
}

function selfTest() {
  const dir = mkdtempSync(join(tmpdir(), 'size-gate-'))
  let failed = 0
  const expect = (what, got, want) => {
    if (got !== want) {
      failed++
      console.error(`FAIL ${what}: got ${got}, want ${want}`)
    }
  }
  try {
    writeFileSync(join(dir, 'small.js'), 'x'.repeat(1000))
    expect('a small build passes', check(dir).problems.length, 0)
    // Random bytes don't compress, so brotli is about their size.
    writeFileSync(join(dir, 'big.wasm.br'), Buffer.alloc(2_000_000, 7).map((_, i) => (i * 2654435761) >>> 24))
    expect('a .br counts as stored', check(dir).total > 2_000_000, true)
    expect('over the total fails', check(dir, 1_000_000).problems.some(p => p.includes('as brotli')), true)
    expect('over one file fails', check(dir, MAX_TOTAL, 1_500_000).problems.some(p => p.includes('big.wasm.br')), true)
    expect('under both passes', check(dir).problems.length, 0)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
  if (failed) process.exit(1)
  console.log('size-gate self-test: ok')
}

const args = process.argv.slice(2)
if (args[0] === '--self-test') selfTest()
else if (args[0] && !args[0].startsWith('--')) {
  const at = name => {
    const i = args.indexOf(name)
    return i >= 0 ? Number(args[i + 1]) : undefined
  }
  const { total, problems } = check(args[0], at('--max-total') ?? MAX_TOTAL, at('--max-file') ?? MAX_FILE)
  for (const p of problems) console.error(`size-gate: ${p}`)
  if (problems.length) process.exit(1)
  console.log(`size-gate: ${total} bytes as brotli, within limits`)
} else {
  console.error('usage: size-gate.mjs <dist> [--max-total BYTES] [--max-file BYTES] | --self-test')
  process.exit(2)
}
