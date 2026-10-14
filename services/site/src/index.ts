import { app, isApp, type AppEnv } from './app'
import { withBeacon } from './beacon'
import { canaryDownload, type DownloadEnv } from './download'

export interface Env extends DownloadEnv, AppEnv {
  ASSETS: Fetcher
  ANALYTICS_TOKEN?: string
  /// Set on the canary and preview sites only (wrangler.toml): where every path
  /// outside /app and /download goes, so the marketing page is on www alone.
  MARKETING_ORIGIN?: string
}

function isDownload(pathname: string): boolean {
  return pathname === '/download' || pathname.startsWith('/download/')
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url)
    // The bare domain is an alias: everyone lands on www, path and query kept.
    if (url.hostname === 'farcooler.com') {
      url.hostname = 'www.farcooler.com'
      url.protocol = 'https:'
      return Response.redirect(url.toString(), 301)
    }
    const home = env.MARKETING_ORIGIN?.trim()
    if (home && !isApp(url.pathname) && !isDownload(url.pathname)) {
      return Response.redirect(`${home.replace(/\/+$/, '')}${url.pathname}${url.search}`, 302)
    }
    if (url.pathname === '/download/canary') {
      if (request.method !== 'GET' && request.method !== 'HEAD') {
        return new Response('Method not allowed', { status: 405, headers: { allow: 'GET, HEAD' } })
      }
      return canaryDownload(env)
    }
    // The web app (src/app): its own pages, never the marketing page's assets.
    if (isApp(url.pathname)) return await app(request, env)
    return withBeacon(await env.ASSETS.fetch(request), env.ANALYTICS_TOKEN)
  },
} satisfies ExportedHandler<Env>
