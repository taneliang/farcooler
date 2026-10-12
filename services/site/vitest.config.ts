import { defineWorkersConfig } from '@cloudflare/vitest-pool-workers/config'

// Tests run inside workerd, so HTMLRewriter and the Cache API are the real ones.
// The network is the one thing faked (see download.test.ts): the appcast is a
// string the test controls.
export default defineWorkersConfig({
  test: {
    poolOptions: {
      workers: {
        singleWorker: true,
        miniflare: { compatibilityDate: '2026-01-15' },
      },
    },
  },
})
