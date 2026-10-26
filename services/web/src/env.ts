/// What the web client's Worker reads from wrangler.toml, per channel.

import type { AppEnv } from '../../common/relay'

export interface WebEnv extends AppEnv {
  /// build.sh's output (wrangler.toml `[assets]`).
  ASSETS: Fetcher
  /// The one host this Worker answers on, e.g. app.canary.farcooler.com.
  /// Empty answers nothing.
  APP_HOST?: string
  /// Who gets the client: "" (no one), or "owner" (the ids in WEB_ACCOUNTS).
  /// `admitted` in flag.ts reads both.
  WEB_CLIENT?: string
  /// Account ids, separated by commas or spaces.
  WEB_ACCOUNTS?: string
  /// The origin the page fetches Tailcat's DERP map from.
  DERP_MAP_ORIGIN?: string
  /// The DERP relays' hostnames, separated by spaces; `*.` allowed in front.
  DERP_HOSTS?: string
}
