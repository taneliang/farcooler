/// Delivering to Apple and Google.
///
/// Both transports live behind one call so that everything above them — and the
/// daemon in particular — never learns which kind of phone it is talking to.
/// The daemon says what happened; where that lands is this file's business.

export interface Payload {
  title: string
  subtitle: string
  /// The terminal to open. Enough to make the notification actionable and
  /// nothing more.
  ///
  /// What comes with it is one composed line at a time — `subtitle` is the
  /// agent's question while it is blocked and its composed signal rung while
  /// it works, redacted and cut to a sidebar's width by the runner before it
  /// ever leaves. Never the transcript itself, never a command line, never raw
  /// output.
  ///
  /// Repeatedly, though, and that is worth knowing: a `working` notice moves
  /// the live card for the whole length of a run, so this interface sees a
  /// slow drip of one agent's headline rather than the two lines a run used to
  /// send. Nothing here is stored or logged — see `notify` in `index.ts` — so
  /// the relay stays a delivery service that cannot leak a conversation it
  /// never held.
  terminal: string
  /// `working`, `blocked` or `done`, and the agent's name.
  ///
  /// Not for display — the title and body already say it in a sentence. These
  /// are for the phone's notification service extension, which folds them into
  /// the snapshot its widgets render so that a lock screen widget agrees with
  /// the banner that just arrived. Optional because a daemon built before them
  /// sends neither, and gets exactly the behavior it always got.
  status?: string
  label?: string
  /// Whether the turn behind a `done` ended badly.
  ///
  /// Beside `status` rather than inside it because the relay's own decision —
  /// raise a card, move it, dismiss it — is the same either way: a failed turn
  /// is over exactly as a finished one is. This is only ever forwarded, for the
  /// extension, which has a status word and nothing else to draw a mark from
  /// and would otherwise put a `✓` on an agent that died.
  ///
  /// Optional forever, like the two above: a daemon built before it sends
  /// nothing and gets exactly the behavior it always got.
  failed?: boolean
}

/// Which of Apple's two push services issued a device's token.
///
/// A build signed locally carries `aps-environment: development`, so APNs gives
/// it a SANDBOX token, and the production service answers a sandbox token with
/// BadDeviceToken. Posting everything to `api.push.apple.com` therefore meant
/// push was dead for every dev-signed build while looking, from the relay's
/// side, like a device that had simply gone away.
export type Environment = 'development' | 'production'

/// Whether a client named an environment the relay knows.
///
/// Used to reject anything else at the routes rather than fall through to
/// production, because falling through reinstates exactly the bug above and
/// does it silently.
export function isEnvironment(value: unknown): value is Environment {
  return value === 'development' || value === 'production'
}

/// Anything that is not 'development' is production, and that includes NULL:
/// every device registered before the column existed was registered by a build
/// that could only have been talking to the production service.
function apnsHost(environment: string | null | undefined): string {
  return environment === 'development' ? 'api.sandbox.push.apple.com' : 'api.push.apple.com'
}

/// The suffix a channel's bundle identifier carries. Stable has none.
const CHANNEL_SUFFIX: Record<string, string> = {
  stable: '',
  preview: '.preview',
  canary: '.canary',
  local: '.local',
}

