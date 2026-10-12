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

## What the owner must set up before the first deploy

The deploy workflow (`.github/workflows/site.yml`) is inert until all of these exist.

1. **Repository variable `SITE_DEPLOY` = `true`.** The workflow's gate; without it the job never runs.
2. **Secrets `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`.** The same two the relay and canary workflows use, so they may already exist. The token needs Workers Scripts: Edit, and Zone: Workers Routes: Edit for the farcooler.com zone.
3. **The route.** farcooler.com must be a zone on this Cloudflare account. Then add to `wrangler.toml`:
   `routes = [{ pattern = "www.farcooler.com", custom_domain = true }]`.
   Left out until now because it makes `wrangler deploy` take over that hostname. (The apex, farcooler.com, is not routed; add it, or a redirect to www, if wanted.)
4. **The analytics token.** In Cloudflare, Web Analytics, add the site `www.farcooler.com`, choose the manual (JS snippet) install, copy the `token` value (32 hex characters), and set `ANALYTICS_TOKEN` under `[vars]` in `wrangler.toml`. Until then the page has no beacon.

Until step 3, a deploy publishes only to `farcooler-site.<account>.workers.dev`.
