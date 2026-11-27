// The client's stylesheets, in cascade order: xterm's own first, so the
// page's rules come after it; the tokens (ov-1510) before every rule that
// reads them; the shell's last. build.mjs concatenates them into app.css, and
// the browser checks serve the same concatenation.

import { readFileSync } from 'node:fs'

export const STYLESHEETS = [
  'node_modules/@xterm/xterm/css/xterm.css',
  'src/client/tokens.css',
  'src/client/app.css',
  'src/client/chat.css',
  'src/client/shell.css',
]

export const styles = () => Buffer.concat(STYLESHEETS.map(file => readFileSync(file)))

/// index.html for a browser check's harness page: the shell's markup without
/// the icon and manifest links, which only a build can name.
export const harnessHtml = () => readFileSync('src/client/index.html', 'utf8')
  .replace(/^<link rel="(icon|apple-touch-icon|manifest)"[^\n]*\n/gm, '')
