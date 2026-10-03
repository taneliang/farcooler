import { applyD1Migrations, env } from 'cloudflare:test'
import { describe, expect, it } from 'vitest'

import packageJson from '../package.json?raw'
import { migrateArgs } from '../scripts/migrate-args.mjs'
import { additiveViolations, type Migration } from './additive'
import shipped from './shipped-migrations.json'

// Every migration is additive, every shipped one is frozen, and applying them
// takes a named relay. See `additive.ts` for why the previous worker version
// makes "additive" the rule rather than a preference.

/// The files themselves, byte for byte — not `TEST_MIGRATIONS`, which wrangler
/// has already split into statements and stripped of comments, so a hash of it
/// would miss an edit to a comment and a check of it would trust wrangler's
/// splitter.
const files: Migration[] = Object.entries(
  import.meta.glob('../migrations/*.sql', { query: '?raw', import: 'default', eager: true }) as Record<string, string>,
)
  .map(([path, sql]) => ({ name: path.split('/').pop()!, sql }))
  .sort((a, b) => a.name.localeCompare(b.name))

const applied = () => (env as any).TEST_MIGRATIONS as { name: string; queries: string[] }[]

async function sha256(text: string): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text))
  return [...new Uint8Array(digest)].map(b => b.toString(16).padStart(2, '0')).join('')
}

/// Whether `after` is `before` with one run of text spliced in, naming every
/// column in `added` — the only change ADD COLUMN makes to a table's CREATE
/// text (SQLite puts the new definitions after the last column, ahead of any
/// table constraint). No new column means no change at all.
function onlyInserted(before: string, after: string, added: string[]): boolean {
  if (!added.length) return before === after
  let head = 0
  while (head < before.length && before[head] === after[head]) head++
  let tail = 0
  while (tail < before.length - head && before[before.length - 1 - tail] === after[after.length - 1 - tail]) tail++
  if (head + tail < before.length) return false
  const inserted = after.slice(head, after.length - tail)
  return added.every(name => inserted.includes(name))
}

/// Apply `migrations` to the empty `db` one file at a time and compare the
/// schema after each with the one before, without the parser's help. Nothing
/// that existed may go or change: every table keeps every column as it was
/// and its CREATE text changes only by the columns spliced in (see
/// `onlyInserted`, so a rebuild that merely adds a CHECK or a table UNIQUE
/// shows up here even with every column intact), and every
/// index, trigger and view keeps its text. A new column on an old table is
/// nullable or has a default.
async function schemaRegressions(db: D1Database, migrations: { name: string; queries: string[] }[]) {
  type Column = { name: string; type: string; notnull: number; dflt_value: string | null; pk: number }
  const snapshot = async () => {
    // substr, not LIKE: `_` is a LIKE wildcard. Autoindexes are skipped
    // because the table's own text carries the constraint that makes them.
    const rows = await db
      .prepare(`SELECT type, name, sql FROM sqlite_master WHERE substr(name, 1, 7) != 'sqlite_' AND substr(name, 1, 4) != '_cf_' AND name != 'd1_migrations'`)
      .all<{ type: string; name: string; sql: string | null }>()
    const tables = new Map<string, { sql: string; columns: Map<string, Column> }>()
    const objects = new Map<string, { type: string; sql: string | null }>()
    for (const row of rows.results) {
      if (row.type !== 'table') { objects.set(row.name, { type: row.type, sql: row.sql }); continue }
      const columns = await db.prepare(`PRAGMA table_info("${row.name}")`).all<Column>()
      tables.set(row.name, { sql: row.sql ?? '', columns: new Map(columns.results.map(c => [c.name, c])) })
    }
    return { tables, objects }
  }
  const problems: string[] = []
  let before = await snapshot()
  if (before.tables.size) problems.push('the database was not empty to begin with')
  for (const migration of migrations) {
    await applyD1Migrations(db, [migration])
    const after = await snapshot()
    for (const [table, { sql, columns }] of before.tables) {
      const now = after.tables.get(table)
      if (!now) { problems.push(`${migration.name} removed table ${table}`); continue }
      if (!onlyInserted(sql, now.sql, [...now.columns.keys()].filter(name => !columns.has(name)))) {
        problems.push(`${migration.name} rebuilt ${table}: its CREATE text changed by more than the columns it added`)
      }
      for (const [name, column] of columns) {
        if (JSON.stringify(now.columns.get(name)) !== JSON.stringify(column)) problems.push(`${migration.name} removed or changed ${table}.${name}`)
      }
      for (const [name, column] of now.columns) {
        if (!columns.has(name) && column.notnull !== 0 && column.dflt_value === null) {
          problems.push(`${migration.name} added ${table}.${name} NOT NULL with no default`)
        }
      }
    }
    for (const [name, { type, sql }] of before.objects) {
      if (after.objects.get(name)?.sql !== sql) problems.push(`${migration.name} removed or changed ${type} ${name}`)
    }
    before = after
  }
  return { problems, last: before }
}

