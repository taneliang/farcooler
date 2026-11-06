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
function keyed(node: HTMLButtonElement, key: string): HTMLButtonElement {
  node.setAttribute('data-workflow-control', key); return node
}
export function renderWorkflow(root: HTMLElement, view: WorkflowView, board: WorkflowBoard): void {
  const active = root.contains(document.activeElement) ? document.activeElement : null
  const key = active?.getAttribute('data-workflow-control') ?? null
  const remembered = focusMemory.get(root)
  const focused = key && key === remembered?.fallback ? remembered.intended : key
  const content = el('div'); content.className = 'workflow-runs'
  content.append(keyed(button('Refresh', () => void board.refresh(), view.busy), 'refresh'))
  if (view.notice) { const note = el('p', view.notice); note.setAttribute('role', 'status'); content.append(note) }
  const selected = view.presentation?.runs.find(r => r.item === view.selected)
  if (view.selected) content.append(keyed(button('All Runs', () => board.select(null)), 'all-runs'))
  if (selected) content.append(runCard(selected, view, board))
  else {
    for (const run of view.presentation?.runs ?? []) {
      content.append(keyed(button(`${run.label} · ${run.item} · ${run.phase_label}`, () => board.select(run.item)), JSON.stringify(['run', run.item])))
    }
    if (view.ready && view.presentation?.runs.length === 0) content.append(el('p', 'No workflow runs.'))
    if (view.selected && view.ready) content.append(el('p', 'This run is no longer available.'))
  }
  if (view.inspection) content.append(el('pre', view.inspection))
  root.replaceChildren(content)
  if (focused) {
    const buttons = Array.from(root.querySelectorAll('button'))
    const intended = buttons.find(b => b.getAttribute('data-workflow-control') === focused)
    const target = intended && !intended.disabled ? intended :
      buttons.find(b => b.getAttribute('data-workflow-control') === 'all-runs' && !b.disabled) ?? buttons.find(b => !b.disabled)
    target?.focus()
    if (intended?.disabled) focusMemory.set(root, { intended: focused, fallback: target?.getAttribute('data-workflow-control') ?? null })
    else focusMemory.delete(root)
  } else focusMemory.delete(root)
}
function runCard(run: WorkflowRun, view: WorkflowView, board: WorkflowBoard): HTMLElement {
  const card = el('article'); card.append(el('h2', run.label), el('p', run.phase_label))
  const fields = el('dl')
  const field = (name: string, value: string | number | bigint | null) => {
    if (value !== null) fields.append(el('dt', name), el('dd', String(value)))
  }
  field('Item', run.item); field('Run', run.run); field('Definition', run.definition_pin); field('Policy', run.policy_pin)
  field('Candidate', run.candidate); field('Pending Effect', run.pending_effect)
  field('Iteration', run.iteration); field('Attempt', run.attempt); field('Fence', run.fence); field('Reworks Remaining', run.reworks_remaining)
  for (const [name, value] of Object.entries(run.latest_evidence ?? {})) field(`Evidence ${name}`, value)
  card.append(fields)
  if (run.stage) card.append(el('h3', run.stage.label), el('p', run.stage.progression_label))
  if (run.gate) card.append(el('h3', run.gate.label), el('p', run.gate.text), el('p', run.gate.principal))
  for (const stage of run.stages) card.append(el('p', `${stage.id} · ${stage.label} · ${stage.progression_label}`))
  for (const reason of run.reasons) card.append(el('h3', reason.label), el('p', reason.text))
  const actions = el('div'); actions.className = 'actions'
  for (const action of run.actions) actions.append(keyed(button(action.label, () => void board.dispatch(run.item, action), !view.ready || view.busy), JSON.stringify(['action', run.item, run.run, action.kind, action.guard])))
  card.append(actions); return card
}

// Host installs one getter over the CURRENT authenticated Session. No protocol
// method is invented here; ov-925 attaches the reviewed adapter at this seam.
let currentPort: (runner: string) => WorkflowPort | null = () => null
export function installWorkflowPort(port: (runner: string) => WorkflowPort | null): void { currentPort = port }
export function openWorkflow(dialog: HTMLDialogElement, runner: string, workspace: string): { close(): void; connection(up: boolean): void } {
  const key = `workflow.run.${runner}.${workspace}`
  const root = el('div'); root.className = 'plan-sheet-body'
  const bar = el('div'); bar.className = 'plan-sheet-bar'
  bar.append(el('h2', 'Workflow Runs'), button('Close', () => dialog.close()))
  dialog.replaceChildren(bar, root)
  const board = new WorkflowBoard(workspace, () => currentPort(runner), view => renderWorkflow(root, view, board), item => {
    try { if (item) localStorage.setItem(key, item); else localStorage.removeItem(key) } catch { /* private storage */ }
  })
  try { board.select(localStorage.getItem(key)) } catch { board.select(null) }
  const refresh = () => { if (dialog.open) void board.refresh() }
  window.addEventListener('online', refresh)
  const offline = () => board.disconnect()
  window.addEventListener('offline', offline)
  const stop = () => { board.disconnect(); window.removeEventListener('online', refresh); window.removeEventListener('offline', offline); dialog.removeEventListener('close', stop) }
  dialog.addEventListener('close', stop)
  dialog.showModal(); void board.refresh()
  return { close: () => { dialog.close(); stop() }, connection: up => { if (up) refresh(); else board.disconnect() } }
}
