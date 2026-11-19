import { afterEach, describe, expect, it, vi } from 'vitest'
import { installSignOut } from '../src/client/sign-out'
import { relayCopy, relayRaw } from '../src/client/relay-client'
import { deriveKeys, syncCopy } from '../src/client/vault'
import words from '../../../crates/client/src/testdata/recovery-view.json'

function setup(call: () => Promise<{ status: number; body: any }>, copyWork?: Promise<void>, start = true) {
  const form = new EventTarget() as EventTarget & { submit: () => void }
  form.submit = vi.fn()
  const active = { device: { passkey: { id: new Uint8Array([1]) } }, copyWork, signingOut: false }
  const title = { textContent: '' }
  const detail = { textContent: '' }
  const button = () => ({ textContent: '', hidden: true, onclick: () => {}, focus: vi.fn() })
  const retry = button()
  const anyway = button()
  const elements: Record<string, unknown> = {
    '#sign-out-title': title, '#sign-out-detail': detail,
    '#sign-out-retry': retry, '#sign-out-anyway': anyway,
  }
  const dialog = Object.assign(new EventTarget(), {
    open: false,
    querySelector: (selector: string) => elements[selector],
    showModal() { this.open = true },
  })
  const forget = vi.fn(async () => {})
  const isSigningOut = installSignOut(form as unknown as HTMLFormElement, dialog as unknown as HTMLDialogElement,
    { session: () => active, call, forget, dropSecret: vi.fn() })
  const submit = () => form.dispatchEvent(new Event('submit', { cancelable: true }))
  if (start) submit()
  return { form, forget, active, dialog, title, detail, retry, anyway, submit, isSigningOut }
}

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals() })

describe('Sign Out recovery deletion', () => {
  it('reports deletion failure when an in-flight copy read expires during Sign Out', async () => {
    vi.useFakeTimers()
    const assign = vi.fn()
    vi.stubGlobal('location', { assign })
    let release!: () => void
    let started!: () => void
    const pending = new Promise<void>(resolve => { release = resolve })
    const reading = new Promise<void>(resolve => { started = resolve })
    vi.stubGlobal('fetch', async () => {
      started()
      await pending
      return new Response('{}', { status: 401 })
    })
    const { vaultKey } = await deriveKeys(new Uint8Array(32).fill(7))
    const copyWork = syncCopy(relayCopy, vaultKey, {
      account: 'owner', passkey: { id: new Uint8Array([1]), spki: new Uint8Array([2]) },
      nodePriv: 'private', nodePub: 'public', nodeDerived: true,
    }, false).then(() => {})
    await reading
    const { form, title } = setup(() => relayCopy('web/vault/delete', { credential: 'AQ' }), copyWork)
    release()
    await vi.advanceTimersByTimeAsync(0)
    expect(assign).not.toHaveBeenCalled()
    expect(form.submit).not.toHaveBeenCalled()
    expect(title.textContent).toBe(words.deleteTitle)
  })

  it('keeps the warning actionable when another request expires after Sign Out begins', async () => {
    const assign = vi.fn()
    vi.stubGlobal('location', { assign })
    let respond!: (response: Response) => void
    vi.stubGlobal('fetch', () => new Promise<Response>(resolve => { respond = resolve }))
    const { dialog, title, detail, retry, anyway, isSigningOut, submit } = setup(
      async () => ({ status: 502, body: null }), undefined, false)
    const mayRedirect = () => !isSigningOut()
    const expiredBeforeSignOut = relayRaw('directory/me', {}, mayRedirect)
    respond(new Response(null, { status: 401 }))
    await expiredBeforeSignOut
    expect(assign).toHaveBeenCalledWith('/')
    assign.mockClear()

    vi.useFakeTimers()
    const pending = relayRaw('ceremony/reply', {}, mayRedirect)
    submit()
    await vi.advanceTimersByTimeAsync(0)
    respond(new Response(null, { status: 401 }))
    await pending
    expect(assign).not.toHaveBeenCalled()
    expect(dialog.open).toBe(true)
    expect(dialog.dispatchEvent(new Event('cancel', { cancelable: true }))).toBe(false)
    expect(title.textContent).toBe(words.deleteTitle)
    expect(detail.textContent).toBe(words.deleteDetail)
    expect(retry.hidden).toBe(false)
    expect(anyway.hidden).toBe(false)
  })

  it('shows a failed delete and waits for an explicit retry or Sign Out Anyway', async () => {
    vi.useFakeTimers()
    let status = 502
    const { form, title, retry, forget, active } = setup(async () => ({ status, body: {} }))
    await vi.advanceTimersByTimeAsync(0)
    expect(active.signingOut).toBe(true)
    expect(forget).toHaveBeenCalled()
    expect(form.submit).not.toHaveBeenCalled()
    expect(title.textContent).toBe(words.deleteTitle)
    status = 200
    retry.onclick()
    await vi.advanceTimersByTimeAsync(0)
    expect(form.submit).toHaveBeenCalledOnce()
  })

  it('waits for an outstanding save before deleting, so it cannot recreate the copy after Sign Out', async () => {
    vi.useFakeTimers()
    let finish!: () => void
    const saving = new Promise<void>(resolve => { finish = resolve })
    const call = vi.fn(async () => ({ status: 200, body: {} }))
    const { form } = setup(call, saving)
    await vi.advanceTimersByTimeAsync(0)
    expect(call).not.toHaveBeenCalled()
    finish()
    await vi.advanceTimersByTimeAsync(0)
    expect(call).toHaveBeenCalledOnce()
    expect(form.submit).toHaveBeenCalledOnce()
  })

  it('warns when a save or delete stalls; Sign Out Anyway still works', async () => {
    vi.useFakeTimers()
    const call = vi.fn(async () => ({ status: 200, body: {} }))
    const { form, anyway, title } = setup(call, new Promise(() => {}))
    await vi.advanceTimersByTimeAsync(2_000)
    expect(call).not.toHaveBeenCalled()
    expect(form.submit).not.toHaveBeenCalled()
    expect(title.textContent).toBe(words.deleteTitle)
    anyway.onclick()
    expect(form.submit).toHaveBeenCalledOnce()
    const stalled = setup(() => new Promise(() => {}))
    await vi.advanceTimersByTimeAsync(2_000)
    expect(stalled.title.textContent).toBe(words.deleteTitle)
    expect(stalled.form.submit).not.toHaveBeenCalled()
  })
})
