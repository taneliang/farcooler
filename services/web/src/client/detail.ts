/// A task's detail in the browser (ov-871): what a Needs You item or a plan
/// card opens, as the Mac and the phones show it. The task and its notes are
/// the runner's own (`task.get`, in `task show --json`'s shape); the one thing
/// written from here is a note, the call a phone's Add Note makes.
///
/// Nothing here touches the page or wasm, so a test runs it with a runner it
/// plays by hand. The DOM is detail-view.ts.

/// What was tapped: enough to head the sheet before the task is read.
export interface Subject {
  /// The runner the task is on; the page reads it from that connection.
  runner: string
  id: string
  key: string
  title: string
}

export interface Criterion {
  id: string
  text: string
  met: boolean
}

export interface Note {
  id: string
  kind: string
  actor: string
  /// Unix milliseconds, or null.
  at: number | null
  body: string
}

export interface Detail {
  id: string
  key: string
  title: string
  status: string
  intent: string
  acceptance: Criterion[]
  constraints: string[]
  labels: string[]
  notes: Note[]
  /// The keys of the cards this one waits on.
  blockedBy: string[]
}

const text = (v: unknown): string => (typeof v === 'string' ? v : '')
const list = (v: unknown): any[] => (Array.isArray(v) ? v : [])

/// The task in the runner's `{task, notes, blocks}`, or null when it isn't one.
export function parseDetail(json: string): Detail | null {
  let raw: any
  try {
    raw = JSON.parse(json)
  } catch {
    return null
  }
  const t = raw?.task
  if (!t || typeof t.id !== 'string') return null
  return {
    id: t.id,
    key: text(t.key),
    title: text(t.title),
    status: text(t.status),
    intent: text(t.intent),
    acceptance: list(t.acceptance)
      .filter(a => a && typeof a.text === 'string')
      .map(a => ({ id: text(a.id), text: a.text, met: a.met === true })),
    constraints: list(t.constraints).filter(c => typeof c === 'string'),
    labels: list(t.labels).filter(l => typeof l === 'string'),
    notes: list(raw.notes)
      .filter(n => n && typeof n.body === 'string')
      .map(n => ({ id: text(n.id), kind: text(n.kind), actor: text(n.actor), at: typeof n.at === 'number' ? n.at : null, body: n.body })),
    blockedBy: list(raw.blocks)
      .map(b => text(b?.short))
      .filter(Boolean),
  }
}

/// A status in the Mac's words.
export function statusTitle(status: string): string {
  switch (status) {
    case 'backlog':
      return 'Backlog'
    case 'todo':
      return 'To Do'
    case 'needs_decision':
      return 'Needs Decision'
    case 'in_progress':
      return 'In Progress'
    case 'in_review':
      return 'In Review'
    case 'done':
      return 'Done'
    case 'cancelled':
      return 'Cancelled'
    default:
      return ''
  }
}

/// What kind of entry a note is, in a word or two. The runner writes some
/// kinds itself (a status change, a start): those read as quiet lines.
export function noteKindTitle(kind: string): string {
  switch (kind) {
    case 'decision':
      return 'Decision'
    case 'finding':
      return 'Finding'
    case 'question':
      return 'Question'
    case 'answer':
      return 'Answer'
    case 'progress':
      return 'Progress'
    case 'comment':
      return 'Note'
    case 'status_change':
      return 'Status'
    case 'created':
      return 'Created'
    case 'wait':
      return 'Start'
    case 'worker':
      return 'Agent'
    default:
      return 'Note'
  }
}

/// Entries the runner writes about itself, drawn small.
export const isQuiet = (note: Note): boolean => ['status_change', 'created', 'wait', 'worker'].includes(note.kind)

/// Why the task can't be shown, by the failure word of the read.
export function detailFailure(word: string, runner: string): string {
  if (word === 'runner_too_old') return `Update Far Cooler on ${runner} to see this card here.`
  if (word === 'closed' || word === 'no_answer') return `${runner} didn’t answer. Far Cooler is reconnecting.`
  if (word === 'refused') return 'This card isn’t on the runner anymore.'
  return `Far Cooler couldn’t read this card on ${runner}. It tries again in a few seconds.`
}

/// Why a note didn't land, in plain words, never the runner's code.
export function noteRefusal(failure: { word: string; code?: string }, runner: string): string {
  if (failure.word !== 'refused') {
    return failure.word === 'closed' || failure.word === 'no_answer'
      ? `${runner} didn’t answer. Your note may not have arrived. Check the card, then try again.`
      : 'Your note may not have reached the runner. Try again.'
  }
  switch (failure.code) {
    case 'scope-denied':
      return `This browser can only look at ${runner}. Adding a note needs control, which is granted from a device that already has it.`
    case 'capability-unsupported':
      return `Far Cooler on ${runner} is too old for this. Update it there, then try again.`
    case 'not-found':
      return 'This card isn’t on the runner anymore.'
    default:
      return `${runner} didn’t take that note. Try again.`
  }
}

export interface DetailView {
  /// `loading` until the first read lands.
  state: 'loading' | 'ready' | 'failed'
  subject: Subject
  detail: Detail | null
  /// A note is being sent.
  busy: boolean
  /// Said under the form, when a note didn't land.
  notice: string
  /// Said over the card, when a read failed.
  banner: string
}

export interface DetailDeps {
  runner: string
  task(id: string): Promise<string>
  addNote(id: string, body: string): Promise<void>
  show(view: DetailView): void
}

export class DetailBoard {
  private detail: Detail | null = null
  private state: DetailView['state'] = 'loading'
  private banner = ''
  private notice = ''
  private busy = false
  private reading: Promise<void> | null = null
  private dead = false

  constructor(
    private readonly deps: DetailDeps,
    readonly subject: Subject,
  ) {}

  /// Read the task now; a poll that lands inside a read waits for it.
  refresh(): Promise<void> {
    if (this.dead) return Promise.resolve()
    this.reading ??= this.read().finally(() => {
      this.reading = null
    })
    return this.reading
  }

  private async read(): Promise<void> {
    try {
      const detail = parseDetail(await this.deps.task(this.subject.id))
      if (!detail) throw new Error('protocol')
      this.detail = detail
      this.state = 'ready'
      this.banner = ''
    } catch (e) {
      if (this.dead) return
      // A card already shown stays under the line that says why it may be behind.
      if (this.state !== 'ready') this.state = 'failed'
      this.banner = detailFailure((e as Error).message, this.deps.runner)
    }
    this.publish()
  }

  /// Add a note, trimmed. True when the runner took it; the card is then read
  /// again. Nothing is sent for an empty one, or while one is out.
  async note(body: string): Promise<boolean> {
    const words = body.trim()
    if (this.dead || this.busy || !words) return false
    this.busy = true
    this.notice = ''
    this.publish()
    let ok = true
    try {
      await this.deps.addNote(this.subject.id, words)
    } catch (e) {
      ok = false
      const failure = e as Error & { code?: string }
      this.notice = noteRefusal({ word: failure.message, code: failure.code }, this.deps.runner)
    }
    this.busy = false
    if (ok) await this.refresh()
    else this.publish()
    return ok
  }

  stop(): void {
    this.dead = true
  }

  publish(): void {
    if (this.dead) return
    this.deps.show({ state: this.state, subject: this.subject, detail: this.detail, busy: this.busy, notice: this.notice, banner: this.banner })
  }
}