/// Whether this deployment's APNs topic belongs to the channel it was deployed
/// as, and what is wrong when it does not.
///
/// There is one relay per channel and `apns-topic` MUST equal the receiving
/// app's bundle identifier, which differs per channel. A relay deployed as
/// `canary` holding the stable topic therefore reproduces exactly the bug the
/// partition exists to prevent: APNs rejects every push for a token/topic
/// mismatch, and nothing anywhere says so — `sendApns` returns a boolean and
/// the daemon is told only that a notification "failed".
///
/// Six secrets have to be right per environment and this is the one whose
/// wrongness is invisible, so it is worth the twenty lines to make it loud.
///
/// Checked by SUFFIX rather than against a computed identifier: this service
/// does not know what the app is called, and a check that had to be told would
/// be a second copy of the bundle id scheme to keep in step. Asking only
/// whether the shape agrees with the channel catches the real mistake — a topic
/// belonging to the wrong channel — without inventing a dependency.
export function topicMismatch(env: {
  CHANNEL?: string
  APNS_TOPIC?: string
}): string | null {
  const channel = env.CHANNEL
  const topic = env.APNS_TOPIC
  // A deployment that never declared a channel is one made before this check,
  // and every one of those is the stable relay. Nothing to compare it against,
  // and refusing on absence would take push down on the one channel that must
  // never lose it.
  if (!channel || !topic) return null

  const suffix = CHANNEL_SUFFIX[channel]
  if (suffix === undefined) {
    return `CHANNEL is "${channel}", which is not a channel this relay knows.`
  }

  const others = Object.entries(CHANNEL_SUFFIX)
    .filter(([name, value]) => name !== channel && value !== '')
    .map(([, value]) => value)

  if (suffix === '') {
    // Stable's topic is the bare identifier, so the mistake to catch is a topic
    // wearing SOMEBODY ELSE'S suffix.
    const wrong = others.find(other => topic.endsWith(other))
    return wrong
      ? `APNS_TOPIC "${topic}" ends with "${wrong}", but this relay is deployed as stable.`
      : null
  }

  return topic.endsWith(suffix)
    ? null
    : `APNS_TOPIC "${topic}" does not end with "${suffix}", but this relay is deployed as ${channel}.`
}

export async function sendPush(
  env: any,
  platform: string,
  token: string,
  payload: Payload,
  environment: string | null,
): Promise<boolean> {
  return platform === 'apns'
    ? sendApns(env, token, payload, environment)
    : sendFcm(env, token, payload)
}

// MARK: - Apple

async function sendApns(
  env: any,
  token: string,
  payload: Payload,
  environment: string | null,
): Promise<boolean> {
  const jwt = await apnsToken(env)
  const response = await fetch(`https://${apnsHost(environment)}/3/device/${token}`, {
    method: 'POST',
    headers: {
      authorization: `bearer ${jwt}`,
      'apns-topic': env.APNS_TOPIC,
      'apns-push-type': 'alert',
      // Time-sensitive so an agent that is BLOCKED can break a Focus. It has
      // stopped and will stay stopped until answered, which is the definition
      // of the thing this interruption level exists for.
      'apns-priority': '10',
    },
    body: JSON.stringify({
      aps: {
        alert: { title: payload.title, body: payload.subtitle },
        sound: 'default',
        'interruption-level': 'time-sensitive',
        'thread-id': payload.terminal,
        // Without this the notification service extension is never invoked,
        // and the phone's widgets stay on whatever the app last wrote — which
        // on a phone nobody has opened today is nothing at all. It costs the
        // extension roughly thirty milliseconds and changes nothing about how
        // the banner looks.
        'mutable-content': 1,
      },
      terminal: payload.terminal,
      status: payload.status,
      label: payload.label,
      failed: payload.failed,
    }),
  })
  return response.ok
}

// MARK: - Live Activities

