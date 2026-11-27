// What a view must hold to in the new shell (ov-1510): at most one filled
// primary button, and text that meets WCAG AA contrast. Run in the page.

/// The visible buttons and links filled with the accent, by their words.
export const filledButtons = page => page.evaluate(() => {
  const probe = document.createElement('span')
  probe.style.background = 'var(--accent)'
  document.body.append(probe)
  const accent = getComputedStyle(probe).backgroundColor
  probe.remove()
  return [...document.querySelectorAll('button, a, [role="button"]')]
    .filter(el => el.checkVisibility({ visibilityProperty: true, opacityProperty: true }) && el.getClientRects().length)
    .filter(el => getComputedStyle(el).backgroundColor === accent)
    .map(el => (el.id ? `#${el.id} ` : '') + (el.textContent ?? '').trim().slice(0, 40))
})

/// The WCAG contrast of each pair: [what, ratio, minimum], from the colors the
/// page actually computed for its own elements and tokens.
export const contrastPairs = page => page.evaluate(() => {
  const rgb = value => {
    const probe = document.createElement('span')
    probe.style.color = value
    document.body.append(probe)
    const parts = getComputedStyle(probe).color.match(/[\d.]+/g).map(Number)
    probe.remove()
    return parts.slice(0, 3)
  }
  const token = name => rgb(`var(${name})`)
  const lum = ([r, g, b]) => {
    const c = v => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4 }
    return 0.2126 * c(r) + 0.7152 * c(g) + 0.0722 * c(b)
  }
  const ratio = (a, b) => {
    const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p)
    return Math.round(((x + 0.05) / (y + 0.05)) * 100) / 100
  }
  const worst = (fg, backs) => Math.min(...backs.map(b => ratio(token(fg), token(b))))
  const panels = ['--canvas', '--surface', '--rail']
  return [
    ['Main text / canvas, card or rail', worst('--text', panels), 4.5],
    ['Muted text / canvas, card or rail', worst('--muted', panels), 4.5],
    ['Primary action label / fill', worst('--on-accent', ['--accent']), 4.5],
    ['Primary action label / hover fill', worst('--on-accent', ['--accent-hover']), 4.5],
    ['Link and accent text / canvas, card or rail', worst('--accent', panels), 4.5],
    ['Selected text / selection', worst('--selected-text', ['--selected']), 4.5],
    ['Connected text / status surface', worst('--good', ['--good-bg']), 4.5],
    ['Lost-session text / status surface', worst('--attention', ['--attention-bg']), 4.5],
    ['Danger text / canvas, card or rail', worst('--danger', panels), 4.5],
    ['Control edge / canvas, card or rail', worst('--control-edge', panels), 3],
    ['Focus ring / canvas, card, rail or selection', worst('--focus', [...panels, '--selected']), 3],
    ['Terminal text / terminal surface', worst('--terminal-text', ['--terminal']), 4.5],
  ]
})
