/// What an agent sends, and the images a message carries (ov-1509). A sent
/// image is drawn inline and opens full size; any other file is a row with its
/// name, size and Download. The runner says which files were sent, how big
/// each is, whether it is a picture and whether to draw it unasked
/// (`session_log::projector::sent_files`); this file only draws that.
///
/// Bytes come from the runner over the page's own connection (`agent.file`,
/// `agent.image`), named by a row and a place on it, and are shown from a
/// `blob:` URL. Nothing here builds a URL from anything an agent wrote.

import { fileWords } from './chat-copy'

/// One file as a row names it (`SentFile`, and `ProseImage` with `src`).
export interface FileEntry {
  name: string
  mime: string
  size: number
  size_words: string
  image: boolean
  auto: boolean
  fetchable: boolean
}

/// What to ask the runner for: file `index` of a send, or image `index` of a
/// prompt, on row `row`.
export interface Ask {
  from: 'file' | 'image'
  row: string
  index: number
}

/// A picture to draw.
export interface Picture extends Ask {
  mime: string
  name: string
  /// Said on the button when it waits to be asked for.
  sizeWords: string
  auto: boolean
}

/// How many fetches run at once: each is a run of requests on the one
/// connection the conversation follows on too.
const AT_ONCE = 3
/// How far outside the transcript a picture is fetched ahead of the scroll.
const AHEAD = '300px'
/// The types drawn: the runner's own list, held to again here.
const DRAWN = new Set(['image/png', 'image/jpeg', 'image/gif', 'image/webp'])

const el = <K extends keyof HTMLElementTagNameMap>(tag: K, className = '', text = ''): HTMLElementTagNameMap[K] => {
  const node = document.createElement(tag)
  if (className) node.className = className
  if (text) node.textContent = text
  return node
}

const button = (className: string, text = '') => {
  const node = el('button', className, text)
  node.type = 'button'
  return node
}

/// One conversation's files: each fetched once and kept while the view lives,
/// so a row drawn again shows its picture at once.
export class Media {
  private readonly urls = new Map<string, string>()
  private readonly pending = new Map<string, Promise<string>>()
  /// Pictures asked for with a click: a redraw doesn't ask again.
  private readonly wanted = new Set<string>()
  private readonly waiting: (() => void)[] = []
  private running = 0
  private generation = 0
  private observer: IntersectionObserver | null = null
  private readonly near = new WeakMap<Element, () => void>()
  private viewer: HTMLDialogElement | null = null

  constructor(
    private readonly fetch: (ask: Ask) => Promise<Uint8Array>,
    /// The scrolling box pictures are in.
    private readonly scroller: () => Element,
    /// A picture was drawn, and the transcript is taller than it was.
    private readonly resized: () => void,
  ) {}

  private key = (ask: Ask) => [ask.from, ask.row, ask.index].join('|')

  private async queued<T>(work: () => Promise<T>): Promise<T> {
    while (this.running >= AT_ONCE) await new Promise<void>(go => this.waiting.push(go))
    this.running++
    try {
      return await work()
    } finally {
      this.running--
      this.waiting.shift()?.()
    }
  }

  /// A `blob:` URL of a picture's bytes, fetched once.
  private url(picture: Picture): Promise<string> {
    const key = this.key(picture)
    const held = this.urls.get(key)
    if (held) return Promise.resolve(held)
    let going = this.pending.get(key)
    if (!going) {
      const generation = this.generation
      going = this.queued(async () => {
        if (generation !== this.generation) throw new Error('gone')
        const bytes = await this.fetch(picture)
        if (generation !== this.generation) throw new Error('gone')
        const url = URL.createObjectURL(new Blob([bytes as BlobPart], { type: DRAWN.has(picture.mime) ? picture.mime : 'application/octet-stream' }))
        this.urls.set(key, url)
        return url
      }).finally(() => this.pending.delete(key))
      this.pending.set(key, going)
    }
    return going
  }

  /// Run `go` once `box` is on or near the screen.
  private whenNear(box: Element, go: () => void): void {
    this.observer ??= new IntersectionObserver(
      entries => {
        for (const entry of entries) {
          if (!entry.isIntersecting) continue
          this.observer?.unobserve(entry.target)
          this.near.get(entry.target)?.()
        }
      },
      { root: this.scroller(), rootMargin: AHEAD },
    )
    this.near.set(box, go)
    this.observer.observe(box)
  }

