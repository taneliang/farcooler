import { withBeacon } from './beacon'
import { canaryDownload, type DownloadEnv } from './download'

export interface Env extends DownloadEnv {
  ASSETS: Fetcher
  ANALYTICS_TOKEN?: string
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url)
    if (url.pathname === '/download/canary') {
      if (request.method !== 'GET' && request.method !== 'HEAD') {
        return new Response('Method not allowed', { status: 405, headers: { allow: 'GET, HEAD' } })
      }
      return canaryDownload(env)
    }
    return withBeacon(await env.ASSETS.fetch(request), env.ANALYTICS_TOKEN)
  },
} satisfies ExportedHandler<Env>
