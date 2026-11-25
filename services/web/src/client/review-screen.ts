import type { WorkError } from './work-screen'
import type { WorkflowAction, WorkflowPort } from './workflow'

type Position = {
  comparison: 'branch' | 'local'
  candidate: string
  file: string
  from: number
  line: string
}
export type ReviewRoute = { view: 'directory' | 'file'; path: string } | { view: 'changes'; position?: Position }
export type ReviewLinks = { files?: string; changes?: string; back: string; loading: string }
type Approval = { item: string; action: WorkflowAction }
type Edit = { action: 'review'; reviewed: boolean } | { action: 'comment'; body: string }
type Line = {
  id: string; label: string; kind: string; sign: string; text: string; noNewline: boolean
  oldNumber?: number; newNumber?: number; position: Position; comments: { actor: string; body: string }[]
}
type ChangeView = {
  title: string; branch: string; summary: string; notice?: string; baseWarning?: string; empty: string; refresh: string; workflow: string
  position: Position; marks: unknown
  comparisons: { label: string; position: Position; selected: boolean }[]
  files: { path: string; oldPath?: string; stats?: string; reviewed?: string; position: Position }[]
  diff?: { notice?: string; next?: [string, Position]; hunks: { header: string; lines: Line[] }[] }
  mark?: { label: string; edit: Edit }
  editor?: { label: string; submit: string }
  approval?: Approval
}
type FileView = {
  title: string; path: string; root: string; refresh: string
  parent?: { label: string; path: string }
  entries: { label: string; kind: string; view?: 'directory' | 'file'; path: string }[]
  notice?: string; text?: string
}
export interface ReviewRunner { web_review(workspace: string, ticket: string, request: string): Promise<string> }
export interface ReviewOptions {
  current(): ReviewRunner | null
  navigate(ticket: string, route: ReviewRoute): void
  back(ticket: string): void
  resolve(ticket: string, route: ReviewRoute): void
  failure(error: WorkError): string
  approvalResult(raw: string): string
  workflow: WorkflowPort
  openWorkflow(): void
  storageKey: string
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, text?: string, className?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag)
  if (text !== undefined) node.textContent = text
  if (className) node.className = className
  return node
}

function button(label: string, run: () => void, className = 'quiet'): HTMLButtonElement {
  const node = el('button', label, className)
  node.type = 'button'
  node.onclick = run
  return node
}

function stored(key: string): unknown {
  try { return JSON.parse(sessionStorage.getItem(key) ?? 'null') } catch { return null }
}

function keep(key: string, value: unknown): void {
  try { sessionStorage.setItem(key, JSON.stringify(value)) } catch { /* Private or full storage. */ }
}

