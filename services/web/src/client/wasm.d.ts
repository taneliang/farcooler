/// The two wasm modules' JavaScript faces, as the page uses them. The build
/// (scripts/build.mjs) points these module names at the files
/// crates/web/build.sh made: `fc-web` is crates/web's wasm-bindgen glue, and
/// `fc-go-runtime` is Go's wasm_exec.js, imported for the `Go` class it
/// defines on globalThis.

declare module 'fc-web' {
  export default function init(options: { module_or_path: string }): Promise<unknown>
  /// A new ceremony's reply key, as its secret half. Kept while the code on
  /// screen is up, and given to `makeOffer` and `openReply`.
  export function newReplySecret(): string
  /// The offer this browser shows, as the JSON its code carries. It names the
  /// public half of `replySecret`, so a phone can post a sealed reply.
  export function makeOffer(name: string, account: string, keyA: string, nodeKey: string, replySecret: string): string
  /// Opens the EARLIEST reply in the relay's answer (`{replies:[…]}` as JSON)
  /// and answers `{code, granted_by, runners}` as JSON, or throws an Error
  /// whose message is the ceremony's word (`unopened`, `stale`, …).
  export function openReply(relayAnswer: string, offer: string, replySecret: string, alreadyTaken: boolean, shownAtMs: number): string
  /// What this browser's passkey signs to register `keyLine` for `account`.
  export function registrationData(account: string, challenge: string, keyLine: string): Uint8Array
  /// The armored proof `devices/register` takes, from the passkey's answer.
  export function registrationProof(
    account: string,
    challenge: string,
    keyLine: string,
    authenticatorData: Uint8Array,
    clientDataJson: Uint8Array,
    signature: Uint8Array,
  ): string
  /// `{granted_by, runners}` as JSON, or throws an Error whose message is the
  /// ceremony's word (`stale` once the code is past the window).
  export function acceptReply(reply: string, offer: string, alreadyTaken: boolean, shownAtMs: number): string
  export function offerIsStale(shownAtMs: number): boolean
  export function freshnessMs(): number
  /// This build's app host: the RP ID its passkey is made and used for.
  export function rpId(): string
  /// A passkey's SPKI (`getPublicKey()`) as an `sk-ecdsa` OpenSSH line.
  export function passkeyKeyLine(spki: Uint8Array, comment: string): string
  export function fingerprint(publicKey: string): string | undefined
  export function connect(
    conn: import('./tunnel').TunnelConn,
    hostKey: string,
    user: string,
    keyA: string,
    /// `navigator.credentials.get` over the login's data; rejects with
    /// `declined` when the person or the browser said no.
    assert: (challenge: Uint8Array) => Promise<import('./device').Signed>,
    /// Called once if this connection goes silent for nine seconds or closes
    /// (crates/web/src/liveness.rs). Not called when the runner is freed.
    onLost: () => void,
  ): Promise<WebRunner>
  export function build(): string
  export class WebRunner {
    terminals(): Promise<string>
    attach(terminal: string, output: (bytes: Uint8Array) => void, ended: () => void): Promise<WebPane>
    /// The phones' shapes (crates/client's needs_you_json, plan_json and
    /// page_json), as JSON. Needs You (needs.ts) and the plan (plan.ts) are shown.
    needsYou(): Promise<string>
    /// Answer an ask Needs You lists (its `ask_id`, the action's `id`). A
    /// refusal rejects with message `refused` and the runner's `code` and
    /// `what`.
    answer(terminal: string, ask: string, option: string): Promise<void>
    /// Answer a decision with the option's text, on its task.
    answerDecision(task: string, body: string): Promise<void>
    /// The plan (plan.ts): `plan_json`, the shape `farcooler plan --json` prints.
    plan(workspace: string): Promise<string>
    /// `workspace.list` as `workspace_json`.
    workspaces(): Promise<string>
    /// The plan's edits, each the CLI's own RPC. A refusal rejects with message
    /// `refused` and the runner's `code` and `what`.
    setThemeNext(theme: string, next: string): Promise<void>
    keepRuling(ruling: string): Promise<void>
    keepAllRulings(workspace: string): Promise<number>
    page(workspace: string, slot: string): Promise<string>
    /// An agent's rows in the apps' shape (crates/client's rows_json): the
    /// newest, or up to `limit` before row `before`; and what changed after a
    /// revision, held by the runner while nothing has.
    agentRows(terminal: string, before: number | undefined, limit: number): Promise<string>
    agentRowsFollow(terminal: string, epoch: number, afterRev: number, waitMs: number): Promise<string>
    /// Send a message as the person's; true when the agent was busy and it waits.
    compose(terminal: string, text: string): Promise<boolean>
    /// Stop the turn the agent is working on.
    interrupt(terminal: string): Promise<void>
    /// Whether the runner answers right now. False means the connection is
    /// over, and `onLost` is called.
    check(): Promise<boolean>
    readonly lost: boolean
    free(): void
  }
  export class WebPane {
    write(bytes: Uint8Array): Promise<void>
    resize(columns: number, rows: number): Promise<void>
    close(): void
    free(): void
  }
}

declare module 'fc-go-runtime' {}

/// The hashed paths of the two modules, stamped by the build.
declare const __WASM__: { web: string; tailcat: string }
declare const __BUILD__: string
declare const __CHANNEL__: string
