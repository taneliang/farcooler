import { defineConfig } from 'vitest/config'

// These adapters exercise the actual Rust JSON bridge on the host, with no policy stub.
export default defineConfig({ test: {
  include: ['test/needs.test.ts', 'test/fleet.test.ts', 'test/scope.test.ts'],
  setupFiles: ['test/needs-core-setup.ts'],
  maxWorkers: 1,
} })
