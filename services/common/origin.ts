/// Whether a request comes from this host's own pages: the CSRF check every
/// state-changing request on the site's /app and on the web client takes first.
/// Shared from here so neither copy can drift (ov-764).
export function sameOrigin(request: Request, url: URL): boolean {
  const site = request.headers.get('sec-fetch-site')
  // A browser that says the request is cross-site is believed over a header
  // that says otherwise.
  if (site !== null && site !== 'same-origin') return false
  const origin = request.headers.get('origin')
  // A browser nulls the Origin of a post that sends no Referer, and of one from
  // a sandboxed frame or a data: page. Only the browser's own word that the
  // page is this site's tells the first from the others; a page can't forge it.
  if (origin === 'null') return site === 'same-origin'
  if (origin !== null) return origin === url.origin
  return site === 'same-origin'
}
