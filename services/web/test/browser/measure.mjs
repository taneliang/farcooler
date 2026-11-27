// Typing and scrolling, measured in the page (ov-1510): how long from a key
// or wheel event to the frame that shows its effect, and how long frames take
// while scrolling. csp-check.mjs --measure FILE runs these on the real app's
// orchestrator chat and terminal; chat-check.mjs --measure FILE on its long
// transcript. Run the same code against two builds to compare them.
//
// "To the frame" is the event's own timestamp (when the browser received the
// input) to a task queued from the next requestAnimationFrame: the frame that
// paints whatever the handlers changed has then been produced. It counts the
// page's handlers, style, layout and paint, not the display's scan-out.

const stats = values => {
  const sorted = [...values].sort((a, b) => a - b)
  const at = q => sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))]
  const round = v => Math.round(v * 10) / 10
  return { n: sorted.length, median: round(at(0.5)), p95: round(at(0.95)), max: round(sorted.at(-1) ?? 0) }
}

/// Type `text` one key at a time into what `target` focuses, `gap` ms apart.
export async function typing(page, target, text, gap = 40) {
  await target.focus()
  await page.evaluate(() => {
    window.__input = []
    window.__inputListener ??= addEventListener('keydown', event => {
      const start = event.timeStamp
      requestAnimationFrame(() => setTimeout(() => window.__input?.push(performance.now() - start)))
    }, { capture: true })
  })
  for (const key of text) {
    await page.keyboard.press(key === '\n' ? 'Enter' : key)
    await page.waitForTimeout(gap)
  }
  await page.waitForTimeout(300)
  return stats(await page.evaluate(() => window.__input))
}

/// Wheel over `target` `ticks` times, down then up, recording each wheel's
/// time to its frame and every frame's length meanwhile.
export async function scrolling(page, target, ticks = 60, gap = 16) {
  const box = await target.boundingBox()
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await page.evaluate(() => {
    window.__wheel = []
    window.__frames = []
    window.__scrolling = true
    let last = performance.now()
    const frame = now => {
      window.__frames.push(now - last)
      last = now
      if (window.__scrolling) requestAnimationFrame(frame)
    }
    requestAnimationFrame(frame)
    window.__wheelListener ??= addEventListener('wheel', event => {
      const start = event.timeStamp
      requestAnimationFrame(() => setTimeout(() => window.__wheel?.push(performance.now() - start)))
    }, { capture: true, passive: true })
  })
  for (let i = 0; i < ticks; i++) {
    await page.mouse.wheel(0, i < ticks / 2 ? 120 : -120)
    await page.waitForTimeout(gap)
  }
  await page.waitForTimeout(300)
  const { wheel, frames } = await page.evaluate(() => {
    window.__scrolling = false
    return { wheel: window.__wheel, frames: window.__frames.slice(1) }
  })
  return { wheelToFrame: stats(wheel), frame: stats(frames) }
}
