// The page's own markup, stylesheet, shell and Engine view, for engine-check.mjs. The session answers
// `engine.read` with the page the shared core made from a real runner's reply on a large board
// (`ov1486_a_board_of_340_items_loads_in_one_bounded_read`), and keeps what the page sends back.

import { openEngine, type EngineOutcome } from '../../src/client/engine-view'
import { harnessShell, runnerNamed } from './harness-shell'

const fixture = (window as any).__fixture as { workspace: string; page: { words: Record<string, string> } }
const shell = harnessShell({ page: 'engine' })
shell.set({ runner: runnerNamed('Studio Mac') })
shell.command('tools', 'engine', { label: 'Engine' })
for (const id of ['pane', 'needs', 'board', 'plan']) document.getElementById(id)!.hidden = true

const sent: { method: string; args: any }[] = []
let answer: EngineOutcome | 'lost' = { ok: true, message: 'Cancellation accepted.', detail: null }
let up = true
// A press whose answer is held back until the check lets it go.
let held: Promise<void> | null = null
let release = () => {}
const session = {
  async workflow_call(method: string, args: string): Promise<string> {
    sent.push({ method, args: JSON.parse(args) })
    if (method === 'engine.read') return JSON.stringify(fixture.page)
    await held
    if (answer === 'lost') throw new Error('the link dropped')
    return JSON.stringify(answer)
  },
}
let runsOpened = 0
const started = performance.now()
const view = openEngine(document.getElementById('engine') as HTMLDialogElement, () => up ? session : null,
  fixture.workspace, fixture.page.words, () => {}, () => ++runsOpened)
Object.assign(window, {
  engine: {
    sent, started,
    answer: (next: EngineOutcome | 'lost') => { answer = next },
    connection: (next: boolean) => { up = next; view.connection(next) },
    runsOpened: () => runsOpened,
    hold: () => { held = new Promise(resolve => { release = () => { held = null; resolve() } }) },
    release: () => release(),
  },
})
