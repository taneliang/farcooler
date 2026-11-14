import { afterEach, describe, expect, it, vi } from 'vitest'
import { installSignOut } from '../src/client/sign-out'
import { relayCopy } from '../src/client/relay-client'
import { deriveKeys, syncCopy } from '../src/client/vault'

function setup(call: () => Promise<{ status: number; body: any }>, copyWork?: Promise<void>) {
  const form = new EventTarget() as EventTarget & { submit: () => void }
  form.submit = vi.fn()
  const active = { device: { passkey: { id: new Uint8Array([1]) } }, copyWork, signingOut: false }
  const failed = vi.fn()
  const forget = vi.fn(async () => {})
  installSignOut(form as unknown as HTMLFormElement, { session: () => active, call, forget, dropSecret: vi.fn(), waiting: vi.fn(), failed })
  form.dispatchEvent(new Event('submit', { cancelable: true }))
  return { form, failed, forget, active }
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
    const { form, failed } = setup(() => relayCopy('web/vault/delete', { credential: 'AQ' }), copyWork)
    release()
    await vi.advanceTimersByTimeAsync(0)
    expect(assign).not.toHaveBeenCalled()
    expect(form.submit).not.toHaveBeenCalled()
    expect(failed).toHaveBeenCalledOnce()
  })

  it('shows a failed delete and waits for an explicit retry or Sign Out Anyway', async () => {
    vi.useFakeTimers()
    let status = 502
    const { form, failed, forget, active } = setup(async () => ({ status, body: {} }))
    await vi.advanceTimersByTimeAsync(0)
    expect(active.signingOut).toBe(true)
    expect(forget).toHaveBeenCalled()
    expect(form.submit).not.toHaveBeenCalled()
    expect(failed).toHaveBeenCalledOnce()
    status = 200
    failed.mock.calls[0][0]()
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
    const { form, failed } = setup(call, new Promise(() => {}))
    await vi.advanceTimersByTimeAsync(2_000)
    expect(call).not.toHaveBeenCalled()
    expect(form.submit).not.toHaveBeenCalled()
    failed.mock.calls[0][1]()
    expect(form.submit).toHaveBeenCalledOnce()
    const stalled = setup(() => new Promise(() => {}))
    await vi.advanceTimersByTimeAsync(2_000)
    expect(stalled.failed).toHaveBeenCalledOnce()
    expect(stalled.form.submit).not.toHaveBeenCalled()
  })
})
