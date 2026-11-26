import { readLocal, writeLocal } from './workspace-layout'
import { WorkflowBoard, type WorkflowPort, type WorkflowRun, type WorkflowView } from './workflow'

const el = <K extends keyof HTMLElementTagNameMap>(tag: K, text = '') => {
  const node = document.createElement(tag); node.textContent = text; return node
}
const button = (title: string, act: () => void, disabled = false) => {
  const node = el('button', title); node.type = 'button'; node.disabled = disabled; node.onclick = act; return node
}
// Retain intended focus across disabled busy renders. A user move to another
// control replaces that intent; removed actions fall back to navigation.
const focusMemory = new WeakMap<HTMLElement, { intended: string; fallback: string | null }>()
function keyed<T extends HTMLElement>(node: T, key: string): T {
  node.setAttribute('data-workflow-control', key); return node
}
export function renderWorkflow(root: HTMLElement, view: WorkflowView, board: WorkflowBoard): void {
  const active = root.contains(document.activeElement) ? document.activeElement : null
  const key = active?.getAttribute('data-workflow-control') ?? null
  const remembered = focusMemory.get(root)
  const focused = key && key === remembered?.fallback ? remembered.intended : key
  const content = el('div'); content.className = 'workflow-runs'
  content.append(keyed(button(view.readFailed ? 'Try Again' : 'Refresh', () => void board.refresh(), view.busy), 'refresh'))
  if (view.notice) { const note = el('p', view.notice); note.setAttribute('role', 'status'); content.append(note) }
  for (const retained of view.retained ?? []) content.append(keyed(button(
    `${retained.reinspect ? 'Reinspect Unsubmitted Action' : 'Retry Retained Action'} · ${retained.item} · ${retained.label}`, () => void board.retry(retained), view.busy || !view.ready), `retry:${retained.id}`))
  const selected = view.presentation?.runs.find(r => r.item === view.selected)
  if (view.selected) content.append(keyed(button('All Runs', () => board.select(null)), 'all-runs'))
  if (selected) content.append(runCard(selected, view, board))
  else {
    for (const run of view.presentation?.runs ?? []) {
      content.append(keyed(button(`${run.label} · ${run.item} · ${run.phaseLabel}`, () => board.select(run.item)), JSON.stringify(['run', run.item])))
    }
    if (view.ready && view.presentation?.runs.length === 0) content.append(el('p', 'No workflow runs.'))
    if (view.selected && view.ready) content.append(el('p', 'This run is no longer available.'))
  }
  if (view.inspection) content.append(el('pre', view.inspection))
  root.replaceChildren(content)
  if (focused) {
    const buttons: (HTMLButtonElement | HTMLTextAreaElement)[] = [...root.querySelectorAll('button'), ...root.querySelectorAll('textarea')]
    const intended = buttons.find(b => b.getAttribute('data-workflow-control') === focused)
    const target = intended && !intended.disabled ? intended :
      buttons.find(b => b.getAttribute('data-workflow-control') === 'all-runs' && !b.disabled) ?? buttons.find(b => !b.disabled)
    target?.focus()
    if (intended?.disabled) focusMemory.set(root, { intended: focused, fallback: target?.getAttribute('data-workflow-control') ?? null })
    else focusMemory.delete(root)
  } else focusMemory.delete(root)
}
const drafts = new WeakMap<WorkflowBoard, Map<string, { value: string; start: number; end: number }>>()
const draftScopes = new WeakMap<WorkflowBoard, string>()
function runCard(run: WorkflowRun, view: WorkflowView, board: WorkflowBoard): HTMLElement {
  const card = el('article'); card.append(el('h2', run.page.ticket), el('h3', run.page.outcome), el('p', run.page.reason))
  for (const line of run.page.more) card.append(el('p', line))
  const actions = el('div'); actions.className = 'actions'
  for (const action of run.actions) {
    const key = JSON.stringify(['action', run.item, run.run, action.kind, action.guard])
    const form = run.forms?.find(f => f.action === action.kind)
    const weighted = (node: HTMLButtonElement) => { node.className = `workflow-action-${action.weight.toLowerCase()}`; return node }
    if (form) {
      const scope = draftScopes.get(board)
      const draftKey = scope && `${scope}:reason:${key}`
      let saved = drafts.get(board)
      if (!saved) { saved = new Map(); drafts.set(board, saved) }
      let draft = saved.get(key) ?? { value: '', start: 0, end: 0 }
      const memory = !saved.has(key) && draftKey ? readLocal(draftKey) : ''
      if (memory) {
        try {
          const restored = JSON.parse(memory)
          if (typeof restored?.value === 'string') draft = restored
        } catch {}
      }
      saved.set(key, draft)
      const label = el('label', form.label), input = keyed(el('textarea'), key + ':input')
      input.value = draft.value; input.disabled = !view.ready || view.busy
      input.setAttribute('aria-label', form.label)
      input.oninput = input.onselect = () => {
        draft = { value: input.value, start: input.selectionStart, end: input.selectionEnd }
        saved.set(key, draft)
        if (draftKey) writeLocal(draftKey, draft.value ? JSON.stringify(draft) : '')
      }
      input.onfocus = () => input.setSelectionRange(draft.start, draft.end)
      label.append(input); actions.append(label, el('p', form.help),
        keyed(weighted(button(form.submitLabel, () => void board.dispatch(run.item, action, draft.value), !view.ready || view.busy)), key))
    } else actions.append(keyed(weighted(button(action.label, () => void board.dispatch(run.item, action), !view.ready || view.busy)), key))
  }
  card.append(actions)
  for (const step of run.page.steps) card.append(el('p', `${step.label} · ${step.state}`))
  const details = el('details'); details.append(el('summary', run.page.detailsLabel))
  for (const row of run.page.details) {
    const line = el('p', `${row.title}: ${row.value}`)
    const whole = row.whole
    if (whole) line.append(keyed(button(run.page.copyLabel, () => void navigator.clipboard?.writeText(whole)), `copy:${row.title}`))
    details.append(line)
  }
  card.append(details)
  for (const history of run.reworkHistory ?? []) card.append(el('p', `${history.principal} · ${history.iteration}`), el('pre', history.reason))
  return card
}

