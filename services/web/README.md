# Web client

The Worker behind app.farcooler.com (and app.canary.farcooler.com). Its shell's
Content-Security-Policy is built from `wrangler.toml` (`src/headers.ts`).

## Adding a Tailcat relay host

The CSP names Tailcat's relays one by one, not as `*.ipn.dev`, so a relay that
appears in https://tailcat.dev/derpmap.json is blocked until its host is listed:
connects to runners homed there fail with a false "didn't answer".

1. Add the host to `DERP_HOSTS` (space separated) in every environment of
   `wrangler.toml`: the top-level `[vars]` and `[env.canary.vars]`.
2. Update the host list in `test/config.test.ts`.
3. Run `scripts/derp-hosts-check.py` from the repo root; it exits 0 when the
   map and both lists agree.

A daily workflow (`.github/workflows/derp-hosts.yml`) runs that check and opens
an issue when the map and `DERP_HOSTS` differ.
