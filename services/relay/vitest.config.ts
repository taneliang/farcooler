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
    // The suite reads two files outside this package: `wrangler.toml` beside it,
    // and the Android app's `Notifier.kt` four directories up. Vite refuses an
    // import above the project root unless told otherwise, and the second one is
    // worth telling it about — the relay now names Android's notification
    // channel ids, and FCM DROPS a notification whose channel the app never
    // created, so a drift between the two languages is a push that arrives
    // nowhere and reports success. Nothing but a test that reads both would see
    // it.
    server: { fs: { allow: [fileURLToPath(new URL('../..', import.meta.url))] } },
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
              // The `iss` every token in this suite carries, in the shape a real
              // AuthKit access token carries it: the path form, ending in this
              // environment's client id. It said `https://api.workos.com` here
              // and in all four blocks of wrangler.toml until 2026-08-19, on the
              // strength of a docs sample, and every session route 401'd on
              // every channel for two days. A decoded live token settled it; see
              // the WORKOS_ISSUER comment in wrangler.toml.
              //
              // The suite does not take this on trust — `the issuer this relay
              // accepts` in relay.test.ts reads wrangler.toml and checks all
              // four values against a literal recorded from that token, because
              // fixtures minted from the same binding the verifier compares
              // against agree with any value at all, including the wrong one.
              WORKOS_ISSUER: 'https://api.workos.com/user_management/client_test',
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
              // A throwaway service account, generated for this file and used
              // nowhere else — there is no Firebase project in this repository and
              // this names none. It was the empty string until now, so `sendFcm`
              // died in `JSON.parse` before it built anything and the Android half
              // of push had NO test at all: the missing `data` key that made the
              // high-importance channel unreachable was invisible to this suite by
              // construction. The RSA key has to be real for the same reason
              // `APNS_KEY_P8` does — `googleAccessToken` signs its assertion with
              // `crypto.subtle`, and a fake one dies in `importKey`.
              FCM_SERVICE_ACCOUNT: JSON.stringify({
                project_id: 'farcooler-test',
                client_email: 'relay@farcooler-test.iam.gserviceaccount.com',
                private_key: [
                  '-----BEGIN PRIVATE KEY-----',
                  'MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQCUGNmNtGrfm4QA',
                  '9TlAN4pCy/hw1eJNuVUI3uJxAzJj4btXyNh5p7fGQa2esH0ERAzJgIsZ/P4lddTk',
                  'XNMZfHkMC1OQkZSqg7OicLdLUkrAuDosjGNFGmxohyYw85CW2ZoYYe6yFAhAnBES',
                  'mn+LMe7HEefLQ5np9Qipy3JtjKK+VxT7uPHgUWMZamKTilUwxi2mRTPA0AEn2Pyv',
                  '6sv/YQ+3e9GANH9xBjtQv1zVTz3tAbPbq5FacLSq+uU4LClVKappv5DAFeYXboYW',
                  'TaPMYDoD0e40NZeID4Xp4D8a+0PGU0DqCRfr2nW/98Iho8iizwd70aGMfuMV7cuH',
                  'b6EfHvJZAgMBAAECggEAHFxC4yxa8A3Jwl4zk7zfFQoW/lKzNvOoGV4HaqF0V6TN',
                  'edLcQ7XOz2BZ9YLyOewnL7pWSQiGCdnuBjpRcbbAvoH3w35hhCLa9l9T9pBggNX2',
                  'y/upcf0MqBrDgUnPVVf/9q3gZklISEkqys9738XU5lnjM+1y7tbuDQgJFnoBW2Yq',
                  'qnjbyFfYl5PB6Kim18Oq76uxw6vIZjqO7jpue5QnUcUITH0adkoApzjr1UnmBfHo',
                  'ftSG9lM+bFBdzfEpTjSN0eJV4pxnVxD3fK2yiqhvcMaINv7Of+hvjOekiPyiY01V',
                  'fYqtDSClHuHjJLQ+y8i1hihzNdI+L+pH/4uh6T/21QKBgQDEIDP7VGpmBauQj7HU',
                  'IvwA1pkDaqV68zis2jceCEymhPWS/taC2iEq5ytq/1m1EZ2BJpQ1d70vW8m9iMCR',
                  '8osU5UnNsEnpmeimLiMyL6yiwaYgmYCEcYCzRdmHUnFa621CT9Cj5Cw8TTWZF7Rs',
                  'VtpmJEsyUw4/Dka5ZHn27GRurQKBgQDBTxA4WfciiiS/3M1+iAT6d5dk9gjTR3hA',
                  'hS2w5F3sG7Jrc6/qj0603y5Z7sTAY35MbKW0nBL2iEkDXtlP7wrLgIKD8j/g5xOK',
                  '8j5mFHqjO5+4Vr7v5a42CdA8bxr2WtCZ7O+lUhfzzxb+vJFSFj9fV368pk9oOY6V',
                  'tlkKKGjj3QKBgBVHxTwlCbJuNRJndQ0mip9wqYOkY7Y2g9TFjKt2jRKYZKkBe7cR',
                  'Af5MvPpMIKiz85oa3IP8rQthcz9cgkCTx6GJy3tFAJAXQhYd9XWxlJLIXkU1Qquc',
                  'QTGyh4rWWDRcTSufy2ytClu0qPcmik4jEml40KvyNR6EZwogq9cuCSu1AoGBAKxg',
                  '3ZzylM+nEnhI5LJlhtL3G/j68Qm+3LvkRsdMDXkDhcoN4pwu6MefkUy+/5Jz5mcu',
                  'J2H0H3DaPQmVZgHCrwSjdz9EIbRjOukXdY8/ydCP1bDjIeb5EK29eIS7qvZuK0Bn',
                  'qZfpqdRPIjlMW+YwUpiphCmjwIG3ea+FaMcHG+m9AoGAEd0+gLPeXXsogXXEDYcy',
                  'lvtybDzkuaiAcHoJtkk08Re6xZMNz0YXJZtM6cZXOsFTlBObT+9JDIxdK57RVrKd',
                  'AhggZ9p0Ygz8HbssjeoPcSbcoPvE317jxceGs1DGRBaF2E+BN3lapzJsY1cPTc4/',
                  'hRNhsv5iMzAFjsfCBGl2kTI=',
                  '-----END PRIVATE KEY-----',
                ].join('\n'),
              }),
            },
          },
        },
      },
    },
  }
})
