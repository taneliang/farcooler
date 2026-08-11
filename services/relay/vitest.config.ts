import { fileURLToPath } from 'node:url'

import { defineWorkersConfig, readD1Migrations } from '@cloudflare/vitest-pool-workers/config'

// Tests run INSIDE workerd, against a real D1, not against mocks.
//
// The alternative — hand-written fakes for D1 and the analytics binding — would
// let a query that SQLite rejects pass the suite, which is most of what these
// tests are for: the account scoping on the revoke routes and the expiry
// predicate on notify are SQL, and SQL is the part worth actually running.
//
// The schema comes from the migration files themselves rather than a CREATE
// TABLE copied into the test. A copy drifts, and it drifts silently in the one
// direction that matters here: this service's rule is that migrations are
// additive, and a test that builds the final shape in one statement can never
// catch an ALTER that does not apply on top of what is already deployed.
export default defineWorkersConfig(async () => {
  const migrations = await readD1Migrations(fileURLToPath(new URL('./migrations', import.meta.url)))

  return {
    test: {
      setupFiles: ['./test/migrations.ts'],
      poolOptions: {
        workers: {
          singleWorker: true,
          miniflare: {
            // Not the real wrangler.toml: that one names a D1 database by id and
            // declares a rate-limit binding, neither of which exists locally. The
            // absent limiter is deliberate — `withinRate` fails open without one,
            // and these tests are about the routes, not the throttle.
            compatibilityDate: '2026-01-15',
            d1Databases: ['DB'],
            analyticsEngineDatasets: { METRICS: { dataset: 'farcooler_events' } },
            bindings: {
              TEST_MIGRATIONS: migrations,
              WORKOS_CLIENT_ID: 'client_test',
              WORKOS_API_KEY: 'sk_test_do_not_use',
              ANALYTICS_SALT: 'salt',
              // A throwaway P-256 key, generated for this file and used nowhere
              // else. It has to be a real one: the APNs path signs a JWT with
              // `crypto.subtle`, so an empty string means every test that
              // touches push dies in importKey instead of testing the thing it
              // was written for — which is how the sandbox-host bug survived.
              APNS_KEY_P8: [
                '-----BEGIN PRIVATE KEY-----',
                'MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg7vuAkIVf+IvkRxSY',
                'ZTS7sgGxbchvLkTrJnVc8Yo76CGhRANCAARyPTz7pwNEaAeA78IggdkQuC9ZC3JB',
                '0R5Emi7GlaiIWfFm/xhNWog3wT70VXKApJa6gQyLFTM74ODLT1QmYlkV',
                '-----END PRIVATE KEY-----',
              ].join('\n'),
              APNS_KEY_ID: 'KEYID00000',
              APNS_TEAM_ID: 'TEAMID0000',
              APNS_TOPIC: 'com.farcooler.ios',
              FCM_SERVICE_ACCOUNT: '',
            },
          },
        },
      },
    },
  }
})
