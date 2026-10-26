/// CSP violation reports, from `report-to csp` and Firefox's `report-uri`.
///
/// They are how we hear that the policy is wrong: xterm.js wanting an inline
/// style, or a runner on a DERP map of its own whose relay the policy doesn't
/// list. A report can carry the page's URL and a sample of the blocked code,
/// so only two things are logged: the directive, and the blocked resource's
/// host (or the keyword the browser gave instead of a URL). Nothing else from
/// the body, and nothing from the request.

const MAX_BODY = 16 * 1024
const DIRECTIVE = /^[a-z-]{1,40}$/
const KEYWORD = /^(inline|eval|wasm-eval|data|blob|trusted-types-policy|trusted-types-sink|self)$/

/// Log what each report says, at most 10 of them, and answer 204.
export async function cspReport(request: Request): Promise<Response> {
  const text = await capped(request)
  if (text !== null) {
    for (const [directive, blocked] of violations(text).slice(0, 10)) console.log(`csp ${directive} ${blocked}`)
  }
  return new Response(null, { status: 204 })
}

/// The body as text, or null when it says or turns out to be over MAX_BODY.
/// Read a chunk at a time and abandoned at the cap, so an endless body costs
/// at most MAX_BODY bytes of memory.
async function capped(request: Request): Promise<string | null> {
  if (Number(request.headers.get('content-length') ?? 0) > MAX_BODY || !request.body) return null
  const reader = request.body.getReader()
  const chunks: Uint8Array[] = []
  let size = 0
  try {
    for (;;) {
      const { done, value } = await reader.read()
      if (done) break
      size += value.byteLength
      if (size > MAX_BODY) {
        await reader.cancel().catch(() => {})
        return null
      }
      chunks.push(value)
    }
  } catch {
    return null
  }
  const all = new Uint8Array(size)
  let at = 0
  for (const c of chunks) {
    all.set(c, at)
    at += c.byteLength
  }
  return new TextDecoder().decode(all)
}

/// Each report's directive and blocked host, as far as both are well formed.
export function violations(text: string): [string, string][] {
  let parsed: unknown
  try {
    parsed = JSON.parse(text)
  } catch {
    return []
  }
  // report-to sends an array of reports; report-uri sends one {"csp-report": …}.
  const reports = Array.isArray(parsed) ? parsed.map(r => r?.body) : [(parsed as any)?.['csp-report']]
  const found: [string, string][] = []
  for (const r of reports) {
    if (!r || typeof r !== 'object') continue
    const directive = String(r.effectiveDirective ?? r['effective-directive'] ?? r['violated-directive'] ?? '').split(' ')[0]
    if (!DIRECTIVE.test(directive)) continue
    found.push([directive, blockedHost(r.blockedURL ?? r['blocked-uri'])])
  }
  return found
}

function blockedHost(value: unknown): string {
  if (typeof value !== 'string') return '-'
  if (KEYWORD.test(value)) return value
  try {
    const host = new URL(value).hostname
    return /^[a-z0-9.-]{1,253}$/.test(host) ? host : '-'
  } catch {
    return '-'
  }
}
