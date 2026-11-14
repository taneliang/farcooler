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
      if (c.roster.length) same('roster heading', text(root.querySelector('.plan-pipeline h2')), `${c.rosterTitle}${c.agentWords}`)
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
  // ov-1105: the current Plan renderer keeps a queue-only plan reachable and
  // wraps every supplied member/failure line at a narrow phone width.
  const page = await browser.newPage({ viewport: { width: 320, height: 800 } })
  await page.setContent(html)
  await page.waitForFunction(() => window.done === true)
  await page.evaluate(() => { document.getElementById('plan').hidden = true })
  failures.push(...await page.evaluate(() => {
    const out = []
    const root = document.getElementById('landing-queue')
    const golden = window.queueGolden
    if (JSON.stringify(window.queueRequest.plan.workflow) !== JSON.stringify(golden.request.plan.workflow)) out.push('queue workflow did not reach the presenter')
    if (root.querySelector('h2')?.textContent !== 'Landings') out.push('queue-only plan has no Landings section')
    const cards = [...root.querySelectorAll('.plan-landing-group')]
    if (cards.length !== golden.expected.length) out.push(`queue draws ${cards.length} groups`)
    golden.expected.forEach((group, index) => {
      const card = cards[index]
      if (!card) return
      if (card.getBoundingClientRect().height === 0) out.push(`${group.id}: group is hidden`)
      if (card.dataset.testid !== `plan-landing-group-${group.id}` || card.querySelector('h3')?.textContent !== group.title) out.push(`queue group ${index} differs`)
      const lines = [...card.querySelectorAll('p')]
      if (JSON.stringify(lines.map(p => p.textContent)) !== JSON.stringify(group.lines)) out.push(`${group.id}: incomplete or reordered lines`)
      for (const line of lines) {
        const range = document.createRange()
        range.selectNodeContents(line)
        for (const rect of range.getClientRects()) {
          if (rect.left < 0 || rect.right > innerWidth + 0.5) out.push(`${group.id}: line overflows at 320 px`)
        }
        if (line.scrollHeight > line.clientHeight) out.push(`${group.id}: line is clipped`)
      }
    })
    if (document.documentElement.scrollWidth > innerWidth) out.push('queue overflows horizontally at 320 px')
    return out
  }))
  await page.close()
} finally { await browser.close() }
if (failures.length) { console.error(failures.join('\n')); process.exit(1) }
console.log('plan render check: golden and ov-895 pipeline pass at 360 and 1440 px; landing queue lines pass at 320 px')
