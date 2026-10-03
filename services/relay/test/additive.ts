// What a relay migration may say, checked by reading the SQL rather than by
// grepping it.
//
// WHY THE RULE EXISTS. A deploy runs `wrangler d1 migrations apply` and THEN
// `wrangler deploy` (.github/workflows/relay.yml), so between the two the
// worker still serving requests is the PREVIOUS version, reading and writing
// the NEW schema. It keeps doing so for as long as the deploy takes, and for
// good if the deploy step fails after the migration landed — and a rollback
// to an older worker is the same situation again. So every migration has to
// leave a schema the previous worker can still use:
//
//   - nothing it reads may disappear or change shape: no DROP, no RENAME, no
//     table rebuild (the only way SQLite can change a column's type);
//   - nothing it writes may start failing: a new column is nullable or has a
//     default (the old worker's INSERT never names it), carries no CHECK, and
//     a new UNIQUE index on an old table must include a column this same
//     migration added as nullable, so every row the old worker writes holds
//     a NULL there and SQLite never counts two NULLs as equal;
//   - no data it relies on may vanish or change: no DELETE, and an UPDATE may
//     only fill in columns this same migration added (a backfill);
//   - nothing it deletes may start failing: a new foreign key into a table
//     that existed before this file must say ON DELETE CASCADE, or ON DELETE
//     SET NULL on a nullable column. D1 enforces foreign keys, and the
//     default action is NO ACTION, so once the new worker has written a child
//     row the previous worker's revoke (`DELETE FROM devices …`, index.ts)
//     would fail on the parent. CASCADE takes the child with it and SET NULL
//     detaches it; either way the old worker's DELETE goes through. Leaving
//     the key out entirely is the other way to keep it working. Every key in
//     0001 through 0017 is already ON DELETE CASCADE.
//
// So the grammar below is an ALLOWLIST. A statement is accepted only if it is
// one of the shapes named here; everything else — DROP, ALTER … RENAME,
// ALTER … DROP COLUMN, DELETE, a trigger, a view, a PRAGMA, anything nobody
// thought of — is rejected because it is not on the list, not because a
// pattern happened to match it. Comments and string literals are tokenized
// away first, so the word DROP inside `-- we never DROP` or `DEFAULT 'drop'`
// is not a statement.
//
// 0001 is the baseline: it is read for the tables it creates and not judged.
//
// There is no waiver. A contract-phase change — dropping a column once no
// worker that reads it can still be running — is a deliberate edit to this
// file in the same commit, naming the migration it lets through, so review
// sees the rule bend rather than a migration slip past it.

export interface Migration {
  name: string
  sql: string
}

type Token = { kind: 'word' | 'ident' | 'string' | 'number' | 'punct'; text: string }

/// SQLite's lexical structure, enough of it to know where statements end and
/// which words are keywords: comments dropped, strings and quoted identifiers
/// kept whole.
export function tokenize(sql: string): Token[] {
  const tokens: Token[] = []
  let i = 0
  while (i < sql.length) {
    const c = sql[i]
    if (/\s/.test(c)) { i++; continue }
    if (c === '-' && sql[i + 1] === '-') {
      while (i < sql.length && sql[i] !== '\n') i++
      continue
    }
    if (c === '/' && sql[i + 1] === '*') {
      const end = sql.indexOf('*/', i + 2)
      if (end < 0) throw new Error('unterminated /* comment')
      i = end + 2
      continue
    }
    if (c === "'" || c === '"' || c === '`' || c === '[') {
      const close = c === '[' ? ']' : c
      let j = i + 1
      let text = ''
      for (;;) {
        if (j >= sql.length) throw new Error(`unterminated ${c}`)
        if (sql[j] === close) {
          if (close !== ']' && sql[j + 1] === close) { text += close; j += 2; continue }
          break
        }
        text += sql[j++]
      }
      tokens.push({ kind: c === "'" ? 'string' : 'ident', text })
      i = j + 1
      continue
    }
    if (/[A-Za-z_]/.test(c)) {
      let j = i
      while (j < sql.length && /[A-Za-z0-9_$]/.test(sql[j])) j++
      tokens.push({ kind: 'word', text: sql.slice(i, j) })
      i = j
      continue
    }
    if (/[0-9]/.test(c) || (c === '.' && /[0-9]/.test(sql[i + 1] ?? ''))) {
      let j = i
      while (j < sql.length && /[0-9A-Za-z_.]/.test(sql[j])) j++
      tokens.push({ kind: 'number', text: sql.slice(i, j) })
      i = j
      continue
    }
    tokens.push({ kind: 'punct', text: c })
    i++
  }
  return tokens
}

