/// An agent's prose, read as the small Markdown it writes (ov-776): paragraphs,
/// bullet and numbered lists, headings, fenced code, and `code` and **bold**
/// inside a line. Nothing else is interpreted, and nothing becomes markup: the
/// result is a plain tree the view turns into elements with `textContent`, as
/// the page's Trusted Types require, so a link or a tag in a reply is only
/// ever text.

export type Span = { text: string; code?: boolean; bold?: boolean }

export type Block =
  | { kind: 'paragraph'; spans: Span[] }
  | { kind: 'heading'; spans: Span[] }
  | { kind: 'list'; ordered: boolean; items: Span[][] }
  | { kind: 'code'; text: string }

/// One line's `code` and **bold** spans. An unclosed marker is its own text.
export function spansOf(line: string): Span[] {
  const spans: Span[] = []
  const pattern = /`([^`\n]+)`|\*\*([^*\n]+)\*\*/g
  let at = 0
  for (let m = pattern.exec(line); m; m = pattern.exec(line)) {
    if (m.index > at) spans.push({ text: line.slice(at, m.index) })
    spans.push(m[1] !== undefined ? { text: m[1], code: true } : { text: m[2], bold: true })
    at = m.index + m[0].length
  }
  if (at < line.length) spans.push({ text: line.slice(at) })
  return spans
}

export function blocksOf(text: string): Block[] {
  const blocks: Block[] = []
  const lines = text.replace(/\r\n?/g, '\n').split('\n')
  let paragraph: string[] = []
  let list: { ordered: boolean; items: Span[][] } | null = null
  const flush = () => {
    if (paragraph.length) blocks.push({ kind: 'paragraph', spans: spansOf(paragraph.join('\n')) })
    paragraph = []
  }
  const endList = () => {
    if (list) blocks.push({ kind: 'list', ...list })
    list = null
  }
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    if (/^\s*```/.test(line)) {
      flush()
      endList()
      const code: string[] = []
      for (i++; i < lines.length && !/^\s*```/.test(lines[i]); i++) code.push(lines[i])
      blocks.push({ kind: 'code', text: code.join('\n') })
      continue
    }
    const bullet = /^\s*(?:[-*•]|(\d+)[.)])\s+(.*)$/.exec(line)
    if (bullet) {
      flush()
      const ordered = bullet[1] !== undefined
      if (list && list.ordered !== ordered) endList()
      list ??= { ordered, items: [] }
      list.items.push(spansOf(bullet[2]))
      continue
    }
    endList()
    const heading = /^#{1,6}\s+(.*)$/.exec(line)
    if (heading) {
      flush()
      blocks.push({ kind: 'heading', spans: spansOf(heading[1]) })
    } else if (line.trim() === '') flush()
    else paragraph.push(line)
  }
  flush()
  endList()
  return blocks
}
