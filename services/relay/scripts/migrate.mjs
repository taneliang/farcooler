// `npm run migrate -- --env <channel>`: apply migrations/ to one relay's D1.
// Refuses to run without --env; see migrate-args.mjs.

import { spawnSync } from 'node:child_process'

import { migrateArgs } from './migrate-args.mjs'

const args = migrateArgs(process.argv.slice(2))
if (args instanceof Error) {
  console.error(args.message)
  process.exit(2)
}
const result = spawnSync('npx', ['wrangler', ...args], { stdio: 'inherit' })
process.exit(result.status ?? 1)
