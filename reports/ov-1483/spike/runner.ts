type Stage = 'Ready' | 'In progress' | 'Review'
export type Card = { key: string; title: string; stage: Stage; detail: string }
export const stages: Stage[] = ['Ready', 'In progress', 'Review']

const overnight: Card[] = [
  { key: 'ov-1483', title: 'Choose a web design system', stage: 'Ready', detail: 'Compare shell options and bring one proposal to the owner.' },
  { key: 'ov-1485', title: 'Show recovery on lost terminals', stage: 'Ready', detail: 'Keep recovery beside the interrupted session.' },
  { key: 'ov-1480', title: 'Synchronize orchestrator startup', stage: 'In progress', detail: 'Verify the runner before opening the stream.' },
  { key: 'ov-1476', title: 'Review workspace navigation', stage: 'Review', detail: 'Check history and workspace identity.' },
]

export const spaces = [
  { id: 'overnight', label: 'Overnight', repository: 'farcooler / overnight' },
  { id: 'nightly', label: 'Nightly builds', repository: 'farcooler / nightly' },
  { id: 'support', label: 'Customer support', repository: 'farcooler / support' },
]

export type Snapshot = {
  status: 'Connected' | 'Session lost'
  boards: Record<string, Card[]>
}

export const transcript = [
  '\x1b[38;2;152;223;176m✓ Runner connected\x1b[0m',
  'Workspace: Overnight · main',
  '',
  'Watching the delivery board.',
  '1 ticket in progress · 1 in review',
  '',
  'Fixture commands: status, clear',
  '',
].join('\r\n') + '\r\n$ '

/** A fixture transport implementing the shell-facing seams of a runner session. */
export class RunnerSession {
  private listeners = new Set<() => void>()
  private output = new Set<(bytes: Uint8Array) => void>()
  private scrollback = transcript
  private input = ''
  private encode = new TextEncoder()
  private snapshot: Snapshot = {
    status: 'Connected',
    boards: {
      overnight,
      nightly: [{ key: 'nb-46', stage: 'In progress', title: 'Check nightly artifacts', detail: 'Validate the current release bundle.' }],
      support: [{ key: 'su-18', stage: 'Ready', title: 'Triage connection reports', detail: 'Group reports by runner and device.' }],
    },
  }

  read = () => this.snapshot

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  attach(listener: (bytes: Uint8Array) => void) {
    this.output.add(listener)
    listener(this.encode.encode(this.scrollback))
    return () => { this.output.delete(listener) }
  }

  private publish(next: Snapshot) {
    this.snapshot = next
    this.listeners.forEach(listener => listener())
  }

  private write(text: string) {
    this.scrollback = (this.scrollback + text).slice(-16000)
    const bytes = this.encode.encode(text)
    this.output.forEach(listener => listener(bytes))
  }

  start(space: string, key: string) {
    const cards = this.snapshot.boards[space]
    this.publish({
      ...this.snapshot,
      boards: { ...this.snapshot.boards, [space]: cards.map(card => card.key === key ? { ...card, stage: 'In progress' } : card) },
    })
    if (this.snapshot.status === 'Connected') this.write('\r\nStarted ' + key + '\r\n$ ')
  }

  recover() {
    this.publish({ ...this.snapshot, status: 'Connected' })
    this.write('\r\nSession recovered.\r\n$ ')
  }

  send(data: string) {
    if (this.snapshot.status !== 'Connected') return
    for (const character of data) {
      if (character === '\r') {
        const command = this.input.trim()
        this.input = ''
        switch (command) {
          case 'status': this.write('\r\nConnected · orchestrator running\r\n$ '); break
          case 'clear': this.scrollback = ''; this.write('\x1b[2J\x1b[H$ '); break
          case 'exit': this.write('\r\nSession lost.\r\n'); this.publish({ ...this.snapshot, status: 'Session lost' }); break
          case '': this.write('\r\n$ '); break
          default: this.write('\r\nFixture commands: status, clear, exit\r\n$ ')
        }
      } else if (character === '\x7f' && this.input) {
        this.input = this.input.slice(0, -1)
        this.write('\b \b')
      } else if (/^[\x20-\x7e]$/.test(character) && this.input.length < 120) {
        this.input += character
        this.write(character)
      }
    }
  }
}