/// Everything the lock-screen card can say.
///
/// A fixed contract with the iOS app: these are the stored properties of its
/// `ContentState`, by these names. An extra key, or a `status` outside this
/// set, and the app cannot decode the push at all — the activity then freezes
/// on whatever it last showed, which looks exactly like the relay never sent
/// anything.
///
/// **This is the card's LEADER**, and it used to be the card's identity. There
/// is one card per app install now rather than one per terminal, and the agent
/// it leads with changes over the card's life as different agents block and
/// finish — so `terminal`, `label`, `machine` and `startedAt` all moved here
/// from `ActivityAttributes` below. They had to: attributes are fixed for an
/// activity's whole life and APNs rejects a push that repeats them, so a leader
/// living there could only ever be changed by ending the card and starting
/// another, which is two cards on the lock screen for the moment in between.
///
/// Nothing here says how many OTHER agents there are, and that is deliberate.
/// The relay stores no fleet and a daemon knows only its own runner, so neither
/// end of this contract can count one honestly; the card reads its own tail from
/// the fleet snapshot in the phone's App Group, which is the only thing that has
/// ever seen every runner. A count field here would be a number nothing writes,
/// which this contract has been burned by once already — see the note on
/// `detail` in `AgentActivityAttributes.swift`.
export interface ActivityState {
  /// The leading agent's terminal, so a tap on the card opens the pane it is
  /// about. Rebuilt into the card's `widgetURL` on every render, which is what
  /// lets the tap target follow a change of leader.
  terminal: string
  /// The leading agent's name, and the runner it is on. Both plain strings
  /// rather than ids: the widget extension has no connection to look anything
  /// up with.
  ///
  /// `machine` keeps that spelling because the app decodes this payload by
  /// field name. It is **runner** in every word a person reads.
  label: string
  machine: string
  status: 'working' | 'blocked' | 'done'
  detail: string
  /// When the LEADER's turn began, in Unix milliseconds, so the card can run
  /// its own clock.
  ///
  /// Here rather than in `ActivityAttributes` because it is part of the leader
  /// and the leader changes: a card that kept the first agent's start would
  /// count the second agent's work from the wrong moment. It was on the
  /// attributes precisely because it did NOT change, and that stopped being
  /// true when the card stopped being about one terminal.
  ///
  /// The app still renders it as a native timer, which is the reason it is a
  /// date and not a rendered string: no push per tick, and it keeps counting on
  /// a phone that is off the network.
  ///
  /// A NUMBER on the wire, never a date string. The app's decoder tells Unix
  /// seconds from Unix milliseconds apart by magnitude and reads this field
  /// leniently, so a string does not break the card — it silently costs the
  /// timer, which is a failure nothing reports.
  ///
  /// Optional because a daemon older than the field sends none, and because a
  /// card whose turn clock could not be read should show no timer rather than
  /// a wrong one.
  startedAt?: number
}

/// What is fixed for the life of an install's card, which is now almost nothing.
///
/// Only a `start` carries these. They are the activity's identity rather than
/// its state, so APNs rejects a push that repeats them on an update or an end —
/// which is why the union below makes it impossible to attach them to one.
export interface ActivityAttributes {
  /// Which SHAPE the card was started in, and the only thing left here.
  ///
  /// `1` is a card started before the per-install restructure: terminal-scoped,
  /// with its leader in the attributes. `2` is this one. The app uses it to
  /// decide which card survives when both are in flight after an upgrade — see
  /// `LiveActivities.precedence` — and it has to be TOLD rather than inferred,
  /// because both shapes are the same ActivityKit type under the same
  /// `attributes-type` string and the app decodes an old card's state leniently.
  ///
  /// Must equal `AgentActivityAttributes.fleetVersion` in
  /// `apps/shared/AgentKit/Sources/AgentKit/AgentActivityAttributes.swift`.
  version: number
}

/// The shape this relay starts cards in. See `ActivityAttributes.version`.
export const ACTIVITY_VERSION = 2

interface ActivityBase {
  state: ActivityState
  /// What to show if the activity itself cannot be — a locked Apple Watch, a
  /// device that has never run the app.
  ///
  /// Optional, and leaving it off is not a detail. An activity push carrying an
  /// alert dictionary is PRESENTED to the person; one without it changes the
  /// card in place and makes no sound. Routine progress must be the second kind
  /// — a banner every ten seconds for an agent that is merely busy is the thing
  /// people switch notifications off over, which then costs them the one push
  /// this product exists to deliver.
  alert?: { title: string; body: string }
}

/// When a card that is ending should leave the lock screen.
///
/// `delayed` is what a FINISHED agent gets, and is the default because it is
/// what every end push meant before there was another kind: the last state
/// stays up for `DISMISSAL_DELAY_S` so that somebody who picks the phone up
/// because of the alert beside it has something to read when they get there.
///
/// `immediate` is for a card being RETIRED rather than finished — see
/// `/v1/notify/retire`. There the runner has said it can no longer account for
/// the run behind the card, so there is no last word to leave up, and every
/// second it stays is a second of the lock screen stating something that stopped
/// being true. A minute of "Finished" for an agent whose pane the person closed
/// themselves is not a courtesy, it is the same wrong card for a shorter time.
export type Dismissal = 'delayed' | 'immediate'

export type Activity =
  | (ActivityBase & { event: 'start'; attributes: ActivityAttributes })
  | (ActivityBase & { event: 'update' })
  | (ActivityBase & { event: 'end'; dismissal?: Dismissal })