  /// A picture, inline: drawn as it comes near the screen, or behind a button
  /// saying its size where the runner says not to draw it unasked.
  picture(picture: Picture): HTMLElement {
    const key = this.key(picture)
    const words = fileWords()
    const box = el('span', 'chat-image')
    box.dataset.name = picture.name

    const show = (url: string) => {
      const open = button('chat-image-open')
      open.setAttribute('aria-label', words.open(picture.name))
      const img = el('img')
      img.alt = picture.name
      img.decoding = 'async'
      img.onload = () => {
        box.dataset.state = 'shown'
        this.resized()
      }
      // Bytes that aren't the picture their name says.
      img.onerror = () => fail()
      img.src = url
      open.append(img)
      open.onclick = () => this.view(url, picture.name)
      box.replaceChildren(open)
    }
    const fail = () => {
      const url = this.urls.get(key)
      if (url) URL.revokeObjectURL(url)
      this.urls.delete(key)
      box.dataset.state = 'failed'
      const retry = button('quiet', words.retry)
      retry.onclick = () => load()
      box.replaceChildren(el('span', 'chat-image-name', picture.name), el('span', 'chat-image-said', words.failed), retry)
      this.resized()
    }
    const load = () => {
      box.dataset.state = 'loading'
      box.replaceChildren(el('span', 'chat-image-said', words.loading))
      this.url(picture).then(show, fail)
    }

    const held = this.urls.get(key)
    if (held) show(held)
    else if (picture.auto || this.wanted.has(key)) {
      box.dataset.state = 'waiting'
      box.append(el('span', 'chat-image-name', picture.name))
      this.whenNear(box, load)
    } else {
      box.dataset.state = 'asking'
      const ask = button('quiet', words.load(picture.sizeWords))
      ask.onclick = () => {
        this.wanted.add(key)
        load()
      }
      box.append(el('span', 'chat-image-name', picture.name), ask)
    }
    return box
  }

  /// A file that isn't drawn: its name, its size, and Download.
  file(ask: Ask, entry: FileEntry): HTMLElement {
    const words = fileWords()
    const row = el('div', 'chat-file')
    const about = el('div', 'chat-file-about')
    about.append(el('span', 'chat-file-name', entry.name), el('span', 'chat-file-size', entry.size_words))
    row.append(about)
    if (!entry.fetchable) {
      about.append(el('span', 'chat-file-said', words.tooLarge))
      return row
    }
    const save = button('quiet', words.download)
    save.classList.add('chat-file-save')
    const said = el('span', 'chat-file-said')
    said.setAttribute('role', 'alert')
    save.onclick = async () => {
      save.disabled = true
      said.textContent = ''
      try {
        // Saved, never opened: an octet stream can't run as a page of ours.
        const bytes = await this.queued(() => this.fetch(ask))
        const url = URL.createObjectURL(new Blob([bytes as BlobPart], { type: 'application/octet-stream' }))
        const link = el('a')
        link.href = url
        link.download = entry.name
        link.click()
        setTimeout(() => URL.revokeObjectURL(url), 60_000)
      } catch {
        said.textContent = words.failed
      } finally {
        save.disabled = false
      }
    }
    row.append(save)
    about.append(said)
    return row
  }

  /// A picture over the page: fitted to the window, and at its own size, to
  /// scroll around, on a click of the picture or Actual Size.
  private view(url: string, name: string): void {
    this.viewer?.close()
    const words = fileWords()
    const dialog = el('dialog', 'chat-viewer')
    dialog.setAttribute('aria-label', name)
    const bar = el('div', 'chat-viewer-bar')
    const size = button('quiet', words.actual)
    const close = button('quiet', words.close)
    bar.append(el('span', 'chat-viewer-name', name), size, close)
    const stage = el('div', 'chat-viewer-stage')
    stage.tabIndex = 0
    const img = el('img')
    img.src = url
    img.alt = name
    stage.append(img)
    dialog.append(bar, stage)
    const toggle = () => {
      const full = dialog.classList.toggle('full')
      size.textContent = full ? words.fit : words.actual
    }
    size.onclick = toggle
    img.onclick = toggle
    close.onclick = () => dialog.close()
    dialog.addEventListener('close', () => {
      dialog.remove()
      if (this.viewer === dialog) this.viewer = null
    }, { once: true })
    document.body.append(dialog)
    this.viewer = dialog
    dialog.showModal()
    close.focus()
  }

  /// Let go of everything held: the view is going, or is another terminal's.
  clear(): void {
    this.generation++
    this.viewer?.close()
    this.observer?.disconnect()
    this.observer = null
    for (const url of this.urls.values()) URL.revokeObjectURL(url)
    this.urls.clear()
    this.wanted.clear()
    this.pending.clear()
    for (const go of this.waiting.splice(0)) go()
  }
}