// Host installs one getter over the CURRENT authenticated Session. No protocol
// method is invented here; ov-925 attaches the reviewed adapter at this seam.
let currentPort: (runner: string) => WorkflowPort | null = () => null
export function installWorkflowPort(port: (runner: string) => WorkflowPort | null): void { currentPort = port }
export function openWorkflow(dialog: HTMLDialogElement, runner: string, workspace: string, scope: string, closePage: () => void): { close(): void; connection(up: boolean): void } {
  const key = `${scope}:run`
  const scrollKey = `${scope}:scroll`
  const root = el('div'); root.className = 'plan-sheet-body'
  const bar = el('div'); bar.className = 'plan-sheet-bar'
  bar.append(el('h2', 'Workflow Runs'), button('Close', closePage))
  dialog.replaceChildren(bar, root)
  let position = Number(readLocal(scrollKey))
  const board = new WorkflowBoard(workspace, () => currentPort(runner), view => {
    renderWorkflow(root, view, board)
    if (view.ready) root.scrollTop = position
  }, item => writeLocal(key, item ?? ''))
  draftScopes.set(board, scope)
  root.onscroll = () => {
    if (root.scrollHeight <= root.clientHeight && position > 0) return
    position = root.scrollTop
    writeLocal(scrollKey, String(position))
  }
  dialog.setAttribute('open', '')
  board.select(readLocal(key) || null)
  const refresh = () => { if (dialog.open) void board.refresh() }
  window.addEventListener('online', refresh)
  const offline = () => board.disconnect()
  window.addEventListener('offline', offline)
  void board.refresh()
  return {
    close() {
      dialog.removeAttribute('open')
      board.disconnect()
      window.removeEventListener('online', refresh)
      window.removeEventListener('offline', offline)
    },
    connection: up => { if (up) refresh(); else board.disconnect() },
  }
}
