/// The Tailcat module (crates/tailcat/go, built for GOOS=js): Go's runtime
/// starts it, and it publishes `globalThis.fcTailcat` once it is ready.

import 'fc-go-runtime'

/// A dialed tunnel connection, as crates/tailcat/go/main_js.go hands it over.
export interface TunnelConn {
  read(): Promise<Uint8Array | null>
  write(bytes: Uint8Array): void
  close(): void
}

export interface Tailcat {
  /// A fresh node key pair in this product's 43-character encoding.
  mintNodeKey(): { priv: string; pub: string }
  /// Rejects with an Error whose message is `derp`, `no_answer` or `io`.
  dial(token: string, nodeKey: string): Promise<TunnelConn>
}

declare const Go: new () => { importObject: WebAssembly.Imports; run(instance: WebAssembly.Instance): Promise<void> }

/// Start the module and answer its API once it says it is ready.
export async function loadTailcat(url: string): Promise<Tailcat> {
  const go = new Go()
  const ready = new Promise<void>(resolve => {
    ;(globalThis as any).fcTailcatReady = resolve
  })
  const { instance } = await WebAssembly.instantiateStreaming(fetch(url), go.importObject)
  void go.run(instance)
  await ready
  return (globalThis as any).fcTailcat as Tailcat
}
