#!/usr/bin/env node
// The real Rust projection's golden and ov-895 edge case fixture, drawn at
// phone and desktop widths. Optional first argument saves narrow UI evidence.
import { mkdirSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { build } from 'esbuild'
import { chromium } from 'playwright-core'

const out = process.argv[2] ? resolve(process.argv[2]) : null
if (out) mkdirSync(out, { recursive: true })
const bundle = await build({ entryPoints: ['test/browser/plan-render-harness.ts'], bundle: true, write: false, format: 'esm', target: 'es2022', logLevel: 'error' })
const css = readFileSync('src/client/app.css', 'utf8')
const html = readFileSync('src/client/index.html', 'utf8')
  .replace('<link rel="stylesheet" href="{{app.css}}">', `<style>${css}</style>`)
  .replace('<script type="module" src="{{app.js}}"></script>', '')
  .replace('</body>', `<script type="module">${bundle.outputFiles[0].text}</script></body>`)
const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH })
const failures = []
try {
  for (const focused of [false, true]) for (const width of [360, 1440]) {
    const page = await browser.newPage({ viewport: { width, height: 1100 } })
    await page.setContent(html.replace('<head>', `<head><script>window.focused=${focused}</script>`))
    await page.waitForFunction(() => window.done === true)
    const errors = await page.evaluate(() => {
      const c = window.golden.column
      const root = document.getElementById('plan')
      const out = []
      if (!root.checkVisibility() || root.getBoundingClientRect().width === 0) out.push('plan is hidden')
      for (const row of root.querySelectorAll('[data-agent], [data-task]')) {
        if (!row.checkVisibility() || row.getBoundingClientRect().height === 0) out.push(`hidden row ${row.dataset.agent ?? row.dataset.task}`)
      }
      const text = el => el?.textContent?.replace(/\s+/g, ' ').trim() ?? ''
      const same = (what, actual, expected) => {
        if (JSON.stringify(actual) !== JSON.stringify(expected)) out.push(`${what}: ${JSON.stringify(actual)} != ${JSON.stringify(expected)}`)
      }
      same('facts', [...root.querySelectorAll('.plan-fact')].map(text), c.facts.map(f => f.text))
      const stages = [...root.querySelectorAll('.plan-stage')]
      same('stage order', stages.map(s => s.dataset.stage), c.pipeline.map(g => g.stage))
      const cards = [...root.querySelectorAll('[data-agent]')]
      same('unique roster', cards.map(a => a.dataset.agent), c.roster.map(a => a.id))
      if (cards.some(a => a.querySelector('[data-task]'))) out.push('ticket nested under an agent')
      for (const a of c.roster) {
        const card = cards.find(el => el.dataset.agent === a.id)
        for (const words of [a.name, a.title, a.doing, a.time, a.assignmentWords]) {
          if (words && !text(card).includes(words)) out.push(`agent ${a.id}: missing ${words}`)
        }
      }
      for (const g of c.pipeline) {
        const stage = stages.find(s => s.dataset.stage === g.stage)
        if (!stage) { out.push(`missing stage ${g.stage}; found ${stages.map(s => s.dataset.stage)}`); continue }
        if (text(stage.querySelector('h3')) !== `${g.title}${g.countWords}`) out.push(`stage count ${g.stage}`)
        const tickets = [...g.assigned, ...g.unattended]
        same(`${g.stage} membership`, [...stage.querySelectorAll('[data-task]')].map(t => t.dataset.task), tickets.map(t => t.task))
        if (g.count !== tickets.length) out.push(`${g.stage} ticket count differs from membership`)
        for (const t of tickets) {
          const row = [...stage.querySelectorAll('[data-task]')].find(el => el.dataset.task === t.task)
          for (const words of [t.key, t.title, t.detail, t.initiative]) if (words && !text(row).includes(words)) out.push(`ticket ${t.task}: missing ${words}`)
          if (t.detail && !row.getAttribute('aria-label').includes(t.detail)) out.push(`ticket ${t.task}: missing accessible context`)
        }
        if (g.unattendedWords && !text(stage).includes(g.unattendedWords)) out.push(`${g.stage}: missing unassigned count`)
      }
      const rows = [...root.querySelectorAll('.plan-stage [data-task]')].map(t => t.dataset.task)
      if (new Set(rows).size !== rows.length) out.push('duplicate pipeline ticket')
      for (const i of c.initiatives) for (const words of [i.name, i.progress, i.next]) if (words && !text(root).includes(words)) out.push(`initiative ${i.name}: ${words}`)
      same('footer', [...root.querySelectorAll('.plan-count-link')].map(text), c.footer.map(f => f.words))
      if (document.documentElement.scrollWidth > innerWidth) out.push('horizontal overflow')
      return out
    })
    failures.push(...errors.map(e => `${focused ? 'focused' : 'golden'} ${width}: ${e}`))
    if (out && focused) {
      await page.setViewportSize({ width, height: Math.max(1100, await page.evaluate(() => document.getElementById('plan').scrollHeight + 120)) })
      await page.screenshot({ path: join(out, `pipeline-${width}.png`), fullPage: true })
    }
    await page.close()
  }
} finally { await browser.close() }
if (failures.length) { console.error(failures.join('\n')); process.exit(1) }
console.log('plan render check: golden and ov-895 membership, counts, activity and accessibility pass at 360 and 1440 px')