/// One list of tokens per statement, split on the semicolons that are tokens
/// (never one inside a string or a comment).
export function statements(sql: string): Token[][] {
  const out: Token[][] = []
  let current: Token[] = []
  for (const token of tokenize(sql)) {
    if (token.kind === 'punct' && token.text === ';') {
      if (current.length) out.push(current)
      current = []
    } else {
      current.push(token)
    }
  }
  if (current.length) out.push(current)
  return out
}

class Cursor {
  i = 0
  constructor(readonly tokens: Token[]) {}
  peek(offset = 0): Token | undefined { return this.tokens[this.i + offset] }
  done(): boolean { return this.i >= this.tokens.length }
  isWord(word: string, offset = 0): boolean {
    const t = this.peek(offset)
    return t?.kind === 'word' && t.text.toUpperCase() === word
  }
  isPunct(p: string): boolean {
    const t = this.peek()
    return t?.kind === 'punct' && t.text === p
  }
  word(word: string): boolean {
    if (!this.isWord(word)) return false
    this.i++
    return true
  }
  expect(word: string) {
    if (!this.word(word)) throw new Error(`expected ${word}, found ${this.peek()?.text ?? 'the end'}`)
  }
  /// A table, column or index name: bare or quoted, never schema-qualified.
  name(): string {
    const t = this.peek()
    if (!t || (t.kind !== 'word' && t.kind !== 'ident')) throw new Error(`expected a name, found ${t?.text ?? 'the end'}`)
    this.i++
    if (this.isPunct('.')) throw new Error('a schema-qualified name')
    return t.text.toLowerCase()
  }
  /// The tokens of a parenthesized group, split on its top-level commas.
  group(): Token[][] {
    if (!this.isPunct('(')) throw new Error(`expected (, found ${this.peek()?.text ?? 'the end'}`)
    this.i++
    const items: Token[][] = [[]]
    let depth = 0
    for (;;) {
      const t = this.peek()
      if (!t) throw new Error('unbalanced parentheses')
      this.i++
      if (t.kind === 'punct' && t.text === '(') depth++
      if (t.kind === 'punct' && t.text === ')') {
        if (depth === 0) return items
        depth--
      }
      if (depth === 0 && t.kind === 'punct' && t.text === ',') items.push([])
      else items[items.length - 1].push(t)
    }
  }
  rest(): Token[] {
    const out = this.tokens.slice(this.i)
    this.i = this.tokens.length
    return out
  }
}

const upper = (t: Token) => (t.kind === 'word' ? t.text.toUpperCase() : '')

/// The words of `tokens` outside any parentheses: `DEFAULT (CAST(0 AS
/// INTEGER))` holds no constraint keyword, though it spells AS.
function topLevelWords(tokens: Token[]): string[] {
  return topLevelTokens(tokens).map(upper)
}

