import { defineWorkersConfig } from '@cloudflare/vitest-pool-workers/config'

import { relayTestConfig } from './vitest.config'

// The signaling suite alone, with isolated storage off: `vitest.config.ts`
// says why.
export default defineWorkersConfig(() => relayTestConfig({ isolated: false }))