export function mountReview(root: HTMLElement, scroll: HTMLElement, options: ReviewOptions) {
  let selection: { workspace: string; ticket: string; route: ReviewRoute } | null = null
  let runner: ReviewRunner | null = null
  let generation = 0
  let scrollKey: string | null = null
  let resolved: ReviewRoute | null = null
  let busy = false
  let links: ReviewLinks | null = null
  const status = el('p', '', 'plan-notice')
  status.setAttribute('role', 'status')
  status.hidden = true

  function report(words: string): void {
    status.textContent = words
    status.hidden = !words
  }

  function remember(): void {
    if (scrollKey !== null) keep(scrollKey, scroll.scrollTop)
  }

  async function read(edit?: Edit, approval?: Approval): Promise<void> {
    if (!selection || !runner || busy) return
    const request = resolved ?? selection.route
    const { workspace, ticket } = selection
    const source = runner
    const token = ++generation
    const current = () => token === generation && source === options.current()
    const key = options.storageKey + JSON.stringify([workspace, ticket])
    const marksKey = key + ':marks:' + (request.view === 'changes' ? request.position?.comparison ?? 'branch' : '')
    remember()
    busy = true
    root.inert = true
    root.setAttribute('aria-busy', 'true')
    report('')
    try {
      const send = (request: object) => source.web_review(workspace, ticket, JSON.stringify(request))
      let outcome = ''
      if (approval && request.view === 'changes') {
        await send({ view: 'verify', candidate: request.position?.candidate })
        if (!current()) return
        await options.workflow.read(workspace)
        if (!current()) return
        outcome = options.approvalResult(await options.workflow.dispatch(workspace, approval.item, approval.action))
        if (!current()) return
      }
      const raw = await send({ ...request, marks: stored(marksKey) ?? {}, edit })
      if (!current()) return
      if (request.view === 'changes') {
        const view = JSON.parse(raw) as ChangeView
        keep(marksKey, view.marks)
        resolved = { view: 'changes', position: view.position }
        renderChanges(view, ticket)
        report(outcome || view.notice || '')
      } else {
        const view = JSON.parse(raw) as FileView
        resolved = { view: request.view, path: view.path }
        renderFiles(view, ticket)
      }
      options.resolve(ticket, resolved)
      scrollKey = key + ':scroll:' + JSON.stringify(resolved)
      const top = stored(scrollKey)
      const line = request.view === 'changes' && request.position?.line
      const row = line && document.getElementById('diff-' + line)
      if (typeof top === 'number') scroll.scrollTop = top
      else if (row && root.contains(row)) row.scrollIntoView({ block: 'center' })
      else scroll.scrollTop = 0
    } catch (error) {
      if (current()) report(options.failure(error as WorkError))
    } finally {
      if (token === generation) {
        busy = false
        root.inert = false
        root.removeAttribute('aria-busy')
      }
    }
  }

  function shell(title: string, ticket: string): HTMLElement {
    const article = el('article', undefined, 'worktree-review')
    const nav = el('nav', undefined, 'board-actions')
    if (links) {
      nav.append(button(links.back, () => options.back(ticket)))
      if (links.files) nav.append(button(links.files, () => options.navigate(ticket, { view: 'directory', path: '' })))
      if (links.changes) nav.append(button(links.changes, () => options.navigate(ticket, { view: 'changes' })))
    }
    article.append(nav)
    if (title) article.append(el('h1', title))
    article.append(status)
    root.replaceChildren(article)
    return article
  }

  function renderFiles(view: FileView, ticket: string): void {
    const article = shell(view.title, ticket)
    const actions = el('div', undefined, 'board-actions')
    if (view.parent) {
      const parent = view.parent
      actions.append(button(parent.label, () => options.navigate(ticket, { view: 'directory', path: parent.path })))
    }
    actions.append(button(view.refresh, () => void read()))
    article.append(el('h2', view.path || view.root), actions)
    if (view.notice) article.append(el('p', view.notice))
    if (view.text != null) article.append(el('pre', view.text, 'review-preview'))
    if (!view.entries.length) return
    const list = el('ul', undefined, 'review-files')
    for (const entry of view.entries) {
      const li = el('li')
      const target = entry.view
      li.append(target ? button(entry.label, () => options.navigate(ticket, { view: target, path: entry.path })) : el('span', entry.label),
        el('span', entry.kind, 'board-meta'))
      list.append(li)
    }
    article.append(list)
  }

  function renderChanges(view: ChangeView, ticket: string): void {
    const article = shell(view.title, ticket)
    const go = (position: Position) => options.navigate(ticket, { view: 'changes', position })
    article.append(el('p', view.branch, 'review-branch'), el('p', view.summary, 'board-meta'))
    if (view.baseWarning) article.append(el('p', view.baseWarning, 'board-meta'))
    const actions = el('div', undefined, 'board-actions')
    for (const comparison of view.comparisons) {
      const choice = button(comparison.label, () => go(comparison.position))
      choice.setAttribute('aria-pressed', String(comparison.selected))
      actions.append(choice)
    }
    actions.append(button(view.refresh, () => void read()), button(view.workflow, options.openWorkflow))
    if (view.approval) actions.append(button(view.approval.action.label, () => void read(undefined, view.approval), ''))
    article.append(actions)
    if (!view.files.length) {
      article.append(el('p', view.empty))
      return
    }
    const columns = el('div', undefined, 'review-columns')
    const files = el('ul', undefined, 'review-files')
    for (const file of view.files) {
      const item = el('li')
      const choice = button(file.path, () => go(file.position))
      choice.setAttribute('aria-current', String(view.position.file === file.path))
      item.append(choice)
      for (const text of [file.oldPath, file.stats, file.reviewed]) if (text) item.append(el('span', text, 'board-meta'))
      files.append(item)
    }
    columns.append(files)
    const diff = el('section', undefined, 'review-diff')
    const heading = el('header', undefined, 'board-actions')
    heading.append(el('h2', view.position.file))
    if (view.mark) heading.append(button(view.mark.label, () => void read(view.mark?.edit)))
    diff.append(heading)
    if (view.diff?.notice) diff.append(el('p', view.diff.notice))
    for (const hunk of view.diff?.hunks ?? []) {
      diff.append(el('div', hunk.header, 'review-hunk'))
      for (const line of hunk.lines) {
        const row = el('div', undefined, 'review-line ' + line.kind)
        row.id = 'diff-' + line.id
        const select = button(
          [line.oldNumber, line.newNumber].map(value => value ?? ' ').join('  '),
          () => go(line.position), 'review-number',
        )
        select.setAttribute('aria-label', line.label)
        select.setAttribute('aria-pressed', String(view.position.line === line.id))
        row.append(select, el('span', line.sign), el('code', line.text + (line.noNewline ? ' ␤' : '')))
        diff.append(row)
        for (const comment of line.comments) {
          const note = el('blockquote', undefined, 'review-note')
          note.append(el('strong', comment.actor), el('p', comment.body))
          diff.append(note)
        }
        if (line.id === view.position.line && view.editor) {
          const form = el('form', undefined, 'review-note')
          const label = el('label', view.editor.label)
          const input = el('textarea')
          input.required = true
          label.append(input)
          const submit = el('button', view.editor.submit)
          form.append(label, submit)
          form.onsubmit = event => {
            event.preventDefault()
            void read({ action: 'comment', body: input.value })
          }
          diff.append(form)
        }
      }
    }
    if (view.diff?.next) {
      const [label, position] = view.diff.next
      diff.append(button(label, () => go(position)))
    }
    columns.append(diff)
    article.append(columns)
  }

  function show(workspace: string, ticket: string, route: ReviewRoute, labels: ReviewLinks | null): void {
    const next = { workspace, ticket, route }
    const live = options.current()
    if (live === runner && (JSON.stringify(next) === JSON.stringify(selection) ||
      selection?.workspace === workspace && selection.ticket === ticket && JSON.stringify(resolved) === JSON.stringify(route))) return
    remember()
    scrollKey = null
    resolved = null
    generation++
    busy = false
    selection = next
    runner = live
    links = labels
    shell('', ticket)
    report(live ? links?.loading ?? '' : options.failure(new Error('closed')))
    void read()
  }

  function close(): void {
    if (!selection) return
    remember()
    scrollKey = null
    generation++
    selection = null
    root.replaceChildren()
    root.inert = false
    root.removeAttribute('aria-busy')
  }

  return { show, close, remember }
}