/// Every `REFERENCES <parent>` in `tokens` (a column definition or a table
/// constraint) whose parent existed before this file must delete along with
/// it: ON DELETE CASCADE, or ON DELETE SET NULL when `nullable`. See the
/// header for why.
function checkForeignKeys(where: string, tokens: Token[], existing: Set<string>, nullable: boolean) {
  const words = topLevelWords(tokens)
  for (let k = 0; k < words.length; k++) {
    if (words[k] !== 'REFERENCES') continue
    const parent = topLevelTokens(tokens)[k + 1]?.text.toLowerCase() ?? ''
    if (!existing.has(parent)) continue
    let action = ''
    for (let j = k + 1; j < words.length && words[j] !== 'REFERENCES'; j++) {
      if (words[j] === 'ON' && words[j + 1] === 'DELETE') {
        action = words[j + 2] === 'SET' ? `SET ${words[j + 3]}` : words[j + 2]
      }
    }
    if (action === 'CASCADE' || (action === 'SET NULL' && nullable)) continue
    throw new Error(
      `${where} references ${parent} ${action ? `ON DELETE ${action}` : 'with no ON DELETE'}, so the previous worker's DELETE from ${parent} fails once a row points at it: use ON DELETE CASCADE, or SET NULL on a nullable column`,
    )
  }
}

function topLevelTokens(tokens: Token[]): Token[] {
  let depth = 0
  const out: Token[] = []
  for (const t of tokens) {
    if (t.kind === 'punct' && t.text === '(') depth++
    else if (t.kind === 'punct' && t.text === ')') depth--
    else if (depth === 0) out.push(t)
  }
  return out
}

const isNotNull = (words: string[]) => words.some((w, k) => w === 'NOT' && words[k + 1] === 'NULL')

/// The constraint words after a new column's type that the previous worker
/// cannot trip over. NOT NULL is allowed only together with a DEFAULT.
function checkNewColumn(table: string, column: string, spec: Token[]) {
  const words = topLevelWords(spec)
  for (const banned of ['PRIMARY', 'UNIQUE', 'CHECK', 'GENERATED', 'AS']) {
    if (words.includes(banned)) {
      throw new Error(`${table}.${column} is added with ${banned}, which a row the previous worker writes can violate`)
    }
  }
  if (isNotNull(words) && !words.includes('DEFAULT')) {
    throw new Error(`${table}.${column} is added NOT NULL with no DEFAULT, so the previous worker's INSERT fails`)
  }
}

/// Whether a column definition is nullable and has no default — the shape a
/// UNIQUE index needs to be harmless to rows the previous worker writes.
function nullableNoDefault(spec: Token[]): boolean {
  const words = topLevelWords(spec)
  return !words.includes('DEFAULT') && !isNotNull(words)
}

