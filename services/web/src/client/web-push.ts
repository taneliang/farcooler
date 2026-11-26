// The browser owns permissions and the PushManager subscription.
import { relayCopy } from './relay-client'

const worker = '/web-push-worker.js'
type PolicyFactory = { createPolicy(name: string, options: { createScriptURL(): string }): { createScriptURL(input: string): string } }
let workerUrl: string | undefined
function scriptUrl(): string {
  const trusted = (window as Window & { trustedTypes?: PolicyFactory }).trustedTypes
  return workerUrl ??= trusted?.createPolicy('fc', { createScriptURL: () => worker }).createScriptURL(worker) ?? worker
}
const configure = (action: string, body?: { endpoint: string }) => relayCopy(`account/web-push/${action}`, body)

async function subscription(): Promise<PushSubscription | null> {
  return await (await navigator.serviceWorker?.getRegistration('/'))?.pushManager?.getSubscription() ?? null
}

export async function stopWebPush(): Promise<void> {
  const kept = await subscription()
  if (!kept) return
  await kept.unsubscribe()
  await configure('remove', { endpoint: kept.endpoint })
}

export function mountWebPush(button: HTMLButtonElement, note: HTMLElement): void {
  if (!('serviceWorker' in navigator) || !('PushManager' in window) || !('Notification' in window)) return
  void prepare().catch(() => {})

  async function prepare() {
    const existing = await subscription()
    const { status, body } = await configure('key', existing ? { endpoint: existing.endpoint } : undefined)
    if (status !== 200) return
    let kept = body?.enrolled === true ? existing : null
    let stale = kept ? null : existing
    const key = typeof body?.key === 'string' ? body.key : null
    if (!key && !kept) return
    const registration = key ? await navigator.serviceWorker.register(scriptUrl()) : null
    if (registration) await navigator.serviceWorker.ready
    const draw = () => button.setAttribute('aria-pressed', String(Boolean(kept)))
    draw()
    button.hidden = false
    button.onclick = async () => {
      button.disabled = true
      note.hidden = true
      try {
        if (kept) {
          await stopWebPush()
          kept = null
        } else if (key && registration) {
          if (await Notification.requestPermission() !== 'granted') throw new Error()
          // subscribe() can return the old endpoint, which may still belong
          // to the previous account. Replace it before registering again.
          if (stale) {
            await stale.unsubscribe()
            stale = null
          }
          const fresh = await registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key })
          try {
            const added = await configure('add', { endpoint: fresh.endpoint })
            if (added.status !== 200) throw new Error()
            kept = fresh
          } catch (error) {
            await fresh.unsubscribe()
            throw error
          }
        }
      } catch {
        note.hidden = false
      } finally {
        draw()
        button.hidden = !key && !kept
        button.disabled = false
      }
    }
  }
}
