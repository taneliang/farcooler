// A cancelled run's card, for workflow-cancelled-capture.mjs: the page's own markup, stylesheet and workflow view
// code, over the shared core's "cancelled" golden case instead of a tunnel (ov-1053).

import golden from '../../../../crates/client/src/testdata/workflow-presentation.json'
import { renderWorkflow } from '../../src/client/workflow-view'
import type { WorkflowBoard, WorkflowView } from '../../src/client/workflow'

const card = (golden as any[]).find(c => c.name === 'cancelled').response.card
const view: WorkflowView = { presentation: card, selected: card.runs[0].item, ready: true, busy: false, notice: '', inspection: '' }
// The sheet app.ts opens, built as openWorkflow builds it.
const dialog = document.getElementById('workflow-runs') as HTMLDialogElement
const root = document.createElement('div'); root.className = 'plan-sheet-body'
const bar = document.createElement('div'); bar.className = 'plan-sheet-bar'
const title = document.createElement('h2'); title.textContent = 'Workflow Runs'
const close = document.createElement('button'); close.type = 'button'; close.textContent = 'Close'
bar.append(title, close)
dialog.replaceChildren(bar, root)
dialog.showModal()
renderWorkflow(root, view, { select() {}, refresh: async () => null, retry() {} } as unknown as WorkflowBoard)
;(window as any).done = true