/// How long the finished state stays up before the card clears itself.
///
/// Without a dismissal date an ended activity sits on the lock screen for up to
/// four hours, which is the state this feature is least useful in and most
/// annoying in. A minute is long enough to be seen by someone picking the phone
/// up because of the alert that went with it.
const DISMISSAL_DELAY_S = 60

/// How long a push-started card may claim to be current.
///
/// There is a real hole under this. When the relay starts a card while the app
/// is not running, nothing reports that activity's update token until the
/// person next opens the app — so the relay has a live card it cannot end. If
/// the agent then finishes, or is answered from the Mac, `done` arrives to find
/// no row and the card sits there saying "Needs You" forever.
///
/// One card per install made that hole SMALLER, which is worth writing down
/// because it was the thing to check. It is one row per install now rather than
/// one per terminal, so a phone left in a pocket while four agents start work
/// ends up with one unaddressable card instead of four: the first push claims
/// the row, every later push finds it and either moves the card or is refused as
/// not the leader, and the single token the app files when it next opens
/// addresses the single card that exists. The blind window is unchanged in
/// LENGTH — it is still "until the app runs" — but what accumulates inside it no
/// longer scales with the fleet.
///
/// A stale date does not dismiss it. It marks the content as out of date so the
/// system can present it as stale rather than as current truth, which is the
/// honest answer: after an hour of silence the relay genuinely does not know
/// whether this is still true.
///
/// NOT a dismissal date. A card that vanishes on a timer while the agent is
/// still blocked deletes the one notification this whole product exists to
/// deliver, which is a worse failure than showing a stale one.
const STALE_AFTER_S = 60 * 60

/// Push a Live Activity, which is a different message to a different topic than
/// the alert push above even though it goes to the same service.
///
/// The token is NOT the device token: a start uses the app install's
/// push-to-start token and an update or end uses the token APNs issued for that
/// one running activity. Sending either to `/3/device/<device token>` is
/// rejected, and so is the reverse.
export async function sendLiveActivity(
  env: any,
  token: string,
  activity: Activity,
  environment: string | null,
): Promise<boolean> {
  // Seconds. APNs drops an activity push whose timestamp is not newer than the
  // last one it saw for that activity, and `Date.now()` puts the first push
  // fifty-odd thousand years ahead — after which every real update is stale
  // and silently discarded.
  const now = Math.floor(Date.now() / 1000)

  const aps: Record<string, unknown> = {
    timestamp: now,
    event: activity.event,
    'content-state': activity.state,
  }
  // Present only when there is something to interrupt for. See `ActivityBase`.
  if (activity.alert) aps.alert = activity.alert
  if (activity.event === 'start') {
    aps['attributes-type'] = 'AgentActivityAttributes'
    aps.attributes = activity.attributes
    aps['stale-date'] = now + STALE_AFTER_S
  }
  if (activity.event === 'end') {
    // `now` rather than an omitted date for a retirement: a dismissal date that
    // has already passed takes the card off the lock screen at once, which is
    // exactly what is being asked for, where omitting the field leaves it there
    // for hours. See `Dismissal`.
    aps['dismissal-date'] = activity.dismissal === 'immediate' ? now : now + DISMISSAL_DELAY_S
  }

  const response = await fetch(`https://${apnsHost(environment)}/3/device/${token}`, {
    method: 'POST',
    headers: {
      authorization: `bearer ${await apnsToken(env)}`,
      // The alert topic with a suffix, not a topic of its own. APNs routes on
      // it, and the plain bundle id here is a 400 rather than a delivery.
      'apns-topic': `${env.APNS_TOPIC}.push-type.liveactivity`,
      'apns-push-type': 'liveactivity',
      // Priority 10 consumes the app's Live Activity push budget, and the
      // budget it consumes is the one the ALERT depends on. Routine progress
      // therefore goes out at 5: delivered when convenient, which is the right
      // urgency for a line that will change again in ten seconds.
      'apns-priority': activity.state.status === 'working' ? '5' : '10',
    },
    body: JSON.stringify({ aps }),
  })
  return response.ok
}

