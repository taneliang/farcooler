/// How every /app response is made: the same security headers on each.

export const SECURITY_HEADERS: Record<string, string> = {
  'cache-control': 'no-store',
  'content-security-policy':
    "default-src 'none'; style-src 'unsafe-inline'; img-src 'self'; form-action 'self' https://api.workos.com; frame-ancestors 'none'; base-uri 'none'",
  // An invite code travels in a path and a cookie; a page that links off-site
  // must not hand it on in a Referer, so nothing leaves the site. It is
  // same-origin and not no-referrer because a browser sends `Origin: null`
  // on a form post under no-referrer (Fetch, "serialize a request origin"),
  // and forms.ts refuses a post that doesn't name this site (ov-602).
  'referrer-policy': 'same-origin',
  'x-content-type-options': 'nosniff',
  'x-frame-options': 'DENY',
}

export function respond(body: string | null, status: number, extra: Record<string, string> = {}, cookies: string[] = []): Response {
  const headers = new Headers({ ...SECURITY_HEADERS, ...extra })
  if (body !== null) headers.set('content-type', 'text/html; charset=utf-8')
  for (const value of cookies) headers.append('set-cookie', value)
  return new Response(body, { status, headers })
}

export const redirect = (location: string, cookies: string[] = []) => respond(null, 303, { location }, cookies)
