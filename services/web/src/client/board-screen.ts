import type { WorkError } from './work-screen'

type Card = { id: string; key: string; title: string; unread?: string }
type Mark = { label: string; through: number }
type Board = {
  title: string
  refresh: string
  empty: string
  unread?: string
  create?: string
  markAll?: Mark
  columns: { title: string; rows: Card[] }[]
}
type Field = { name: string; label: string; multiline: boolean; required?: boolean; value: string }
type Form = { heading: string; submit: string; cancel: string; fields: Field[] }
type Link = { id: string; label: string }
type Ticket = {
  task: Card
  status: string
  version: string
  back: string
  activity: string
  edit?: Form
  start?: { label: string; guard: string }
  mark: number | null
  agent?: Link
  worktree?: Link
  sections: { label: string; lines?: string[]; criteria?: { text: string; met: boolean }[] }[]
}
type Runner = {
  board(workspace: string, repository: string): Promise<string>
  board_ticket(workspace: string, task: string): Promise<string>
  board_intent(workspace: string, intent: string): Promise<string>
}

interface Options {
  root: HTMLElement
  dialog: HTMLDialogElement
  current(): Runner | null
  create(): string
  failure(error: WorkError): string
  href(id?: string, kind?: 'terminal' | 'worktree'): URL
  openTicket(id?: string): void
  openAgent(id: string): void
  openWorktree(id: string): void
  openActivity(card: Card): void
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, text?: string, className?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag)
  if (text !== undefined) node.textContent = text
  if (className) node.className = className
  return node
}

function button(label: string, run: () => void, quiet = false): HTMLButtonElement {
  const node = el('button', label, quiet ? 'quiet' : undefined)
  node.type = 'button'
  node.onclick = run
  return node
}

