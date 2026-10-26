/// The client shell's script (ov-764). Today: who is signed in, and a Reload
/// offer when a newer build is deployed. The terminal arrives in the next step
/// of slice 1.
///
/// Every value from the network goes into the page through `textContent`,
/// never markup: the CSP's Trusted Types (`require-trusted-types-for
/// 'script'`) refuse an HTML string sink outright.

declare const __BUILD__: string

/// A call to the relay, through this Worker (services/web/src/api.ts). The
/// `x-fc` header and the same-origin `Origin` are its CSRF check.
async function relay(route: string, body: unknown = {}): Promise<any> {
  const response = await fetch(`/api/relay/${route}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-fc': '1' },
    body: JSON.stringify(body),
    credentials: 'same-origin',
  })
  if (response.status === 401) {
    location.assign('/')
    return null
  }
  return response.ok ? await response.json() : null
}

async function showWho(): Promise<void> {
  const me = await relay('directory/me').catch(() => null)
  const email = typeof me?.user?.email === 'string' ? me.user.email : null
  const who = document.getElementById('who')
  if (who && email) who.textContent = email
}

/// Never reloads on its own: a reload mid-keystroke would lose what was typed.
async function checkVersion(): Promise<void> {
  const response = await fetch('/version.json', { cache: 'no-store' }).catch(() => null)
  const deployed = response?.ok ? await response.json().catch(() => null) : null
  if (typeof deployed?.build !== 'string' || deployed.build === __BUILD__) return
  document.getElementById('update')?.removeAttribute('hidden')
}

document.getElementById('reload')?.addEventListener('click', () => location.reload())
window.addEventListener('focus', () => void checkVersion())
void showWho()
