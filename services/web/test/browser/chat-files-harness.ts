/// chat-files-check.mjs's page: the scripted conversation (chat-page.ts) on
/// index.html under the React shell (ov-1510), so the chat's column is as wide
/// as the signed-in app makes it at each width (ov-1520).

import './chat-page'
import { harnessShell, runnerNamed } from './harness-shell'

harnessShell({ page: 'terminals', runner: runnerNamed('studio') })
