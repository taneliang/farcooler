import { applyD1Migrations, env } from 'cloudflare:test'

// The suite's schema is `migrations/`, applied in order, exactly as a deploy
// applies it. So a migration that only works against a fresh database — an
// ALTER naming a column 0001 never created, a CREATE TABLE that already exists
// — fails here rather than half way through a production rollout, which is the
// one moment this service cannot take a failure.
await applyD1Migrations((env as any).DB, (env as any).TEST_MIGRATIONS)
