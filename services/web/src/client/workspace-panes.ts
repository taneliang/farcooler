import { workFailure, type WebRunner } from 'fc-web'
import type { WorkError } from './work-screen'
import { AgentPane } from './agent-pane'
import { ChatView } from './chat'
import { connectFailure } from './copy'
import { PaneSet, type Budget } from './panes'
import { PaneView } from './terminal'
import { readLocal, type LayoutView, type PaneModel, type Slot, writeLocal } from './workspace-layout'

const node = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T
const slots = ['main', 'side'] as const

export class WorkspacePanes {
  private readonly set: PaneSet<WebRunner>
  private readonly mounts = new Map<string, { root: HTMLElement; pane: PaneView | AgentPane<WebRunner> }>()
  private readonly models = new Map<string, PaneModel>()
  private view: LayoutView | undefined
  private content = ''
  private work = Promise.resolve()
  private stopped = false

  constructor(
    scope: string,
    private readonly label: string,
    budget: Budget,
    private readonly runner: () => WebRunner | null,
    private readonly ended: (runner: WebRunner, id: string) => void,
  ) {
    this.set = new PaneSet(id => {
      const model = this.models.get(id)!
      const root = document.createElement('div')
      root.className = 'workspace-screen'
      this.stage(id).append(root)
      const host = () => {
        const element = document.createElement('div')
        element.className = 'terminal-host'
        root.append(element)
        return element
      }
      const key = `${scope}:terminal:${JSON.stringify(id)}`
      const shell = () => new PaneView(host(), (word, code, what) => this.problem(this.failure({ message: word, code, what }), id), false, {
        load: () => readLocal(`${key}:raw-scroll`),
        save: value => writeLocal(`${key}:raw-scroll`, value),
      })
      const pane = model.agent ? new AgentPane(new ChatView(host(), label, undefined, {
        load: () => readLocal(`${key}:draft`),
        save: text => writeLocal(`${key}:draft`, text),
        position: () => readLocal(`${key}:chat-scroll`),
        savePosition: value => writeLocal(`${key}:chat-scroll`, value),
      }), shell) : shell()
      this.mounts.set(id, { root, pane })
      return pane
    }, budget)
  }

  render(view: LayoutView, retry = false): void {
    this.view = view
    node('workspace-side').hidden = !view.side
    for (const slot of slots) {
      const model = view[slot]
      node(`workspace-${slot}`).dataset.focused = String(view.focus === slot)
      if (!model) continue
      const side = slot === 'side'
      const title = node(side ? 'side-title' : 'pane-title')
      title.textContent = model.title
      if (side) node('workspace-side').setAttribute('aria-label', model.title)
      node(side ? 'side-switch' : 'pane-bar').hidden = !model.agent || !model.attach
      node(side ? 'side-conversation' : 'view-conversation').setAttribute('aria-pressed', String(model.mode === 'conversation'))
      node(side ? 'side-terminal' : 'view-terminal').setAttribute('aria-pressed', String(model.mode === 'terminal'))
      if (model.terminal) {
        this.models.set(model.terminal, model)
        if (!model.attach) this.forget(model.terminal)
      }
    }

    const content = JSON.stringify([view.main, view.side])
    const changed = this.content !== content
    this.content = content
    const ordered: Slot[] = view.focus === 'main' ? ['main', 'side'] : ['side', 'main']
    const ids = ordered.flatMap(slot => view[slot]?.attach ? view[slot]?.terminal ?? [] : [])
    for (const id of ids) {
      const mount = this.mounts.get(id)
      if (!mount) continue
      if (this.models.get(id)!.agent !== (mount.pane instanceof AgentPane)) {
        this.set.discard(id)
        mount.root.remove()
        this.mounts.delete(id)
      } else if (mount.root.parentElement !== this.stage(id)) this.stage(id).append(mount.root)
    }
    this.set.display(ids)
    for (const slot of slots) {
      const model = view[slot]
      if (!model) continue
      const mount = model.terminal ? this.mounts.get(model.terminal) : null
      if (mount?.pane instanceof AgentPane) mount.pane.setMode(model.mode)
      if (changed || retry) this.note(slot, model.notice)
    }
    if (!changed && !retry) return
    this.enqueue(async runner => {
      for (const id of ids) {
        if (this.runner() !== runner || this.stopped) return
        if (!slots.some(slot => this.view?.[slot]?.terminal === id && this.view[slot]?.attach)) continue
        try { await this.set.openVisible(id, runner, () => this.ended(runner, id)) }
        catch (e) {
          if (this.runner() === runner && !this.stopped) this.problem(this.failure(e as WorkError), id)
        }
      }
    })
  }

  private stage(id: string): HTMLElement {
    return node(this.view?.side?.terminal === id ? 'side-stage' : 'terminal')
  }

  focus(slot: Slot, enter: boolean): void {
    node(`workspace-${slot}`).focus({ preventScroll: true })
    const id = this.view?.[slot]?.terminal
    if (enter && id) this.mounts.get(id)?.pane.focus()
  }

  restoreFocus(enter: boolean): void {
    if (this.view) this.focus(this.view.focus, enter)
  }

  async quote(text: string): Promise<boolean> {
    await this.work
    const id = this.view?.side?.terminal
    const pane = id ? this.mounts.get(id)?.pane : null
    return pane instanceof AgentPane && pane.prefill(text)
  }

  forget(id: string): void {
    this.set.discard(id)
    this.mounts.get(id)?.root.remove()
    this.mounts.delete(id)
  }

  private failure(error: Pick<WorkError, 'message' | 'code' | 'what'>): string {
    return error.code === 'scope-denied'
      ? workFailure(error.message, error.code, error.what ?? '')
      : connectFailure(error.message, this.label)
  }

  resume(runner: WebRunner): Promise<void> {
    return this.enqueue(() => this.set.resume(runner, id => this.ended(runner, id), (id, error) => {
      if (this.runner() === runner && !this.stopped) this.problem(this.failure(error as WorkError), id)
    }))
  }

  problem(text: string, id?: string): void {
    const slot = !id ? this.view?.focus : this.view?.main.terminal === id ? 'main' : this.view?.side?.terminal === id ? 'side' : undefined
    if (slot) this.note(slot, text)
  }

  dispose(): void {
    this.stopped = true
    this.set.dispose()
    for (const mount of this.mounts.values()) mount.root.remove()
    this.mounts.clear()
  }

  private note(slot: Slot, text: string): void {
    const note = node(slot === 'main' ? 'pane-note' : 'side-note')
    note.textContent = text
    note.hidden = !text
  }

  private enqueue(attach: (runner: WebRunner) => Promise<void>): Promise<void> {
    const runner = this.runner()
    this.work = this.work.catch(() => {}).then(async () => {
      if (!this.stopped && runner && this.runner() === runner) await attach(runner)
    })
    return this.work
  }
}