describe('migrations', () => {
  /// Without this, a glob that matched nothing would make every check below
  /// pass on an empty list.
  it('reads the same files the suite applies', () => {
    expect(files.length).toBeGreaterThan(16)
    expect(files.map(f => f.name)).toEqual(applied().map(m => m.name))
  })

  it('are all additive from 0002 on', () => {
    expect(additiveViolations(files)).toEqual([])
  })

  /// The same rule checked without the parser: see `schemaRegressions`.
  it('never remove or change a table, column or index', async () => {
    const db = (env as any).SCRATCH as D1Database
    const { problems, last } = await schemaRegressions(db, applied())
    expect(problems).toEqual([])
    // The comparisons ran on something: the tables and indexes 0001 makes.
    expect(last.tables.has('live_activities')).toBe(true)
    expect(last.objects.has('devices_by_account')).toBe(true)
  })

  /// The parser-free check must also be able to say no.
  it.each([
    ['a dropped table', 'DROP TABLE devices', /removed table devices/],
    ['a dropped column', 'DROP INDEX devices_by_label; ALTER TABLE devices DROP COLUMN label', /removed or changed devices.label/],
    ['a dropped index', 'DROP INDEX devices_by_label', /removed or changed index devices_by_label/],
    ['a table UNIQUE added by rebuild', 'CREATE TABLE d2 (id TEXT PRIMARY KEY, label TEXT, UNIQUE (label)); INSERT INTO d2 SELECT * FROM devices; DROP TABLE devices; ALTER TABLE d2 RENAME TO devices; CREATE INDEX devices_by_label ON devices (label)', /rebuilt devices/],
    ['a dropped trigger', 'DROP TRIGGER devices_touch', /removed or changed trigger devices_touch/],
    ['a new NOT NULL column with no default, on an empty table', 'CREATE TABLE d2 (id TEXT PRIMARY KEY, label TEXT); INSERT INTO d2 SELECT id, label FROM devices; DROP TABLE devices; CREATE TABLE devices (id TEXT PRIMARY KEY, label TEXT, extra TEXT NOT NULL); CREATE INDEX devices_by_label ON devices (label)', /added devices.extra NOT NULL with no default/],
    [
      'a rebuild that only adds a constraint',
      "CREATE TABLE d2 (id TEXT PRIMARY KEY, label TEXT CHECK (label != '')); INSERT INTO d2 SELECT * FROM devices; DROP TABLE devices; ALTER TABLE d2 RENAME TO devices; CREATE INDEX devices_by_label ON devices (label)",
      /rebuilt devices/,
    ],
  ])('notices %s', async (_, sql, reason) => {
    const db = (env as any).SCRATCH as D1Database
    const base = { name: '0001_base.sql', queries: ['CREATE TABLE devices (id TEXT PRIMARY KEY, label TEXT)', 'CREATE INDEX devices_by_label ON devices (label)', 'CREATE TRIGGER devices_touch AFTER INSERT ON devices BEGIN SELECT 1; END'] }
    const next = { name: '0002_next.sql', queries: sql.split('; ') }
    const { problems } = await schemaRegressions(db, [base, next])
    expect(problems.join('\n')).toMatch(reason)
  })

  /// D1 never runs a file twice, so an edit to one that a relay already applied
  /// changes nothing there — it only changes a FRESH database, this suite's
  /// included. The suite goes on passing while canary's schema is missing
  /// whatever the edit added, and every query naming it is a 500. Everything
  /// on main has been applied (canary migrates on every push), so every file
  /// is listed in `shipped-migrations.json` with its hash. Change the schema
  /// with a new file instead; a new file adds its own line.
  it('that shipped are frozen', async () => {
    const manifest = shipped as Record<string, string>
    const problems: string[] = []
    for (const [name, hash] of Object.entries(manifest)) {
      const file = files.find(f => f.name === name)
      if (!file) problems.push(`${name} was shipped and is gone`)
      else if ((await sha256(file.sql)) !== hash) problems.push(`${name} was shipped and has been edited: write a new migration instead`)
    }
    for (const file of files) {
      if (!(file.name in manifest)) {
        problems.push(`${file.name} is new: add "${file.name}": "${await sha256(file.sql)}" to test/shipped-migrations.json`)
      }
    }
    expect(problems).toEqual([])
  })
})

