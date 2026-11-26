import { afterEach, expect, it, vi } from 'vitest'
import { mountWebPush } from '../src/client/web-push'

afterEach(() => vi.unstubAllGlobals())

it('opts in with a fresh endpoint after the relay forgets this browser', async () => {
  const staleEndpoint = 'https://fcm.googleapis.com/fcm/send/old'
  const freshEndpoint = 'https://fcm.googleapis.com/fcm/send/new'
  const stale = {
    endpoint: staleEndpoint,
    unsubscribe: vi.fn(async () => { current = null; return true }),
  }
  const fresh = { endpoint: freshEndpoint, unsubscribe: vi.fn(async () => true) }
  let current: typeof stale | null = stale
  const registration = {
    pushManager: {
      getSubscription: async () => current,
      subscribe: async () => current ?? (current = fresh),
    },
  }
  vi.stubGlobal('navigator', { serviceWorker: {
    getRegistration: async () => registration,
    register: async () => registration,
    ready: Promise.resolve(registration),
  } })
  vi.stubGlobal('window', { PushManager: true, Notification: true })
  vi.stubGlobal('Notification', { requestPermission: async () => 'granted' })

  const fetch = vi.fn(async (url: string, options: RequestInit) => {
    const { endpoint } = JSON.parse(String(options.body))
    if (url.endsWith('/key')) return Response.json({ key: 'public-key', enrolled: false })
    return Response.json({}, { status: endpoint === freshEndpoint ? 200 : 409 })
  })
  vi.stubGlobal('fetch', fetch)
  const setAttribute = vi.fn()
  const button = { hidden: true, disabled: false, onclick: async () => {}, setAttribute }
  const note = { hidden: true }
  mountWebPush(button as unknown as HTMLButtonElement, note as HTMLElement)
  await vi.waitFor(() => expect(button.hidden).toBe(false))
  expect(setAttribute).toHaveBeenLastCalledWith('aria-pressed', 'false')

  await button.onclick()

  expect(fetch).toHaveBeenLastCalledWith('/api/relay/account/web-push/add', expect.objectContaining({
    body: JSON.stringify({ endpoint: freshEndpoint }),
  }))
  expect(stale.unsubscribe).toHaveBeenCalledOnce()
  expect(setAttribute).toHaveBeenLastCalledWith('aria-pressed', 'true')
  expect(note.hidden).toBe(true)
})
