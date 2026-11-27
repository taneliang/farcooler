/// The two wasm modules' JavaScript faces, as the page uses them. The build
/// (scripts/build.mjs) points these module names at the files
/// crates/web/build.sh made: `fc-web` is crates/web's wasm-bindgen glue, and
/// `fc-go-runtime` is Go's wasm_exec.js, imported for the `Go` class it
/// defines on globalThis.

declare module 'fc-web' {
  export default function init(options: { module_or_path: string }): Promise<unknown>
  export function newWriterKey(): string
  /// A new ceremony's reply key, as its secret half. Kept while the code on
  /// screen is up, and given to `makeOffer` and `openReply`.
  export function newReplySecret(): string
  /// The offer this browser shows, as the JSON its code carries. It names the
  /// public half of `replySecret`, so a phone can post a sealed reply.
  export function makeOffer(name: string, account: string, keyA: string, nodeKey: string, replySecret: string): string
  /// Opens the EARLIEST reply in the relay's answer (`{replies:[…]}` as JSON)
  /// and answers `{code, granted_by, runners}` as JSON, or throws an Error
  /// whose message is the ceremony's word (`unopened`, `stale`, …).
  export function mergeRunnerGrants(known: string, added: string): string
  export function openCliReply(sealed: string, offer: string, replySecret: string, shownAtMs: number): string
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
    /// Called once if this connection goes silent for nine seconds, closes, or
    /// fails a check (crates/web/src/liveness.rs), with why (`probe-timeout`,
    /// `silence`, `socket-close`) and how long nothing had arrived. Not called
    /// when the runner is freed.
    onLost: (cause: string, idleMs: number) => void,
    writerKey?: string,
  ): Promise<WebRunner>
  export function build(): string
  /// The plan's view model (crates/client's `plan_view`): a request as JSON
  /// ({schema, plan, context}), the answer as JSON ({schema, view,
  /// error}). Pure; no I/O.
  export function recoveryView(count: number, current: boolean): string
  export function planView(request: string): string
  export function workspacePageAvailable(page: string, workspace: boolean, ticket: boolean): boolean
  export function workspaceLayout(request: string): string
  export function workspaceOrchestrator(plan: string): string | undefined
  export function runnerHealth(runner: string, connection: string): string
  export function needsAttention(count: number): string | undefined
  export function needsYouView(request: string): string
  export function workView(inventory: string, selection: string): string
  export function reviewFailure(word: string, code: string, what: string): string
  export function reviewApprovalResult(raw: string): string
  export function boardFailure(word: string, code: string, what: string): string
  export function ticketForm(): string
  export function workFailure(word: string, code: string, what: string): string
  export class WebRunner {
    /// Tell the runner this is a sign-out; how long nothing had arrived, ms.
    signing_out(): number
    terminal_writer_supported(): boolean
    terminal_writer(terminal: string, action: string): Promise<string>
    web_review(workspace: string, ticket: string, request: string): Promise<string>
    board(workspace: string, repository: string): Promise<string>
    board_ticket(workspace: string, task: string): Promise<string>
    board_intent(workspace: string, intent: string): Promise<string>
    runtime_health(): Promise<string>
    review_files(worktree: string): Promise<string>
    review_patch(worktree: string, path: string): Promise<string>
    work_inventory(): Promise<string>
    work_intent(intent: string): Promise<string | undefined>
    workflow_call(method: string, args: string): Promise<string>
    terminals(): Promise<string>
    attach(terminal: string, output: (bytes: Uint8Array) => void, ended: () => void): Promise<WebPane>
    /// Needs You in the phones' shape (crates/client's needs_you_json).
    needsYou(): Promise<string>
    /// Answer an ask Needs You lists (its `ask_id`, the action's `id`). A
    /// refusal rejects with message `refused` and the runner's `code` and
    /// `what`.
    answer(terminal: string, ask: string, option: string): Promise<void>
    /// Answer a decision with the option's text, on its task.
    answerDecision(task: string, body: string): Promise<void>
    /// The plan (plan.ts): `plan_json`, the shape `farcooler plan --json` prints.
    plan(workspace: string): Promise<string>
    boardOrder(workspace: string): Promise<string | undefined>
    boardOrderLabel(): string | undefined
    reorderTask(task: string, anchor: string, after: boolean): Promise<void>
    /// `workspace.list` as `workspace_json`.
    workspaces(): Promise<string>
    /// The plan's edits, each the CLI's own RPC. A refusal rejects with message
    /// `refused` and the runner's `code` and `what`.
    setThemeNext(theme: string, next: string): Promise<void>
    keepRuling(ruling: string): Promise<void>
    keepAllRulings(workspace: string): Promise<number>
    /// An agent's rows in the apps' shape (crates/client's rows_json): the
    /// newest, or up to `limit` before row `before`; and what changed after a
    /// revision, held by the runner while nothing has.
    agentRows(terminal: string, before: number | undefined, limit: number, agent?: string): Promise<string>
    agentRowsFollow(terminal: string, epoch: number, afterRev: number, waitMs: number, agent?: string): Promise<string>
    /// A file an agent sent (`agent.file`), and a prompt's image (`agent.image`).
    agentFile(terminal: string, row: string, index: number, agent?: string): Promise<Uint8Array>
    agentImage(terminal: string, row: string, index: number): Promise<Uint8Array>
    /// Send a message as the person's; true when the agent was busy and it waits.
    compose(terminal: string, text: string): Promise<boolean>
    /// `compose`, with images (`{mime, data}`) typed into the agent's box
    /// too. A refusal for size rejects with message `refused` and `what` of
    /// `image_too_large` or `images_too_large`.
    composeImages(terminal: string, text: string, images: { mime: string; data: Uint8Array }[]): Promise<boolean>
    /// What the composer may send, as `{images, each, total}` JSON.
    composeLimits(): string
    /// One task with its record (`farcooler task show --json`), as JSON.
    task(task: string): Promise<string>
    /// Add a note to a task as the person's.
    addNote(task: string, body: string): Promise<void>
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
