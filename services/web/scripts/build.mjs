// The web client's build (build.sh): src/client into dist/.
//
//   dist/index.html        the shell, naming each hashed file below
//   dist/version.json      { build, channel, files }, which an open tab polls
//   dist/a/<name>.<hash>.<ext>
//   dist/a/<name>.<hash>.wasm.br   a wasm module, as brotli only (src/assets.ts)
//
// A hash is the first 16 hex digits of the file's SHA-256, so a name never
// changes content and the Worker can cache it for a year. Wasm modules come
// from WEB_WASM_DIR when it is set (later steps of slice 1 put the Rust and Go
// modules there); none is needed today.

import { createHash } from 'node:crypto'
import { mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { basename, join } from 'node:path'
import { brotliCompressSync, constants } from 'node:zlib'

import { build } from 'esbuild'

const args = process.argv.slice(2)
const flag = name => {
  const at = args.indexOf(name)
  return at >= 0 ? args[at + 1] : undefined
}
const channel = flag('--channel') ?? 'canary'
if (channel !== 'canary' && channel !== 'stable') {
  console.error(`build: --channel is canary or stable, not ${channel}`)
  process.exit(2)
}
const buildId = flag('--build') ?? process.env.GITHUB_SHA ?? 'dev'
if (!/^[A-Za-z0-9._-]{1,64}$/.test(buildId)) {
  console.error('build: --build is a commit or a short name')
  process.exit(2)
}

const out = 'dist'
rmSync(out, { recursive: true, force: true })
mkdirSync(join(out, 'a'), { recursive: true })

const files = {}
const hashOf = bytes => createHash('sha256').update(bytes).digest('hex').slice(0, 16)
function emit(name, ext, bytes) {
  const file = `a/${name}.${hashOf(bytes)}.${ext}`
  writeFileSync(join(out, file), bytes)
  files[`${name}.${ext}`] = `/${file}`
}

const bundled = await build({
  entryPoints: ['src/client/app.ts'],
  bundle: true,
  minify: true,
  format: 'esm',
  target: 'es2022',
  write: false,
  define: { __BUILD__: JSON.stringify(buildId), __CHANNEL__: JSON.stringify(channel) },
})
emit('app', 'js', bundled.outputFiles[0].contents)
emit('app', 'css', readFileSync('src/client/app.css'))

const wasmDir = process.env.WEB_WASM_DIR
if (wasmDir) {
  for (const entry of readdirSync(wasmDir).filter(f => f.endsWith('.wasm'))) {
    const raw = readFileSync(join(wasmDir, entry))
    const name = basename(entry, '.wasm')
    const file = `a/${name}.${hashOf(raw)}.wasm`
    const br = brotliCompressSync(raw, { params: { [constants.BROTLI_PARAM_QUALITY]: 11, [constants.BROTLI_PARAM_SIZE_HINT]: raw.length } })
    writeFileSync(join(out, `${file}.br`), br)
    files[`${name}.wasm`] = `/${file}`
  }
}

let html = readFileSync('src/client/index.html', 'utf8')
html = html.replace(/\{\{([a-z0-9_.-]+)\}\}/g, (_, key) => {
  if (!files[key]) throw new Error(`index.html names ${key}, which the build didn't make`)
  return files[key]
})
writeFileSync(join(out, 'index.html'), html)
writeFileSync(join(out, 'version.json'), JSON.stringify({ build: buildId, channel, files }, null, 2) + '\n')
console.log(`built ${channel} ${buildId}: ${Object.values(files).join(' ')}`)
