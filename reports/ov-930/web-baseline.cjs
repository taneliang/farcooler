const ts = require('/Users/e-liang/.npm/_npx/67eb4586ca667318/node_modules/typescript')
const {execFileSync} = require('node:child_process')
const path = require('node:path')
const config = ts.readConfigFile('services/web/tsconfig.client.json', ts.sys.readFile)
const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, path.resolve('services/web'))
const relative = file => path.relative(process.cwd(), file)
const base = 'b736f1d89f59faa7c81b93a7dc92b4fbe3fcbcf3'
const baselineFiles = new Map(['app.ts','wasm.d.ts','workflow.ts','workflow-view.ts'].map(p => 'services/web/src/client/'+p).map(p => [path.resolve(p), execFileSync('git',['show',`${base}:${p}`],{encoding:'utf8'})]))
const newFiles = ['workflow-session.ts'].map(p => path.resolve('services/web/src/client',p))
function diagnostics(baseline) {
 const host = ts.createCompilerHost(parsed.options)
 const read = host.readFile
 host.readFile = file => baseline && baselineFiles.has(path.resolve(file)) ? baselineFiles.get(path.resolve(file)) : read(file)
 const program = ts.createProgram(parsed.fileNames.filter(p => !baseline || !newFiles.includes(p)),parsed.options,host)
 return ts.getPreEmitDiagnostics(program).map(d => `${d.file ? relative(d.file.fileName) : ''}:${d.code} ${ts.flattenDiagnosticMessageText(d.messageText,' ')}`)
}
const before = diagnostics(true), after = diagnostics(false)
console.log('BASELINE', before); console.log('CURRENT', after)
if (JSON.stringify(before) !== JSON.stringify(after)) process.exit(1)
console.log(`Same ${after.length} diagnostics at baseline and current; missing local npm dependencies. Not a full app typecheck pass.`)
