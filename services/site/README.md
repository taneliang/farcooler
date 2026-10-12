# Far Cooler site

A Cloudflare Worker with static assets, serving www.farcooler.com.

- `/` is `public/index.html`, a single file.
- `/download/canary` is a 302 to the newest canary `.dmg`, read from `APPCAST_URL`
  (the canary appcast). It is cached for 60 seconds. If the appcast can't be read,
  it redirects to the last build this Worker read (remembered in the edge cache for
  up to 30 days; the bucket can't list its folder, so there is nothing else to
  fall back to). With nothing remembered, it shows a friendly 503. It only ever
  redirects to an `https` `.dmg` under the appcast's own host and folder.
- A Cloudflare Web Analytics beacon is added to the page only when `ANALYTICS_TOKEN` is set.

```sh
npm ci
npm test            # inside workerd
npx tsc --noEmit
npx wrangler deploy --dry-run --outdir /tmp/site-build
```

## Deploy and setup

`.github/workflows/site.yml` deploys on every push to main that touches the site, after its own tsc and tests, using the repository's existing `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` secrets. `www.farcooler.com` is a Workers custom domain in `wrangler.toml`, like the relay's hostname.

Analytics is the only thing left:

1. Run the **Site analytics** workflow (Actions, Run workflow). It finds or creates the Web Analytics site for www.farcooler.com through the API and prints the site tag. The token must have `Account Settings: Write` to create (`Read` to find an existing site); without it Cloudflare returns 403 and the job fails with that message, changing nothing. If it fails, add the permission to the token, or create the site once in the dashboard (Web Analytics, Add a site).
2. Set `ANALYTICS_TOKEN` in `wrangler.toml` `[vars]` to the printed tag and push. Until then the page has no beacon.

Wrangler has no Web Analytics command; the API is `POST /accounts/{id}/rum/site_info`.
