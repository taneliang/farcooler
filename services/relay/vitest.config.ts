import { defineWorkersConfig } from '@cloudflare/vitest-pool-workers/config'

// Tests run INSIDE workerd, against a real D1, not against mocks.
//
// The alternative — hand-written fakes for D1 and the analytics binding — would
// let a query that SQLite rejects pass the suite, which is most of what these
// tests are for: the account scoping on the revoke routes and the expiry
// predicate on notify are SQL, and SQL is the part worth actually running.
export default defineWorkersConfig({
  test: {
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
            WORKOS_CLIENT_ID: 'client_test',
            WORKOS_API_KEY: 'sk_test_do_not_use',
            ANALYTICS_SALT: 'salt',
            APNS_KEY_P8: '',
            APNS_KEY_ID: '',
            APNS_TEAM_ID: '',
            APNS_TOPIC: 'com.farcooler.ios',
            FCM_SERVICE_ACCOUNT: '',
          },
        },
      },
    },
  },
})