/// The checker must be able to say no, or the test above proves nothing.
describe('the additive check', () => {
  const base: Migration = {
    name: '0001_init.sql',
    sql: `CREATE TABLE accounts (id TEXT PRIMARY KEY, email TEXT);
          CREATE TABLE devices (id TEXT PRIMARY KEY, account_id TEXT NOT NULL, label TEXT);`,
  }
  const check = (sql: string) => additiveViolations([base, { name: '0002_next.sql', sql }])

  it.each([
    ['DROP TABLE devices', /DROP is not an additive statement/],
    ['DROP INDEX devices_by_account', /DROP is not an additive statement/],
    ['drop table if exists devices', /DROP is not an additive statement/],
    ['ALTER TABLE devices RENAME TO phones', /only ADD COLUMN is additive/],
    ['ALTER TABLE devices RENAME COLUMN label TO name', /only ADD COLUMN is additive/],
    ['ALTER TABLE devices DROP COLUMN label', /only ADD COLUMN is additive/],
    ['ALTER TABLE devices ADD COLUMN state TEXT NOT NULL', /NOT NULL with no DEFAULT/],
    ['ALTER TABLE devices ADD COLUMN state TEXT CHECK (state IS NOT NULL)', /added with CHECK/],
    ['ALTER TABLE devices ADD COLUMN serial TEXT UNIQUE', /added with UNIQUE/],
    ['ALTER TABLE nowhere ADD COLUMN x TEXT', /which no migration creates/],
    ['CREATE TABLE devices (id TEXT)', /already exists/],
    ['CREATE TABLE copy AS SELECT * FROM devices', /expected \(/],
    ['CREATE UNIQUE INDEX one_label ON devices (label)', /UNIQUE index on the existing table devices/],
    ['DELETE FROM devices', /DELETE is not an additive statement/],
    ['UPDATE devices SET label = NULL', /UPDATE of devices.label/],
    ['INSERT INTO devices (id) VALUES (1)', /INSERT into devices/],
    ['CREATE TRIGGER t AFTER INSERT ON devices BEGIN DELETE FROM accounts; END', /only CREATE TABLE and CREATE \[UNIQUE\] INDEX/],
    ['PRAGMA foreign_keys = OFF', /PRAGMA is not an additive statement/],
    ['ALTER TABLE devices ADD COLUMN x TEXT; /* a comment */ DROP TABLE accounts', /DROP TABLE accounts: DROP is not/],
    // Foreign keys into a table the previous worker deletes from.
    ['CREATE TABLE pulses (id TEXT PRIMARY KEY, device_id TEXT NOT NULL REFERENCES devices(id))', /references devices with no ON DELETE/],
    ['ALTER TABLE devices ADD COLUMN owner TEXT REFERENCES accounts(id)', /references accounts with no ON DELETE/],
    ['ALTER TABLE devices ADD COLUMN owner TEXT REFERENCES accounts(id) ON DELETE RESTRICT', /ON DELETE RESTRICT/],
    ['CREATE TABLE pulses (id TEXT, device_id TEXT NOT NULL REFERENCES devices ON DELETE SET NULL)', /ON DELETE SET NULL/],
    ['CREATE TABLE pulses (id TEXT, device_id TEXT NOT NULL, FOREIGN KEY (device_id) REFERENCES devices(id) ON DELETE SET NULL)', /pulses.device_id references devices/],
    ['CREATE TABLE pulses (id TEXT, device_id TEXT, CONSTRAINT fk FOREIGN KEY (device_id) REFERENCES devices(id) ON UPDATE CASCADE)', /with no ON DELETE/],
  ])('rejects %s', (sql, reason) => {
    expect(check(sql).join('\n')).toMatch(reason)
  })

  it.each([
    ['ALTER TABLE devices ADD COLUMN version TEXT'],
    ['ALTER TABLE devices ADD state TEXT NOT NULL DEFAULT \'verified\''],
    ['-- we never DROP TABLE devices\nALTER TABLE devices ADD COLUMN note TEXT DEFAULT \'drop; table\''],
    ['CREATE TABLE cards (id TEXT PRIMARY KEY); CREATE INDEX cards_by_id ON cards (id); INSERT INTO cards (id) VALUES (\'a\')'],
    ['CREATE INDEX devices_by_label ON devices (label)'],
    ['ALTER TABLE devices ADD COLUMN pulse TEXT; CREATE UNIQUE INDEX devices_pulse ON devices (pulse) WHERE pulse IS NOT NULL'],
    ['ALTER TABLE devices ADD COLUMN kind TEXT; UPDATE devices SET kind = \'phone\' WHERE kind IS NULL'],
    ['ALTER TABLE devices ADD COLUMN n INTEGER NOT NULL DEFAULT (CAST(0 AS INTEGER))'],
    ['CREATE TABLE pulses (id TEXT, device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE)'],
    ['CREATE TABLE pulses (id TEXT, device_id TEXT REFERENCES devices(id) ON UPDATE NO ACTION ON DELETE SET NULL)'],
    ['CREATE TABLE pulses (id TEXT, device_id TEXT, FOREIGN KEY (device_id) REFERENCES devices(id) ON DELETE SET NULL)'],
    ['ALTER TABLE devices ADD COLUMN owner TEXT REFERENCES accounts(id) ON DELETE SET NULL'],
    ['CREATE TABLE a (id TEXT PRIMARY KEY); CREATE TABLE b (a_id TEXT NOT NULL REFERENCES a(id))'],
  ])('accepts %s', sql => {
    expect(check(sql)).toEqual([])
  })
})

describe('npm run migrate', () => {
  /// It was `wrangler d1 migrations apply farcooler --remote`: stable, by
  /// default, from any terminal.
  it('runs the script that requires a named relay', () => {
    expect(JSON.parse(packageJson).scripts.migrate).toBe('node scripts/migrate.mjs')
  })

  it('refuses without --env, or with a relay that does not exist', () => {
    expect(migrateArgs([])).toBeInstanceOf(Error)
    expect(migrateArgs(['--env'])).toBeInstanceOf(Error)
    expect(migrateArgs(['--env', 'prod'])).toBeInstanceOf(Error)
  })

  it('names the database by binding, so wrangler.toml picks it per relay', () => {
    expect(migrateArgs(['--env', 'canary'])).toEqual(['d1', 'migrations', 'apply', 'DB', '--remote', '--env', 'canary'])
    expect(migrateArgs(['--env=preview'])).toEqual(['d1', 'migrations', 'apply', 'DB', '--remote', '--env', 'preview'])
    expect(migrateArgs(['--env', 'stable'])).toEqual(['d1', 'migrations', 'apply', 'DB', '--remote'])
  })

  /// It used to drop them, so `--env canary --local` migrated the REAL canary.
  it('passes other arguments through, and --local replaces --remote', () => {
    expect(migrateArgs(['--env', 'canary', '--local'])).toEqual(['d1', 'migrations', 'apply', 'DB', '--env', 'canary', '--local'])
    expect(migrateArgs(['--local', '--env=stable'])).toEqual(['d1', 'migrations', 'apply', 'DB', '--local'])
    expect(migrateArgs(['--env', 'preview', '--persist-to', 'x'])).toEqual(['d1', 'migrations', 'apply', 'DB', '--remote', '--env', 'preview', '--persist-to', 'x'])
  })
})
