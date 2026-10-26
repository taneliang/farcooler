/// The two wasm modules' JavaScript faces, as the page uses them. The build
/// (scripts/build.mjs) points these module names at the files
/// crates/web/build.sh made: `fc-web` is crates/web's wasm-bindgen glue, and
/// `fc-go-runtime` is Go's wasm_exec.js, imported for the `Go` class it
/// defines on globalThis.

declare module 'fc-web' {
  export default function init(options: { module_or_path: string }): Promise<unknown>
  /// The offer this browser shows, as the JSON its code carries.
  export function makeOffer(name: string, account: string, keyA: string, nodeKey: string): string
  /// `{granted_by, runners}` as JSON, or throws an Error whose message is the
  /// ceremony's word (`stale` once the code is past the window).
  export function acceptReply(reply: string, offer: string, alreadyTaken: boolean, shownAtMs: number): string
  export function offerIsStale(shownAtMs: number): boolean
  export function freshnessMs(): number
  export function publicKeyLine(raw: Uint8Array, comment: string): string
  export function fingerprint(publicKey: string): string | undefined
  export function connect(
    conn: import('./tunnel').TunnelConn,
    hostKey: string,
    user: string,
    keyA: string,
    sign: (data: Uint8Array) => Promise<ArrayBuffer>,
  ): Promise<WebRunner>
  export function build(): string
  export class WebRunner {
    terminals(): Promise<string>
    attach(terminal: string, output: (bytes: Uint8Array) => void, ended: () => void): Promise<WebPane>
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
