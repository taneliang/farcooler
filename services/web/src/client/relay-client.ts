/// A call to the relay, through this Worker (services/web/src/api.ts). The
/// `x-fc` header and the same-origin `Origin` are its CSRF check.
export async function relayRaw(route: string, body: unknown = {}, redirect = true): Promise<{ status: number; body: any }> {
  const response = await fetch(`/api/relay/${route}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-fc': '1' },
    body: JSON.stringify(body),
    credentials: 'same-origin',
  })
  if (response.status === 401) {
    if (redirect) location.assign('/')
    return { status: 401, body: null }
  }
  return { status: response.status, body: await response.json().catch(() => null) }
}

/// Copy requests may still be pending when Sign Out begins. Keep failures on
/// this page so Sign Out can report an unconfirmed deletion before leaving.
export function relayCopy(route: string, body: unknown = {}): Promise<{ status: number; body: any }> {
  return relayRaw(route, body, false)
}
