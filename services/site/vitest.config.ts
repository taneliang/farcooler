import { fileURLToPath } from 'node:url'

import { defineWorkersConfig } from '@cloudflare/vitest-pool-workers/config'

// Tests run inside workerd, so HTMLRewriter and the Cache API are the real ones.
// The network is the one thing faked (see download.test.ts): the appcast is a
// string the test controls.
export default defineWorkersConfig({
  // test/channels.test.ts reads the relay's wrangler.toml, to check each
  // channel's site names that channel's relay and WorkOS client.
  server: { fs: { allow: [fileURLToPath(new URL('..', import.meta.url))] } },
  test: {
    poolOptions: {
      workers: {
        singleWorker: true,
        miniflare: {
          compatibilityDate: '2026-01-15',
          // public/, served by the real asset handler, so test/docs.test.ts
          // fetches the docs pages as www does (`/docs/x` serves `x.html`).
          assets: { directory: './public', binding: 'ASSETS' },
        },
      },
    },
  },
})
