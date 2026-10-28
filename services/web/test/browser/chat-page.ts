/// The conversation view alone, on a page of the shell's own layout, with a
/// scripted runner (ov-776): what test/browser/chat-check.mjs drives in a real
/// browser. Nothing here is shipped; `FakeRunner` plays the three calls the
/// wasm module gives the view.

import { ChatView } from '../../src/client/chat'
import type { Row } from '../../src/client/chat-rows'

const EPOCH = 1_700_000_000_000_001

const row = (id: string, ord: number, rev: number, kind: Row['kind'], turn: string | null = null): Row => ({ id, ord, rev, turn, provisional: false, kind })
const turn = (id: string, ord: number, rev: number, prompt: string, extra: object = {}) =>
  row(id, ord, rev, { Turn: { prompt, origin: 'Typed', outcome: null, activity: 'Idle', background_running: 0, ...extra } })

export class FakeRunner {
  rows: Row[] = []
  rev = 0
  composed: string[] = []
  interrupts = 0
  failing: string | null = null
  private parked: (() => void)[] = []
  private log: { rev: number; kind: 'insert' | 'update'; row: Row }[] = []

  put(r: Omit<Row, 'ord' | 'rev'> & { ord?: number }): void {
    this.rev++
    const at = this.rows.findIndex(x => x.id === r.id)
    const full = { ...r, ord: at >= 0 ? this.rows[at].ord : this.rows.length, rev: this.rev } as Row
    if (at >= 0) this.rows[at] = full
    else this.rows.push(full)
    this.log.push({ rev: this.rev, kind: at >= 0 ? 'update' : 'insert', row: full })
    for (const wake of this.parked.splice(0)) wake()
  }

  async agentRows(_t: string, _before: number | undefined, _limit: number): Promise<string> {
    return JSON.stringify({ epoch: EPOCH, rev: this.rev, moreBefore: false, rows: this.rows })
  }

  async agentRowsFollow(_t: string, epoch: number, after: number, _wait: number): Promise<string> {
    const answer = () => ({ epoch: EPOCH, rev: this.rev, reset: epoch !== EPOCH, changes: this.log.filter(c => c.rev > after).map(c => ({ kind: c.kind, id: c.row.id, rev: c.rev, row: c.row })) })
    if (this.rev > after) return JSON.stringify(answer())
    await new Promise<void>(wake => this.parked.push(wake))
    return JSON.stringify(answer())
  }

  async compose(_t: string, text: string): Promise<boolean> {
    if (this.failing) throw new Error(this.failing)
    this.composed.push(text)
    // The Mac would show it as the person's turn, at once.
    this.put(turn(`turn:${this.rev + 1}`, 0, 0, text, { activity: 'Busy' }) as any)
    return false
  }

  async interrupt(): Promise<void> {
    this.interrupts++
    const open = [...this.rows].reverse().find(r => Object.keys(r.kind)[0] === 'Turn')
    if (open) this.put({ ...open, kind: { Turn: { ...open.kind.Turn, outcome: 'Interrupted', activity: 'Idle' } } })
  }
}

/// A morning's work: a request, reading and editing, a subagent, a question
/// asked and answered, a task list, a queued follow-up, and the agent still
/// working.
export function conversation(r: FakeRunner): void {
  r.put(turn('turn:1', 0, 0, 'The web build fails on CI after the chat change. Find out why and fix it without touching the wasm crate.\n\nIt only fails on Linux.', { outcome: 'Finished', duration_ms: 142_000 }) as any)
  r.put(row('think:1', 0, 0, { Thinking: { started_ms: 1000, ended_ms: 5000 } }, 'turn:1') as any)
  r.put(row('p:1', 0, 0, { Prose: { text: 'I’ll start with the CI log, then compare the two builds.', conclusion: false } }, 'turn:1') as any)
  const tool = (id: string, name: string, summary: string, extra: object = {}) => row(id, 0, 0, { Tool: { name, status: 'Done', summary, diff: [], ...extra } }, 'turn:1') as any
  r.put(tool('tool:1', 'Bash', 'Read the failing job’s log', { input: 'command: gh run view --log-failed', result: 'Error: Cannot find module ./chat.css' }))
  r.put(tool('tool:2', 'Read', '/Users/me/overnight/services/web/scripts/build.mjs'))
  r.put(tool('tool:3', 'Grep', 'chat.css'))
  r.put(row('sub:1', 0, 0, { Subagent: { tool_use_id: 'x', agent_type: 'general-purpose', description: 'Map every place the stylesheet is read', background: false, status: 'Completed', tool_count: 14, current_action: '' } }, 'turn:1') as any)
  r.put(
    row('p:2', 0, 0, {
      Prose: {
        text: 'The build reads the stylesheet by a path that only exists on this machine.\n\n- `build.mjs` joined the files with a relative path\n- The Linux runner starts from the repo root\n- **Fix:** resolve it from the script’s own folder\n\n```\nconst css = join(here, "../src/client/chat.css")\n```',
        conclusion: false,
      },
    }, 'turn:1') as any,
  )
  r.put(tool('tool:4', 'Edit', 'services/web/scripts/build.mjs', { diff: [{ old_start: 1, old_lines: 1, new_start: 1, new_lines: 1, lines: ['-readFileSync("src/client/chat.css")', '+readFileSync(join(here, "../src/client/chat.css"))'] }] }))
  r.put(row('ask:1', 0, 0, { Ask: { kind: 'Permission', text: 'Bash: npm run build', tool: 'Bash', answered: true, answered_by: 'iPhone', questions: [] } }, 'turn:1') as any)
  r.put(row('tasks:1', 0, 0, { Tasks: { items: [{ subject: 'Reproduce the failure', status: 'Completed' }, { subject: 'Fix the path', status: 'Completed' }, { subject: 'Run the web tests', status: 'InProgress' }, { subject: 'Push', status: 'Pending' }] } }, 'turn:1') as any)
  r.put(turn('turn:2', 0, 0, 'Also check the size gate while you are there.', { activity: 'Busy' }) as any)
  r.put(row('think:2', 0, 0, { Thinking: { started_ms: 1000, ended_ms: null } }, 'turn:2') as any)
  r.put(row('tool:5', 0, 0, { Tool: { name: 'Bash', status: 'Running', summary: 'node scripts/size-gate.mjs dist', diff: [] } }, 'turn:2') as any)
  r.put(row('queued:1', 0, 0, { Queued: { text: 'After that, run the whole suite.', state: 'Waiting' } }, 'turn:2') as any)
  r.put(row('hint:composer', 0, 0, { Hint: { text: '' } }) as any)
}

const runner = new FakeRunner()
conversation(runner)

const host = document.createElement('div')
host.className = 'terminal-host'
document.getElementById('terminal')!.append(host)
const view = new ChatView(host, 'studio')
;(window as any).__chat = { runner, view }
void view.attach(runner as any, 'term-1', () => {}).then(() => view.show(true))
