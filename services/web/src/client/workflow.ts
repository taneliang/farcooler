// Schema 2 is a local display DTO, never a credential. The authenticated Rust
// Session adapter owns reads, availability and latest-state action validation.
export interface WorkflowAction { kind: string; label: string; guard: string; [key: string]: unknown }
export interface WorkflowStage { id: string; label: string; progression_label: string }
export interface WorkflowForm { action: string; label: string; help: string; submit_label: string; max_bytes: number }
export interface WorkflowRun {
  label: string; phase_label: string; item: string; run: string
  definition_pin: string; policy_pin: string; candidate: string | null
  latest_evidence: { candidate: string; manifest: string; receipt: string; policy_pin: string } | null
  pending_effect: string | null; iteration: number; attempt: number; fence: number | bigint | string
  reworks_remaining: number; stages: WorkflowStage[]; stage: WorkflowStage | null
  gate: { label: string; text: string; principal: string } | null
  forms?: WorkflowForm[]; rework_history?: { reason: string; principal: string; iteration: number }[]
  reasons: { kind: string; label: string; text: string }[]; actions: WorkflowAction[]
}
export interface WorkflowPresentation { schema: 2; workspace: string; epoch: number | bigint | string; revision: number | bigint | string; runs: WorkflowRun[] }
export interface RetainedWorkflow { reinspect?: boolean; id: string; item: string; run: string; label: string }
export interface WorkflowPort {
  pending?(workspace: string): Promise<RetainedWorkflow[]>
  retry?(workspace: string, id: string): Promise<string>
  read(workspace: string): Promise<WorkflowPresentation>
  // Forward the complete supplied descriptor. Rust must authenticate, re-read
  // State and validate the guard; no client-generated Grant or command.
  dispatch(workspace: string, item: string, action: WorkflowAction, reason?: string): Promise<string>
}
export interface WorkflowView {
  retained?: RetainedWorkflow[]; presentation: WorkflowPresentation | null; selected: string | null
  ready: boolean; busy: boolean; notice: string; inspection: string
}
export const WORKFLOW_PENDING = 'Workflow runs are not connected on this runner yet.'

// No polling, ACK or runtime ownership. Refresh on open/reconnect is display I/O.
export class WorkflowBoard {
  private generation = 0
  private selection = 0
  private view: WorkflowView = { presentation: null, selected: null, ready: false, busy: false, notice: '', inspection: '' }
  constructor(readonly workspace: string, private port: () => WorkflowPort | null,
    private show: (view: WorkflowView) => void, private keep: (item: string | null) => void = () => {}) {}
  select(item: string | null): void {
    ++this.selection
    this.view = { ...this.view, selected: item, inspection: '' }
    this.keep(item); this.draw()
  }
  disconnect(): void {
    ++this.generation
    this.view = { ...this.view, ready: false, busy: false, inspection: '', notice: 'Reconnect to refresh workflow runs.' }; this.draw()
  }
  // Return the read ticket so a caller can recheck it after its own await.
  async refresh(): Promise<number | null> {
    if (this.view.busy) return null
    const generation = ++this.generation
    this.view = { ...this.view, ready: false }; this.draw()
    const port = this.port()
    if (!port) { this.view = { ...this.view, notice: WORKFLOW_PENDING }; this.draw(); return null }
    try {
      const presentation = await port.read(this.workspace)
      const retained = await port.pending?.(this.workspace) ?? []
      if (generation !== this.generation) return null
      if (presentation.schema !== 2 || presentation.workspace !== this.workspace) throw new Error('Invalid workflow presentation')
      // A replaced/removed run cannot retain inspection output from its predecessor.
      if (this.view.presentation?.runs.some(old => !presentation.runs.some(run => run.item === old.item && run.run === old.run))) {
        ++this.selection
        this.view = { ...this.view, inspection: '' }
      }
      this.view = { ...this.view, presentation, retained, ready: true, notice: '' }
    } catch {
      if (generation !== this.generation) return null
      this.view = { ...this.view, ready: false, notice: 'Couldn’t refresh workflow runs. Reconnect or try Refresh.' }
    }
    this.draw()
    return generation === this.generation ? generation : null
  }
  async dispatch(item: string, action: WorkflowAction, reason?: string): Promise<void> {
    const port = this.port()
    const card = this.view.presentation?.runs.find(r => r.item === item)
    if (!port || !this.view.ready || this.view.busy || !card?.actions.includes(action)) return
    const generation = this.generation
    const selection = this.selection
    this.view = { ...this.view, busy: true, inspection: '' }; this.draw()
    let failed = false
    try {
      const inspection = await (reason === undefined ? port.dispatch(this.workspace, item, action) : port.dispatch(this.workspace, item, action, reason))
      if (generation === this.generation && selection === this.selection &&
        this.view.selected === item && this.view.presentation?.runs.some(r => r.item === item && r.run === card.run)) {
        this.view = { ...this.view, inspection }
      }
    } catch {
      failed = true
    } finally {
      if (generation === this.generation) {
        this.view = { ...this.view, busy: false, ready: false }; this.draw()
        const current = await this.refresh()
        if (failed && current !== null && current === this.generation) { this.view = { ...this.view, notice: 'The action wasn’t confirmed. Check the refreshed run before trying again.' }; this.draw() }
      }
    }
  }
  async retry(entry: RetainedWorkflow): Promise<void> {
    const port = this.port()
    if (!port?.retry || !this.view.ready || this.view.busy) return
    this.select(entry.item)
    const generation = this.generation, selection = this.selection
    this.view = { ...this.view, busy: true, inspection: '' }; this.draw()
    try {
      const result = await port.retry(this.workspace, entry.id)
      if (generation === this.generation && selection === this.selection && this.view.selected === entry.item &&
        this.view.presentation?.runs.some(r => r.item === entry.item && r.run === entry.run)) {
        this.view = { ...this.view, inspection: result }
      }
    } catch { /* Refresh exposes any remaining bounded explicit retry. */ }
    if (generation !== this.generation) return
    this.view = { ...this.view, busy: false, ready: false }; this.draw()
    await this.refresh()
  }
  private draw(): void { this.show({ ...this.view }) }
}
