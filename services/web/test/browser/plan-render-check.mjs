#!/usr/bin/env node
// The plan renderer draws the view model's column slots (ov-1016): over the
// golden view (test/fixtures/plan-view-apple.json) every fact, pipeline group
// and agent, ticket, initiative and footer count the view says is on the page,
// in the view's order, and nothing sorts or words it again. Chromium comes from playwright-core (CHROME_PATH).
//
//   node test/browser/plan-render-check.mjs

import { readFileSync } from 'node:fs'

import { build } from 'esbuild'
import { chromium } from 'playwright-core'

const bundle = await build({ entryPoints: ['test/browser/plan-render-harness.ts'], bundle: true, write: false, format: 'esm', target: 'es2022', logLevel: 'error' })
const css = readFileSync('src/client/app.css', 'utf8')
const html = readFileSync('src/client/index.html', 'utf8')
  .replace('<link rel="stylesheet" href="{{app.css}}">', `<style>${css}</style>`)
  .replace('<script type="module" src="{{app.js}}"></script>', '')
  .replace('</body>', `<script type="module">${bundle.outputFiles[0].text}</script></body>`)

const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH })
const page = await browser.newPage({ viewport: { width: 360, height: 800 } })
await page.setContent(html)
await page.waitForFunction(() => window.done === true)
const failures = await page.evaluate(() => {
  const v = window.golden
  const out = []
  const root = document.getElementById('plan')
  const text = el => el?.textContent?.replace(/\s+/g, ' ').trim() ?? ''
  const has = (what, words, squashed = false) => {
    const drawn = squashed ? text(root).replace(/\s+/g, '') : text(root)
    if (words && !drawn.includes(words)) out.push(`${what}: "${words}" isn't drawn`)
  }
  const c = v.column
  const parts = sel => [...root.querySelectorAll(sel)].map(text)
  // The facts line: each fact whole, in the view's order.
  const facts = parts('.plan-fact')
  if (JSON.stringify(facts) !== JSON.stringify(c.facts.map(f => f.text))) out.push(`facts ${JSON.stringify(facts)} != ${JSON.stringify(c.facts.map(f => f.text))}`)
  c.facts.forEach((f, i) => {
    if (f.attention !== root.querySelectorAll('.plan-fact')[i]?.classList.contains('plan-fact-amber')) out.push(`fact "${f.text}" amber`)
  })
  // The pipeline: a heading and count per group, in the view's order, each agent by name, each ticket by key and title.
  const groups = parts('.plan-stage > h3').map(t => t.replace(/\s*\d+$/, ''))
  if (JSON.stringify(groups) !== JSON.stringify(c.pipeline.map(g => g.title))) out.push(`pipeline groups ${JSON.stringify(groups)}`)
  has('pipeline heading', `Pipeline${c.agentWords}`.replace(/\s+/g, ''), true)
  for (const g of c.pipeline) {
    const stage = root.querySelector(`.plan-stage[data-stage="${g.stage}"]`)
    if (!stage) continue
    if (g.unattendedWords && !text(stage).includes(g.unattendedWords)) out.push(`${g.title}: "${g.unattendedWords}"`)
    for (const a of g.agents) {
      const card = stage.querySelector(`[data-agent="${CSS.escape(a.id)}"]`)
      if (!card) out.push(`${a.name} has no card`)
      else for (const words of [a.name, a.doing, a.time, ...a.tickets.flatMap(t => [t.key, t.title, t.initiative])]) if (words && !text(card).includes(words)) out.push(`${a.name}: "${words}"`)
    }
    for (const t of g.unattended) if (g.agents.length === 0 && !text(stage).includes(t.title)) out.push(`${g.title}: ${t.key} "${t.title}"`)
  }
  // Initiatives, then the footer's words.
  for (const i of [...c.initiatives, ...c.quietInitiatives]) for (const words of [i.name, i.progress, i.next]) if (words && !(c.quietInitiatives.includes(i) ? c.quietLine : text(root)).includes(words)) out.push(`initiative ${i.name}: "${words}"`)
  for (const i of c.initiatives) for (const words of [i.name, i.progress, i.next]) has(`initiative ${i.name}`, words)
  const footer = parts('.plan-count-link')
  if (JSON.stringify(footer) !== JSON.stringify(c.footer.map(f => f.words))) out.push(`footer ${JSON.stringify(footer)}`)
  if (document.documentElement.scrollWidth > innerWidth) out.push('overflows horizontally at 360 px')
  return out
})
await browser.close()
if (failures.length) {
  console.error(failures.join('\n'))
  process.exit(1)
}
console.log('plan render check: every slot of the golden view is drawn')
