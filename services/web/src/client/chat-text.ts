import MarkdownIt, { type Token } from 'markdown-it'

// HTML tokens let us preserve raw blocks, including script bodies, verbatim.
// Only this DOM renderer sees the tokens. It never uses markdown-it's HTML renderer.
const markdown = MarkdownIt({ html: true })

// markdown-it drops a `file:` destination before it is a token. Let it be one,
// so an image the agent names that way can be matched with what it sent. No
// `file:` URL is ever put on the page: a link passes `safeLink`, and an image
// is only ever drawn from the runner's bytes.
const allowed = markdown.validateLink.bind(markdown)
markdown.validateLink = url => /^file:/i.test(url) || allowed(url)

/// The element for an image whose destination is `src`, or null to leave its
/// alt text.
export type ImageFor = (src: string, alt: string) => HTMLElement | null

const unescaped = (text: string) => {
  try {
    return decodeURI(text)
  } catch {
    return text
  }
}

/// Whether two destinations are one: markdown-it percent-encodes what it reads.
export const sameDestination = (a: string, b: string) => unescaped(a) === unescaped(b)

const tags = new Map<string, keyof HTMLElementTagNameMap>([
  ['paragraph_open', 'p'], ['blockquote_open', 'blockquote'],
  ['bullet_list_open', 'ul'], ['ordered_list_open', 'ol'], ['list_item_open', 'li'],
  ['strong_open', 'strong'], ['em_open', 'em'], ['s_open', 's'],
  ['table_open', 'table'], ['thead_open', 'thead'], ['tbody_open', 'tbody'],
  ['tr_open', 'tr'], ['th_open', 'th'], ['td_open', 'td'],
])

function safeLink(href: string): URL | null {
  // Use the browser's URL parser before checking the scheme. Relative links
  // have no document base in a transcript from a runner.
  try {
    const url = new URL(href)
    return ['https:', 'http:', 'mailto:'].includes(url.protocol) ? url : null
  } catch {
    return null
  }
}

function openElement(token: Token): HTMLElement {
  if (token.type === 'link_open') {
    const href = token.attrGet('href')
    const url = typeof href === 'string' ? safeLink(href) : null
    if (!url) return document.createElement('span')
    const link = document.createElement('a')
    link.href = url.href
    link.target = '_blank'
    link.relList.add('noopener', 'noreferrer')
    const title = token.attrGet('title')
    if (typeof title === 'string') link.title = title
    return link
  }

  if (token.type === 'heading_open' && /^h[1-6]$/.test(token.tag)) {
    const heading = document.createElement(token.tag as `h${1 | 2 | 3 | 4 | 5 | 6}`)
    heading.className = 'prose-heading'
    return heading
  }

  const tag = tags.get(token.type)
  const node = document.createElement(tag ?? 'div')
  if (node instanceof HTMLOListElement) {
    const start = token.attrGet('start')
    if (start) node.start = Number(start)
  }
  if (node instanceof HTMLTableCellElement) {
    if (tag === 'th') node.scope = 'col'
    const alignment = token.attrGet('style')
    if (alignment === 'text-align:center') node.className = 'prose-center'
    if (alignment === 'text-align:right') node.className = 'prose-right'
  }
  return node
}

function appendTokens(parent: HTMLElement, tokens: readonly Token[], imageFor?: ImageFor): void {
  const stack = [parent]
  for (const token of tokens) {
    if (token.hidden) continue // Tight lists omit the paragraph around each item.
    const current = stack[stack.length - 1]
    if (token.nesting === 1) {
      const node = openElement(token)
      if (token.type === 'table_open') {
        const scroll = document.createElement('div')
        scroll.className = 'prose-table'
        scroll.tabIndex = 0
        scroll.append(node)
        current.append(scroll)
      } else current.append(node)
      stack.push(node)
    } else if (token.nesting === -1) {
      stack.pop()
    } else {
      switch (token.type) {
        case 'inline':
          appendTokens(current, token.children ?? [], imageFor)
          break
        case 'fence':
        case 'code_block': {
          const pre = document.createElement('pre')
          pre.className = 'code-block'
          pre.tabIndex = 0
          const code = document.createElement('code')
          code.textContent = token.content
          pre.append(code)
          current.append(pre)
          break
        }
        case 'code_inline': {
          const code = document.createElement('code')
          code.textContent = token.content
          current.append(code)
          break
        }
        case 'html_block': {
          const literal = document.createElement('div')
          literal.className = 'prose-literal'
          literal.textContent = token.content
          current.append(literal)
          break
        }
        case 'html_inline':
        case 'text':
          current.append(document.createTextNode(token.content))
          break
        case 'image': {
          // Never loads an agent-supplied URL: either the runner's own bytes
          // for a file the agent sent, or the alt text.
          const src = token.attrGet('src')
          const drawn = typeof src === 'string' ? imageFor?.(src, token.content) : null
          current.append(drawn ?? document.createTextNode(token.content))
          break
        }
        case 'softbreak': // Agents use single newlines to separate thoughts.
        case 'hardbreak':
          current.append(document.createElement('br'))
          break
        case 'hr':
          current.append(document.createElement('hr'))
          break
      }
    }
  }
}

/// Render untrusted chat Markdown using fixed DOM elements and text nodes.
export function prose(text: string, imageFor?: ImageFor): HTMLElement {
  const box = document.createElement('div')
  box.className = 'prose'
  appendTokens(box, markdown.parse(text, {}), imageFor)
  return box
}
