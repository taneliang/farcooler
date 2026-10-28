import { fileURLToPath } from 'node:url'

import { defineWorkersConfig } from '@cloudflare/vitest-pool-workers/config'

// Tests run inside workerd. The relay and the ASSETS binding are the two
// things each test plays (test/helpers.ts), so a test controls every byte the
// Worker reads.
export default defineWorkersConfig({
  // test/config.test.ts reads this Worker's wrangler.toml and the relay's, and
  // test/needs.test.ts the fixture every client decodes (repo test/fixtures).
  server: { fs: { allow: [fileURLToPath(new URL('../..', import.meta.url))] } },
  test: {
    poolOptions: {
      workers: {
        singleWorker: true,
        miniflare: { compatibilityDate: '2026-01-15' },
      },
    },
  },
})
