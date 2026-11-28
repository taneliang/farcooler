/// split-check.mjs's page (ov-1495): the page's own two-pane layout, with the
/// shared Rust layout policy, `WorkspacePanes` and `ChatView`, wired as app.ts
/// wires them. Only the runner is scripted.

import init, { workspaceLayout } from 'fc-web'
import { useConversationWords } from '../../src/client/chat-copy'
import { WorkspaceLayout } from '../../src/client/workspace-layout'
import { WorkspacePanes } from '../../src/client/workspace-panes'
import words from '../../../../test/fixtures/conversation-words.json'
import { harnessShell, runnerNamed } from './harness-shell'

useConversationWords(request => JSON.stringify((words as any)[`apple_${JSON.parse(request).conversation.agent.toLowerCase()}`]))

/// What each terminal was sent, as `[terminal, words]`.
const composed: [string, string][] = []
let replies: Promise<void> | null = null
const busy = { id: 'turn:1', ord: 0, rev: 1, turn: null, provisional: false, kind: { Turn: { prompt: 'Find out why the web build fails on CI and fix it.', origin: 'Typed', outcome: null, activity: 'Busy', background_running: 0 } } }
const runner = {
  agentRows: async () => JSON.stringify({ epoch: 1, rev: 1, moreBefore: false, rows: [busy] }),
  agentRowsFollow: () => new Promise<string>(() => {}),
  compose: async (terminal: string, text: string) => {
    composed.push([terminal, text])
    await replies
    return false
  },
  composeImages: async () => false,
  composeLimits: () => JSON.stringify({ images: true, each: 1_000_000, total: 3_000_000 }),
  interrupt: async () => {},
  terminal_writer_supported: () => true,
  terminal_writer: async () => JSON.stringify({ writable: true, label: 'Writer', reason: '', action: null }),
  on_terminal_writer_change: () => {},
}

const terminal = (id: string, title: string) => ({ id, title, sidebarTitle: title, sidebarBadge: null, live: true, lost: false, agent: true, running: true, state: 'running', access: '', attach: true, attachNote: '', rename: null, close: null, recover: null })

await init({ module_or_path: '/farcooler_web.wasm' })
harnessShell({ runner: { ...runnerNamed('studio'), health: 'Connected', runtime: '' }, page: 'terminals' })
const panes = new WorkspacePanes('split-check', 'studio', { max: 7 }, () => runner as any, () => {}, () => {})
const layout = new WorkspaceLayout('split-check', 'studio', workspaceLayout, (view, intent) => {
  panes.render(view, intent === 'open' || view.effect.kind === 'refresh_pane')
  for (const [id, action] of [
    ['view-conversation', 'conversation'], ['side-conversation', 'conversation'],
    ['view-terminal', 'raw_terminal'], ['side-terminal', 'raw_terminal'], ['close-split', 'close_split'],
  ] as const) document.getElementById(id)!.textContent = view.actions.find(a => a.id === action)!.label
}, effect => {
  if (effect.kind === 'focus') panes.focus(effect.slot, false)
})
document.getElementById('close-split')!.onclick = () => layout.run('close_split')
layout.contextFor('delivery', 'terminals', {
  workspace: 'delivery', workspaces: [{ id: 'delivery', name: 'Delivery' }], missingTerminal: false, create: null, start: null, labels: { open: 'New Terminal' },
  terminals: [terminal('seat', 'orchestrator'), terminal('fix', 'Fix the web build'), terminal('plan', 'Plan the release')],
} as any, 'seat')

Object.assign(window, {
  __split: {
    composed,
    run: (command: any) => layout.run(command),
    open: (id: string, slot: 'main' | 'side') => layout.open(id, slot),
    /// Hold the next replies; the returned call lets them land.
    hold() {
      let land!: () => void
      replies = new Promise(done => { land = done })
      ;(window as any).__land = () => { replies = null; land() }
    },
  },
})
