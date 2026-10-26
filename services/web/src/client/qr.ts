/// The two QR codes: the offer this browser shows, and the reply it reads
/// through the camera.
///
/// Both libraries are bundled from the lockfile (qrcode-generator to draw,
/// jsQR to read), so nothing loads from elsewhere at runtime. jsQR rather
/// than the browser's BarcodeDetector, which Windows and Linux lack.

import jsQR from 'jsqr'
import qrcode from 'qrcode-generator'

/// Draw `text` as a QR code on `canvas`, with a quiet zone.
export function drawCode(canvas: HTMLCanvasElement, text: string): void {
  const code = qrcode(0, 'M')
  // Byte mode reads each character's low byte, so hand it UTF-8 one byte per
  // character.
  code.addData(String.fromCharCode(...new TextEncoder().encode(text)), 'Byte')
  code.make()
  const modules = code.getModuleCount()
  const quiet = 4
  const scale = Math.max(2, Math.floor(320 / (modules + 2 * quiet)))
  const size = (modules + 2 * quiet) * scale
  canvas.width = size
  canvas.height = size
  const g = canvas.getContext('2d')
  if (!g) return
  g.fillStyle = '#fff'
  g.fillRect(0, 0, size, size)
  g.fillStyle = '#000'
  for (let row = 0; row < modules; row++) {
    for (let col = 0; col < modules; col++) {
      if (code.isDark(row, col)) g.fillRect((col + quiet) * scale, (row + quiet) * scale, scale, scale)
    }
  }
}

/// Read QR codes from the camera until `found` says one is the right kind, or
/// `stop()` is called. Rejects with `camera` when there is no camera to use.
export async function scan(video: HTMLVideoElement, found: (text: string) => boolean): Promise<{ stop: () => void }> {
  let stream: MediaStream
  try {
    stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: 'environment' }, audio: false })
  } catch {
    throw new Error('camera')
  }
  video.srcObject = stream
  await video.play().catch(() => {})
  const frame = document.createElement('canvas')
  const g = frame.getContext('2d', { willReadFrequently: true })
  let running = true
  const stop = () => {
    running = false
    for (const track of stream.getTracks()) track.stop()
    video.srcObject = null
  }
  const look = () => {
    if (!running) return
    if (g && video.videoWidth > 0) {
      frame.width = video.videoWidth
      frame.height = video.videoHeight
      g.drawImage(video, 0, 0)
      const image = g.getImageData(0, 0, frame.width, frame.height)
      const code = jsQR(image.data, image.width, image.height, { inversionAttempts: 'attemptBoth' })
      if (code?.data && found(code.data)) {
        stop()
        return
      }
    }
    // About eight looks a second: enough to feel immediate, without a
    // decode on every frame of a laptop's battery.
    setTimeout(look, 120)
  }
  look()
  return { stop }
}