/// Apple's provider token, cached for its lifetime.
///
/// APNs rejects a token refreshed more often than once every 20 minutes and
/// expires one older than an hour, so this is not merely an optimisation —
/// minting one per notification gets the sender throttled.
let cachedApns: { at: number; jwt: string } | null = null

async function apnsToken(env: any): Promise<string> {
  if (cachedApns && Date.now() - cachedApns.at < 30 * 60_000) return cachedApns.jwt

  const header = { alg: 'ES256', kid: env.APNS_KEY_ID }
  const claims = { iss: env.APNS_TEAM_ID, iat: Math.floor(Date.now() / 1000) }
  const signing = `${base64Url(JSON.stringify(header))}.${base64Url(JSON.stringify(claims))}`

  const key = await crypto.subtle.importKey(
    'pkcs8',
    pemToBytes(env.APNS_KEY_P8),
    { name: 'ECDSA', namedCurve: 'P-256' },
    false,
    ['sign'],
  )
  const signature = await crypto.subtle.sign(
    { name: 'ECDSA', hash: 'SHA-256' },
    key,
    new TextEncoder().encode(signing),
  )
  const jwt = `${signing}.${base64UrlBytes(new Uint8Array(signature))}`
  cachedApns = { at: Date.now(), jwt }
  return jwt
}

// MARK: - Google

async function sendFcm(env: any, token: string, payload: Payload): Promise<boolean> {
  const account = JSON.parse(env.FCM_SERVICE_ACCOUNT)
  const accessToken = await googleAccessToken(account)
  const response = await fetch(
    `https://fcm.googleapis.com/v1/projects/${account.project_id}/messages:send`,
    {
      method: 'POST',
      headers: { authorization: `Bearer ${accessToken}`, 'content-type': 'application/json' },
      body: JSON.stringify({
        message: {
          token,
          notification: { title: payload.title, body: payload.subtitle },
          android: { priority: 'HIGH' },
          data: { terminal: payload.terminal },
        },
      }),
    },
  )
  return response.ok
}

let cachedGoogle: { at: number; token: string } | null = null

async function googleAccessToken(account: any): Promise<string> {
  if (cachedGoogle && Date.now() - cachedGoogle.at < 30 * 60_000) return cachedGoogle.token

  const now = Math.floor(Date.now() / 1000)
  const header = { alg: 'RS256', typ: 'JWT' }
  const claims = {
    iss: account.client_email,
    scope: 'https://www.googleapis.com/auth/firebase.messaging',
    aud: 'https://oauth2.googleapis.com/token',
    iat: now,
    exp: now + 3600,
  }
  const signing = `${base64Url(JSON.stringify(header))}.${base64Url(JSON.stringify(claims))}`
  const key = await crypto.subtle.importKey(
    'pkcs8',
    pemToBytes(account.private_key),
    { name: 'RSASSA-PKCS1-v1_5', hash: 'SHA-256' },
    false,
    ['sign'],
  )
  const signature = await crypto.subtle.sign(
    'RSASSA-PKCS1-v1_5',
    key,
    new TextEncoder().encode(signing),
  )
  const assertion = `${signing}.${base64UrlBytes(new Uint8Array(signature))}`

  const response = await fetch('https://oauth2.googleapis.com/token', {
    method: 'POST',
    headers: { 'content-type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      grant_type: 'urn:ietf:params:oauth:grant-type:jwt-bearer',
      assertion,
    }),
  })
  const body = await response.json<{ access_token: string }>()
  cachedGoogle = { at: Date.now(), token: body.access_token }
  return body.access_token
}

// MARK: - Encoding

function base64Url(text: string): string {
  return base64UrlBytes(new TextEncoder().encode(text))
}

function base64UrlBytes(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes))
    .replaceAll('+', '-')
    .replaceAll('/', '_')
    .replaceAll('=', '')
}

/// A PEM's body, decoded. Both Apple's `.p8` and Google's service-account key
/// arrive this way.
function pemToBytes(pem: string): Uint8Array {
  const body = pem
    .replace(/-----[A-Z ]+-----/g, '')
    .replaceAll('\n', '')
    .replaceAll('\r', '')
    .trim()
  return Uint8Array.from(atob(body), c => c.charCodeAt(0))
}
