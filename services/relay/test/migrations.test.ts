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

  /// The same rule checked without the parser: apply the files one at a time
  /// to an empty database and compare the schema after each with the schema
  /// before it. Nothing that existed may go or change; a new column on an old
  /// table is nullable or has a default.
  it('never remove or change a table, column or index', async () => {
    const db = (env as any).SCRATCH as D1Database
    type Column = { name: string; type: string; notnull: number; dflt_value: string | null; pk: number }
    const snapshot = async () => {
      const objects = await db
        .prepare(`SELECT type, name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' AND name NOT LIKE '_cf_%' AND name != 'd1_migrations'`)
        .all<{ type: string; name: string; sql: string | null }>()
      const tables = new Map<string, Map<string, Column>>()
      const indexes = new Map<string, string | null>()
      for (const object of objects.results) {
        if (object.type === 'index') indexes.set(object.name, object.sql)
        if (object.type !== 'table') continue
        const columns = await db.prepare(`PRAGMA table_info("${object.name}")`).all<Column>()
        tables.set(object.name, new Map(columns.results.map(c => [c.name, c])))
      }
      return { tables, indexes }
    }
    let before = await snapshot()
    expect(before.tables.size).toBe(0)
    for (const migration of applied()) {
      await applyD1Migrations(db, [migration])
      const after = await snapshot()
      for (const [table, columns] of before.tables) {
        expect(after.tables.has(table), `${migration.name} removed table ${table}`).toBe(true)
        const now = after.tables.get(table)!
        for (const [name, column] of columns) {
          expect(now.get(name), `${migration.name} changed ${table}.${name}`).toEqual(column)
        }
        for (const [name, column] of now) {
          if (columns.has(name)) continue
          const harmless = column.notnull === 0 || column.dflt_value !== null
          expect(harmless, `${migration.name} added ${table}.${name} NOT NULL with no default`).toBe(true)
        }
      }
      for (const [index, sql] of before.indexes) {
        expect(after.indexes.get(index), `${migration.name} removed or changed index ${index}`).toBe(sql)
      }
      before = after
    }
    expect(before.tables.has('live_activities')).toBe(true)
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
    ['DROP TABLE devices'],
    ['DROP INDEX devices_by_account'],
    ['drop table if exists devices'],
    ['ALTER TABLE devices RENAME TO phones'],
    ['ALTER TABLE devices RENAME COLUMN label TO name'],
    ['ALTER TABLE devices DROP COLUMN label'],
    ['ALTER TABLE devices ADD COLUMN state TEXT NOT NULL'],
    ['ALTER TABLE devices ADD COLUMN state TEXT CHECK (state IS NOT NULL)'],
    ['ALTER TABLE devices ADD COLUMN serial TEXT UNIQUE'],
    ['ALTER TABLE nowhere ADD COLUMN x TEXT'],
    ['CREATE TABLE devices (id TEXT)'],
    ['CREATE TABLE copy AS SELECT * FROM devices'],
    ['CREATE UNIQUE INDEX one_label ON devices (label)'],
    ['DELETE FROM devices'],
    ['UPDATE devices SET label = NULL'],
    ['INSERT INTO devices (id) VALUES (1)'],
    ['CREATE TRIGGER t AFTER INSERT ON devices BEGIN DELETE FROM accounts; END'],
    ['PRAGMA foreign_keys = OFF'],
    ['ALTER TABLE devices ADD COLUMN x TEXT; /* a comment */ DROP TABLE accounts'],
  ])('rejects %s', sql => {
    expect(check(sql)).not.toEqual([])
  })

  it.each([
    ['ALTER TABLE devices ADD COLUMN version TEXT'],
    ['ALTER TABLE devices ADD state TEXT NOT NULL DEFAULT \'verified\''],
    ['-- we never DROP TABLE devices\nALTER TABLE devices ADD COLUMN note TEXT DEFAULT \'drop; table\''],
    ['CREATE TABLE cards (id TEXT PRIMARY KEY); CREATE INDEX cards_by_id ON cards (id); INSERT INTO cards (id) VALUES (\'a\')'],
    ['CREATE INDEX devices_by_label ON devices (label)'],
    ['ALTER TABLE devices ADD COLUMN pulse TEXT; CREATE UNIQUE INDEX devices_pulse ON devices (pulse) WHERE pulse IS NOT NULL'],
    ['ALTER TABLE devices ADD COLUMN kind TEXT; UPDATE devices SET kind = \'phone\' WHERE kind IS NULL'],
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
})
