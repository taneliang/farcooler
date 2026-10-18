# Far Cooler site

A Cloudflare Worker with static assets, serving www.farcooler.com.

- `/` is `public/index.html`, a single file.
- `/docs/` is the docs: `public/docs/*.html`, served as `/docs/<name>`, with `docs.css` and the
  captures in `public/docs/img/`. Each page repeats the header, the list of pages and the pager;
  `test/docs.test.ts` serves them all through the real asset handler and checks every link.
- `/download/canary` is a 302 to the newest canary `.dmg`, read from `APPCAST_URL`
  (the canary appcast). It is cached for 60 seconds. If the appcast can't be read,
  it redirects to the last build this Worker read (remembered in the edge cache for
  up to 30 days; the bucket can't list its folder, so there is nothing else to
  fall back to). With nothing remembered, it shows a friendly 503. It only ever
  redirects to an `https` `.dmg` under the appcast's own host and folder.
- A Cloudflare Web Analytics beacon is added to the page only when `ANALYTICS_TOKEN` is set.
- `/app` is the web app (`src/app/`): sign in with WorkOS AuthKit, then your teams and their
  members, read from the relay's directory API. Server-rendered HTML with no script. WorkOS's
  own tokens live in an HttpOnly cookie, and the Worker calls the relay with them; it holds no
  secret and issues no session of its own. Signed out, every `/app` page is the sign-in page.
  With `APP_RELAY_URL` or `APP_WORKOS_CLIENT_ID` empty, `/app` says it isn't available yet.

```sh
npm ci
npm test            # inside workerd
npx tsc --noEmit
npx wrangler deploy --dry-run --outdir /tmp/site-build              # www
npx wrangler deploy --dry-run --env canary --outdir /tmp/site-canary
npx wrangler deploy --dry-run --env preview --outdir /tmp/site-preview
```

## Channels

The web app follows the relay's channels: one Worker per channel, the same code, each reading its
own relay and signing in through that relay's WorkOS client.

| Channel | Host | Relay | Deploys |
|---|---|---|---|
| canary | canary.farcooler.com | relay-canary.farcooler.com | every push to main that touches the site, after its tests |
| preview | preview.farcooler.com | relay-preview.farcooler.com | with the preview relay, from promote.yml, at the tag |
| production | www.farcooler.com | relay.farcooler.com | with the stable relay, from promote.yml, at the tag |

- Production is the top level of `wrangler.toml` (no `--env`), as in the relay's. canary and preview
  are `[env.canary]` and `[env.preview]`.
- canary. and preview. serve `/app` and `/download` only. Every other path is a 302 to the same path
  on www.farcooler.com (`MARKETING_ORIGIN`), so the marketing page is on www alone.
- www carries two cadences. The marketing page (`public/`) ships when it lands on main, as before.
  The code, and so www's `/app`, ships with stable. Every www deploy is stable's `services/site` with
  main's `public/`. Until a stable tag carries the site, www deploys main's code with `/app` not set
  up. Once one does, all of www's Worker code, `/download` and the analytics beacon included,
  changes only with a stable promotion; only the marketing page ships from main. See the header of
  `.github/workflows/site.yml`.

### WorkOS clients

Each channel has one WorkOS client, shared by the apps and the web app: the `WORKOS_CLIENT_ID` in
that channel's block of `services/relay/wrangler.toml`, repeated here as `APP_WORKOS_CLIENT_ID`.

| Channel | Client ID |
|---|---|
| canary | `client_01M03MZKRH5ZPR160DATT149WX` |
| preview | `client_01M03MXE27CHCZJJ0TMY7KCMAH` |
| production | `client_01M03MXEFY8BCKCEJCXARCHMMX` |

The Worker sends it to AuthKit to start sign-in. The code exchange and refresh go through the
relay's `/v1/auth/token` and `/v1/auth/refresh`, which hold the API key, so the Worker has no secret.

Each channel's client needs these redirects in the WorkOS dashboard, beside the apps' own:

| Channel | Redirect URI | Sign-out redirect |
|---|---|---|
| canary | `https://canary.farcooler.com/app/callback` | `https://canary.farcooler.com/app` |
| preview | `https://preview.farcooler.com/app/callback` | `https://preview.farcooler.com/app` |
| production | `https://www.farcooler.com/app/callback` | `https://www.farcooler.com/app` |

## Deploy and setup

`.github/workflows/site.yml` deploys as described under Channels, after its own tsc and tests, using the repository's existing `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` secrets. Each host is a Workers custom domain in `wrangler.toml`, like the relays' hostnames: wrangler creates its DNS record and certificate on the first deploy.

Analytics is the only thing left:

1. Run the **Site analytics** workflow (Actions, Run workflow). It finds or creates the Web Analytics site for www.farcooler.com through the API and prints the site tag. The token must have `Account Settings: Write` to create (`Read` to find an existing site); without it Cloudflare returns 403 and the job fails with that message, changing nothing. If it fails, add the permission to the token, or create the site once in the dashboard (Web Analytics, Add a site).
2. Set `ANALYTICS_TOKEN` in `wrangler.toml` `[vars]` to the printed tag and push. Until then the page has no beacon.

Wrangler has no Web Analytics command; the API is `POST /accounts/{id}/rum/site_info`.
