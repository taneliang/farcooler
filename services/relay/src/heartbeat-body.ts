/// How big a runner's heartbeat may be (ov-510), and a body read that stops
/// at a limit.

/// What a heartbeat may be, in bytes. The read stops at `HEARTBEAT_READ_MAX`;
/// the inventory beyond `HEARTBEAT_INVENTORY_MAX` is dropped (the daemon sends
/// at most 256 KB of it, by serialized size); everything else in the beat is
/// a name, a version and a few numbers, and `HEARTBEAT_BEAT_MAX` is generous.
export const HEARTBEAT_READ_MAX = 2 * 1024 * 1024
export const HEARTBEAT_INVENTORY_MAX = 300 * 1024
export const HEARTBEAT_BEAT_MAX = 16 * 1024

/// The request body as text, or `null` if it is longer than `max` bytes. Stops
/// reading at the limit; a `Content-Length` over it is refused unread.
export async function readLimited(request: Request, max: number): Promise<string | null> {
  const declared = Number(request.headers.get('content-length'))
  if (Number.isFinite(declared) && declared > max) return null
  if (!request.body) return ''
  const reader = request.body.getReader()
  const decoder = new TextDecoder()
  let text = ''
  let size = 0
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    size += value.byteLength
    if (size > max) {
      await reader.cancel()
      return null
    }
    text += decoder.decode(value, { stream: true })
  }
  return text + decoder.decode()
}
