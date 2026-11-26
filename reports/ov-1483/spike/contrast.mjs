import { readFileSync } from 'node:fs'
import assert from 'node:assert/strict'

const css = readFileSync(new URL('tokens.css', import.meta.url), 'utf8')
const colors = section => Object.fromEntries([...section.matchAll(/--([\w-]+): (#\w{6});/g)].map(([, key, value]) => [key, value]))
const [light, overrides] = css.split('[data-theme="dark"]')
const palettes = [colors(light), { ...colors(light), ...colors(overrides) }]
const luminance = hex => hex.slice(1).match(/../g).map(value => parseInt(value, 16) / 255)
  .map(value => value <= .04045 ? value / 12.92 : ((value + .055) / 1.055) ** 2.4)
  .reduce((sum, value, i) => sum + [.2126, .7152, .0722][i] * value, 0)
const pairs = [
  ['text', 'canvas', 4.5], ['text', 'surface', 4.5], ['text', 'rail', 4.5],
  ['muted', 'canvas', 4.5], ['muted', 'surface', 4.5], ['muted', 'rail', 4.5],
  ['on-accent', 'accent', 4.5], ['on-accent', 'accent-hover', 4.5], ['selected-text', 'selected', 4.5],
  ['good', 'good-bg', 4.5], ['attention', 'attention-bg', 4.5],
  ['terminal-text', 'terminal', 4.5], ['terminal-positive', 'terminal', 4.5],
  ['control-edge', 'surface', 3], ['control-edge', 'rail', 3], ['control-edge', 'canvas', 3],
  ['accent', 'canvas', 3], ['accent', 'rail', 3], ['accent', 'surface', 3],
  ['focus', 'canvas', 3], ['focus', 'rail', 3], ['focus', 'surface', 3], ['focus', 'selected', 3],
]
console.log('| Pair | Light | Dark | Minimum |\n|---|---:|---:|---:|')
for (const [fg, bg, min] of pairs) {
  const ratios = palettes.map(palette => {
    const [lo, hi] = [luminance(palette[fg]), luminance(palette[bg])].sort((a, b) => a - b)
    return (hi + .05) / (lo + .05)
  })
  assert(ratios.every(ratio => ratio >= min), fg + ' against ' + bg)
  console.log(`| ${fg} on ${bg} | ${ratios[0].toFixed(2)} | ${ratios[1].toFixed(2)} | ${min} |`)
}
