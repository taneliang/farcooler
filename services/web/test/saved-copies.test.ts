import { afterEach, describe, expect, it, vi } from 'vitest'
import words from '../../../crates/client/src/testdata/recovery-view.json'
import { relayCopy } from '../src/client/relay-client'
import { deleteCopy, SavedCopies } from '../src/client/saved-copies'

// A small DOM surface; real text sinks and event handlers are exercised without
// putting ciphertext in markup or needing a passkey to remove junk copies.
class Element {
  children: Element[] = []
  parent?: Element
  textContent = ''
  style = {}
  disabled = false
  open = false
  onclick?: () => Promise<void> | void
  append(...nodes: Element[]) { for (const node of nodes) { node.parent = this; this.children.push(node) } }
  replaceChildren() { this.children = [] }
  remove() { if (this.parent) this.parent.children = this.parent.children.filter(n => n !== this) }
  setAttribute() {}
  showModal() { this.open = true }
  close() { this.open = false }
}

afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers() })

describe('account-owned saved copies', () => {
  it('shows every copy, clears an arbitrary junk copy without a passkey, and keeps failures available to retry', async () => {
    vi.stubGlobal('document', { createElement: () => new Element() })
    const dialog = new Element()
    const deleted: string[] = []
    let fail = true
    const view = new SavedCopies(dialog as unknown as HTMLDialogElement, async (route, body: any) => {
      if (route === 'web/vault') return { status: 200, body: { vaults: [{ credential: 'current' }, { credential: '<junk>' }] } }
      deleted.push(body.credential)
      return { status: fail ? 502 : 200, body: {} }
    }, () => 'current', async (_count, current) => ({ ...words, passkey: current ? words.passkey : 'Another passkey' }))
    await view.open()
    expect(dialog.open).toBe(true)
    const message = dialog.children[3]
    const list = dialog.children[4]
    expect(list.children).toHaveLength(2)
    expect(list.children[0].children[0].textContent).toBe('This browser’s passkey')
    const junk = list.children[1]
    expect(junk.children[1].textContent).toBe('<junk>')
    const clear = junk.children[2]
    await clear.onclick!()
    expect(message.textContent).toContain('couldn’t clear')
    expect(clear.disabled).toBe(false)
    expect(list.children).toHaveLength(2)
    fail = false
    await clear.onclick!()
    expect(list.children).toHaveLength(1)
    expect(deleted).toEqual(['<junk>', '<junk>'])
  })

  it('reports a failed list instead of claiming the account has no copies', async () => {
    vi.stubGlobal('document', { createElement: () => new Element() })
    const dialog = new Element()
    await new SavedCopies(dialog as unknown as HTMLDialogElement, async () => { throw new Error('offline') }, () => undefined, async () => words).open()
    expect(dialog.children[3].textContent).toContain('couldn’t load')
  })

  it('shows a closeable failure when the presentation module cannot load', async () => {
    vi.stubGlobal('document', { createElement: () => new Element() })
    const dialog = new Element()
    const call = vi.fn()
    await new SavedCopies(dialog as unknown as HTMLDialogElement, call, () => undefined,
      async () => { throw new Error('wasm unavailable') }).open()
    expect(dialog.open).toBe(true)
    expect(dialog.children[3].textContent).toBe(words.loadFailed)
    expect(call).not.toHaveBeenCalled()
    await dialog.children[2].onclick!()
    expect(dialog.open).toBe(false)
  })

  it('opens while the presenter loads and stays closed if dismissed during that wait', async () => {
    vi.stubGlobal('document', { createElement: () => new Element() })
    const dialog = new Element()
    const call = vi.fn()
    let loaded!: (view: typeof words) => void
    const opening = new SavedCopies(dialog as unknown as HTMLDialogElement, call, () => undefined,
      () => new Promise(resolve => { loaded = resolve })).open()
    expect(dialog.open).toBe(true)
    expect(dialog.children[3].textContent).toBe(words.loading)
    await dialog.children[2].onclick!()
    loaded(words)
    await opening
    expect(dialog.open).toBe(false)
    expect(call).not.toHaveBeenCalled()
  })

  it('keeps an expired-session delete on the page so Sign Out can report its failure', async () => {
    const assign = vi.fn()
    vi.stubGlobal('location', { assign })
    vi.stubGlobal('fetch', async () => new Response('{}', { status: 401 }))
    expect(await deleteCopy(relayCopy, 'cred')).toBe(false)
    expect(assign).not.toHaveBeenCalled()
  })

  it('accepts only a confirmed delete; refusal, network failure and timeout fail', async () => {
    expect(await deleteCopy(async () => ({ status: 200, body: {} }), 'cred')).toBe(true)
    expect(await deleteCopy(async () => ({ status: 401, body: {} }), 'cred')).toBe(false)
    expect(await deleteCopy(async () => { throw new Error('offline') }, 'cred')).toBe(false)
    vi.useFakeTimers()
    const pending = deleteCopy(() => new Promise(() => {}), 'cred')
    await vi.advanceTimersByTimeAsync(2_000)
    expect(await pending).toBe(false)
  })
})