function anchor(text: string, url: URL, run: () => void): HTMLAnchorElement {
  const node = el('a', text)
  node.href = url.href
  node.onclick = event => {
    if (event.button || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return
    event.preventDefault()
    run()
  }
  return node
}

export function mountBoard(options: Options) {
  const { root, dialog } = options
  const body = el('div')
  const notice = el('p', '', 'plan-notice')
  notice.setAttribute('role', 'status')
  notice.hidden = true
  root.replaceChildren(notice, body)
  let workspace: string | null = null
  let repository: string | null = null
  let task: string | null = null
  let front = false
  let busy = false
  let timer = 0
  let revision = 0
  let reading = false
  let last = ''
  let markedThrough: number | null = null
  dialog.oncancel = event => { if (busy) event.preventDefault() }

  function select(id: string | null): void {
    if (dialog.open) dialog.close()
    task = id
    last = ''
    markedThrough = null
    revision++
    body.replaceChildren()
    root.scrollTop = 0
    report('')
    void refresh()
  }

  function report(words: string, target = notice): void {
    target.textContent = words
    target.hidden = !words
  }

  async function refresh(poll = false): Promise<void> {
    const runner = options.current()
    if (!front || !workspace || !repository || busy || (poll && (reading || dialog.open))) return
    if (!runner) {
      report(options.failure(new Error('closed')))
      return
    }
    const version = ++revision
    reading = true
    body.setAttribute('aria-busy', 'true')
    try {
      const raw = task ? await runner.board_ticket(workspace, task) : await runner.board(workspace, repository)
      if (version !== revision || runner !== options.current()) return
      report('')
      if (raw === last) return
      const focused = body.contains(document.activeElement) ? (document.activeElement as HTMLElement) : null
      const href = focused?.getAttribute('href')
      const label = focused?.textContent
      last = raw
      if (task) {
        const ticket = JSON.parse(raw) as Ticket
        renderTicket(ticket)
        if (ticket.mark !== null && ticket.mark !== markedThrough) {
          markedThrough = ticket.mark
          void runner.board_intent(workspace, JSON.stringify({ action: 'mark', task, through: ticket.mark }))
            .catch(error => {
              if (version !== revision) return
              markedThrough = null
              last = ''
              report(options.failure(error))
            })
        }
      } else renderBoard(JSON.parse(raw) as Board)
      if (focused) [...body.querySelectorAll<HTMLElement>('a,button')].find(node =>
        href ? node.getAttribute('href') === href : node.textContent === label)?.focus()
    } catch (e) {
      if (version === revision) report(options.failure(e as WorkError))
    } finally {
      if (version === revision) {
        reading = false
        body.removeAttribute('aria-busy')
      }
    }
  }

  async function send(intent: object, onSuccess: (reply: string) => void, errorTarget = notice): Promise<void> {
    const runner = options.current()
    if (!front || !workspace || !runner || busy) return
    const destination = { workspace, task }
    const stillHere = () => front && workspace === destination.workspace && task === destination.task
    busy = true
    revision++
    reading = false
    body.setAttribute('aria-busy', 'true')
    report('', errorTarget)
    let reply: string
    try {
      reply = await runner.board_intent(workspace, JSON.stringify(intent))
    } catch (e) {
      if (stillHere()) report(options.failure(e as WorkError), errorTarget)
      return
    } finally {
      busy = false
      body.removeAttribute('aria-busy')
      if (!stillHere()) void refresh()
    }
    if (stillHere()) onSuccess(reply)
  }

  function renderBoard(view: Board): void {
    const toolbar = el('header', undefined, 'board-toolbar')
    toolbar.append(el('h1', view.title))
    if (view.unread) toolbar.append(el('span', view.unread, 'board-meta'))
    const actions = el('div', undefined, 'board-actions')
    if (view.create) actions.append(button(view.create, () => edit(JSON.parse(options.create()) as Form)))
    if (view.markAll) {
      const mark = view.markAll
      actions.append(button(mark.label, () => void send({ action: 'mark', through: mark.through }, () => void refresh()), true))
    }
    actions.append(button(view.refresh, () => void refresh(), true))
    toolbar.append(actions)
    const columns = el('div', undefined, 'board-columns')
    for (const column of view.columns) {
      const section = el('section', undefined, 'board-column')
      const heading = el('h2', column.title)
      heading.append(el('span', String(column.rows.length), 'board-meta'))
      section.append(heading)
      const list = el('ul')
      for (const row of column.rows) {
        const item = el('li')
        const link = anchor('', options.href(row.id), () => options.openTicket(row.id))
        link.append(el('span', row.key, 'board-key'), el('strong', row.title))
        if (row.unread) link.append(el('span', row.unread, 'board-unread'))
        item.append(link)
        list.append(item)
      }
      section.append(column.rows.length ? list : el('p', view.empty, 'board-meta'))
      columns.append(section)
    }
    body.replaceChildren(toolbar, columns)
  }

  function renderTicket(view: Ticket): void {
    const content = el('article', undefined, 'board-ticket')
    const back = anchor(view.back, options.href(), () => options.openTicket())
    const heading = el('h1', view.task.title)
    const toolbar = el('div', undefined, 'board-actions')
    toolbar.append(button(view.activity, () => options.openActivity(view.task), true))
    if (view.edit) toolbar.append(button(view.edit.heading, () => edit(view.edit!, view), true))
    if (view.start) toolbar.append(button(view.start.label, () => void send(
      { action: 'start', task: view.task.id, guard: view.start!.guard }, () => void refresh(),
    )))
    if (view.agent) {
      const agent = view.agent
      toolbar.append(anchor(agent.label, options.href(agent.id, 'terminal'), () => options.openAgent(agent.id)))
    }
    if (view.worktree) {
      const tree = view.worktree
      toolbar.append(anchor(tree.label, options.href(tree.id, 'worktree'), () => options.openWorktree(tree.id)))
    }
    content.append(back, el('p', [view.task.key, view.status].filter(Boolean).join(' · '), 'board-key'), heading, toolbar)
    for (const section of view.sections) {
      if (!section.lines?.length && !section.criteria?.length) continue
      const block = el('section')
      block.append(el('h2', section.label))
      if (section.lines) for (const line of section.lines) block.append(el('p', line))
      if (section.criteria) {
        const list = el('ul')
        for (const item of section.criteria) {
          const row = el('li')
          const check = el('input')
          check.type = 'checkbox'
          check.checked = item.met
          check.disabled = true
          const label = el('label')
          label.append(check, document.createTextNode(item.text))
          row.append(label)
          list.append(row)
        }
        block.append(list)
      }
      content.append(block)
    }
    body.replaceChildren(content)
  }

  function edit(view: Form, ticket?: Ticket): void {
    const form = el('form')
    const fields = el('div', undefined, 'board-fields')
    for (const field of view.fields) {
      const label = el('label', field.label)
      const input = field.multiline ? el('textarea') : el('input')
      input.name = field.name
      input.required = !!field.required
      input.value = field.value
      label.append(input)
      fields.append(label)
    }
    const actions = el('div', undefined, 'board-actions')
    const submit = el('button', view.submit)
    submit.type = 'submit'
    const cancel = button(view.cancel, () => dialog.close(), true)
    actions.append(submit, cancel)
    const error = el('p', '', 'plan-notice')
    error.setAttribute('role', 'alert')
    error.hidden = true
    const heading = el('h2', view.heading)
    heading.id = 'board-form-heading'
    dialog.setAttribute('aria-labelledby', heading.id)
    form.append(heading, fields, error, actions)
    form.onsubmit = async event => {
      event.preventDefault()
      if (!options.current() || busy) return
      submit.disabled = true
      cancel.disabled = true
      const draft = Object.fromEntries(new FormData(form))
      const request = ticket
        ? { action: 'edit', task: ticket.task.id, version: ticket.version, draft }
        : { action: 'create', draft }
      await send(request, reply => {
        dialog.close()
        const id: string = JSON.parse(reply)
        if (id === task) void refresh()
        else options.openTicket(id)
      }, error)
      submit.disabled = false
      cancel.disabled = false
    }
    dialog.replaceChildren(form)
    dialog.showModal()
  }

  function show(on: boolean, ws: string | null = null, repo: string | null = null, id: string | null = null): void {
    if (on === front && ws === workspace && repo === repository && id === task) return
    const wasFront = front
    front = on
    workspace = ws
    repository = repo
    if (!on) {
      clearInterval(timer)
      revision++
      reading = false
      if (dialog.open) dialog.close()
      return
    }
    select(id)
    if (!wasFront) timer = window.setInterval(() => {
      if (document.visibilityState === 'visible') void refresh(true)
    }, 5000)
  }

  return { show, dispose: () => show(false) }
}
