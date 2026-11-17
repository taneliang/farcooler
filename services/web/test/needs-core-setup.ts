import { execFileSync } from 'node:child_process'
import { resolve } from 'node:path'
import { useNeedsYouPresenter } from '../src/client/needs-present'

const binary = resolve(process.env.CARGO_TARGET_DIR ?? '../../target', 'debug/examples/plan_view_bin')
useNeedsYouPresenter(request => execFileSync(binary, { input: request, encoding: 'utf8' }))
