// The web client's build (build.sh): src/client into dist/.
//
//   dist/index.html        the shell, naming each hashed file below
//   dist/version.json      { build, channel, files }, which an open tab polls
//   dist/a/<name>.<hash>.<ext>
//   dist/a/<name>.<hash>.wasm.br   a wasm module, as brotli only (src/assets.ts)
//
// A hash is the first 16 hex digits of the file's SHA-256, so a name never
// changes content and the Worker can cache it for a year.
//
// The wasm modules come from WEB_WASM_DIR, which crates/web/build.sh fills
// (built for the same --channel): farcooler_web.wasm and its wasm-bindgen glue
// farcooler_web.js, tailcat.wasm and Go's wasm_exec.js. The two .wasm files are
// hashed first so app.js can name them (__WASM__); the two .js files are
// bundled into app.js, so the page loads nothing it didn't name at boot.

import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
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

const wasmDir = process.env.WEB_WASM_DIR
const WASM = ['farcooler_web', 'tailcat']
const GLUE = ['farcooler_web.js', 'wasm_exec.js']
const missing = wasmDir ? [...WASM.map(n => `${n}.wasm`), ...GLUE].filter(f => !existsSync(join(wasmDir, f))) : ['WEB_WASM_DIR']
if (missing.length) {
  console.error(`build: missing ${missing.join(', ')}; run crates/web/build.sh --channel ${channel} --out <dir> and set WEB_WASM_DIR=<dir>`)
  process.exit(2)
}
for (const name of WASM) {
  const raw = readFileSync(join(wasmDir, `${name}.wasm`))
  const file = `a/${name}.${hashOf(raw)}.wasm`
  const br = brotliCompressSync(raw, { params: { [constants.BROTLI_PARAM_QUALITY]: 11, [constants.BROTLI_PARAM_SIZE_HINT]: raw.length } })
  writeFileSync(join(out, `${file}.br`), br)
  files[`${name}.wasm`] = `/${file}`
}

const bundled = await build({
  entryPoints: ['src/client/app.ts'],
  bundle: true,
  minify: true,
  format: 'esm',
  target: 'es2022',
  write: false,
  alias: { 'fc-web': resolve(wasmDir, 'farcooler_web.js'), 'fc-go-runtime': resolve(wasmDir, 'wasm_exec.js') },
  define: {
    __BUILD__: JSON.stringify(buildId),
    __CHANNEL__: JSON.stringify(channel),
    __WASM__: JSON.stringify({ web: files['farcooler_web.wasm'], tailcat: files['tailcat.wasm'] }),
  },
})
emit('app', 'js', bundled.outputFiles[0].contents)
// xterm's own stylesheet first, so the page's rules come after it.
emit('app', 'css', Buffer.concat([readFileSync('node_modules/@xterm/xterm/css/xterm.css'), readFileSync('src/client/app.css')]))

let html = readFileSync('src/client/index.html', 'utf8')
html = html.replace(/\{\{([a-z0-9_.-]+)\}\}/g, (_, key) => {
  if (!files[key]) throw new Error(`index.html names ${key}, which the build didn't make`)
  return files[key]
})
writeFileSync(join(out, 'index.html'), html)
writeFileSync(join(out, 'version.json'), JSON.stringify({ build: buildId, channel, files }, null, 2) + '\n')
console.log(`built ${channel} ${buildId}: ${Object.values(files).join(' ')}`)