/// Every reason `migrations` (in order, 0001 first) is not additive, as
/// "<file>: <statement start>: <reason>". Empty when every file from the second
/// on is.
export function additiveViolations(migrations: Migration[]): string[] {
  const tables = new Set<string>()
  const problems: string[] = []
  migrations.forEach((migration, index) => {
    const baseline = index === 0
    const createdHere = new Set<string>()
    /// The tables that existed before this file: the ones the previous worker knows.
    const previous = new Set(tables)
    /// table -> columns this migration added to it, and whether each is nullable with no default
    const addedHere = new Map<string, Map<string, boolean>>()
    for (const tokens of statements(migration.sql)) {
      const where = `${migration.name}: ${tokens.slice(0, 4).map(t => t.text).join(' ')}`
      try {
        const c = new Cursor(tokens)
        if (c.word('CREATE')) {
          const unique = c.word('UNIQUE')
          if (c.word('TABLE')) {
            if (unique) throw new Error('CREATE UNIQUE TABLE')
            if (c.word('IF')) { c.expect('NOT'); c.expect('EXISTS') }
            const table = c.name()
            if (tables.has(table)) throw new Error(`table ${table} already exists, so this would replace or skip it`)
            const items = c.group()
            // Foreign keys into tables that existed before this file. A
            // column's own REFERENCES is judged with its own NOT NULL; a table
            // constraint `FOREIGN KEY (a, b) REFERENCES …` with the NOT NULL of
            // the columns it names.
            const notNullColumns = new Set(
              items.filter(item => !['FOREIGN', 'PRIMARY', 'UNIQUE', 'CHECK', 'CONSTRAINT'].includes(upper(item[0] ?? { kind: 'punct', text: '' })))
                .filter(item => isNotNull(topLevelWords(item)))
                .map(item => item[0].text.toLowerCase()),
            )
            for (const item of items) {
              const head = upper(item[0] ?? { kind: 'punct', text: '' })
              const keyColumns = head === 'FOREIGN' || head === 'CONSTRAINT'
                ? (() => {
                    const open = item.findIndex(t => t.kind === 'punct' && t.text === '(')
                    const close = item.findIndex(t => t.kind === 'punct' && t.text === ')')
                    return item.slice(open + 1, close).filter(t => t.kind !== 'punct').map(t => t.text.toLowerCase())
                  })()
                : [item[0]?.text.toLowerCase() ?? '']
              const nullable = !keyColumns.some(column => notNullColumns.has(column))
              checkForeignKeys(`${table}.${keyColumns.join(', ')}`, item, previous, nullable)
            }
            // Only table options may follow the column list: no AS SELECT.
            for (const t of c.rest()) {
              if (!['WITHOUT', 'ROWID', 'STRICT'].includes(upper(t)) && !(t.kind === 'punct' && t.text === ',')) {
                throw new Error(`unexpected ${t.text} after the column list`)
              }
            }
            tables.add(table)
            createdHere.add(table)
            continue
          }
          if (c.word('INDEX')) {
            if (c.word('IF')) { c.expect('NOT'); c.expect('EXISTS') }
            c.name()
            c.expect('ON')
            const table = c.name()
            if (!tables.has(table)) throw new Error(`index on ${table}, which no migration creates`)
            const columns = c.group().map(item => (item[0]?.text ?? '').toLowerCase())
            if (!c.done()) c.expect('WHERE')
            if (unique && !baseline && !createdHere.has(table)) {
              const added = addedHere.get(table)
              if (!columns.some(column => added?.get(column) === true)) {
                throw new Error(
                  `a UNIQUE index on the existing table ${table} must include a column this migration adds as nullable with no default, or the previous worker's writes can violate it`,
                )
              }
            }
            continue
          }
          throw new Error('only CREATE TABLE and CREATE [UNIQUE] INDEX are allowed')
        }
        if (c.word('ALTER')) {
          c.expect('TABLE')
          const table = c.name()
          if (!tables.has(table)) throw new Error(`ALTER of ${table}, which no migration creates`)
          if (!c.word('ADD')) throw new Error(`ALTER TABLE ${c.peek()?.text ?? ''}: only ADD COLUMN is additive`)
          c.word('COLUMN')
          const column = c.name()
          const spec = c.rest()
          checkNewColumn(table, column, spec)
          checkForeignKeys(`${table}.${column}`, spec, previous, !isNotNull(topLevelWords(spec)))
          if (!addedHere.has(table)) addedHere.set(table, new Map())
          addedHere.get(table)!.set(column, nullableNoDefault(spec))
          continue
        }
        if (c.word('UPDATE')) {
          const table = c.name()
          c.expect('SET')
          // SET a = …, b = … [WHERE …]: every target must be a column this
          // migration added, so the update only fills in what is new.
          const added = addedHere.get(table)
          let depth = 0
          let expectTarget = true
          while (!c.done()) {
            const t = c.peek()!
            if (depth === 0 && c.isWord('WHERE')) break
            if (t.kind === 'punct' && t.text === '(') depth++
            if (t.kind === 'punct' && t.text === ')') depth--
            if (expectTarget) {
              const column = t.text.toLowerCase()
              if (!added?.has(column)) throw new Error(`UPDATE of ${table}.${column}, which this migration did not add: a backfill may only fill in new columns`)
              expectTarget = false
            } else if (depth === 0 && t.kind === 'punct' && t.text === ',') {
              expectTarget = true
            }
            c.i++
          }
          continue
        }
        if (c.word('INSERT')) {
          c.expect('INTO')
          const table = c.name()
          if (!createdHere.has(table)) throw new Error(`INSERT into ${table}, which this migration did not create`)
          continue
        }
        throw new Error(`${tokens[0].text.toUpperCase()} is not an additive statement`)
      } catch (error) {
        if (!baseline) problems.push(`${where}: ${(error as Error).message}`)
      }
    }
  })
  return problems
}
