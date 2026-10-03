// The arguments `npm run migrate` hands wrangler, kept apart from the script
// that runs it so the suite can check them without spawning anything.
//
// The script used to be `wrangler d1 migrations apply farcooler --remote`: the
// STABLE database, by default, one `npm run migrate` from anyone's terminal.
// Now there is no default. The channel has to be named, stable included, and
// the database comes from that channel's `DB` binding in wrangler.toml rather
// than a name typed here, so the two cannot disagree.

export const CHANNELS = ['stable', 'preview', 'canary', 'local']

/// wrangler's argv for `--env <channel>` in `argv`, or an Error saying what is
/// missing. Stable is wrangler's top-level config, so it takes no `--env`.
/// Any other argument is passed through.
export function migrateArgs(argv) {
  const at = argv.indexOf('--env')
  const inline = argv.find(arg => arg.startsWith('--env='))
  const channel = inline ? inline.slice('--env='.length) : at >= 0 ? argv[at + 1] : undefined
  if (!channel) {
    return new Error(`Name the relay to migrate: npm run migrate -- --env <${CHANNELS.join('|')}>`)
  }
  if (!CHANNELS.includes(channel)) {
    return new Error(`There's no relay called "${channel}". Use one of: ${CHANNELS.join(', ')}.`)
  }
  // Everything else goes to wrangler as given. `--remote` is only the default:
  // `--local` (or `--preview`) replaces it, so `--env canary --local` migrates
  // the local copy of canary's database rather than the real one.
  const rest = argv.filter((arg, k) => !(arg === '--env' || arg.startsWith('--env=') || (at >= 0 && k === at + 1)))
  const target = rest.some(arg => ['--local', '--remote', '--preview'].includes(arg)) ? [] : ['--remote']
  const env = channel === 'stable' ? [] : ['--env', channel]
  return ['d1', 'migrations', 'apply', 'DB', ...target, ...env, ...rest]
}
