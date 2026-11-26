import type { WebRunner } from 'fc-web'

type Review = {
  title: string
  pick: string
  empty: string
  files: { path: string }[]
}

export async function openChanges(
  runner: WebRunner,
  worktree: string,
  dialog: HTMLDialogElement,
  failure: (error: Error) => string,
): Promise<void> {
  const title = dialog.querySelector('h2')!
  const label = dialog.querySelector('label')!
  const name = label.querySelector('span')!
  const files = label.querySelector('select')!
  const output = dialog.querySelector('pre')!
  title.textContent = ''
  label.hidden = true
  output.textContent = ''
  dialog.showModal()
  const visit = new AbortController()
  dialog.addEventListener('close', () => visit.abort(), { once: true })
  const current = () => dialog.open && !visit.signal.aborted
  try {
    const review: Review = JSON.parse(await runner.review_files(worktree))
    if (!current()) return
    title.textContent = review.title
    name.textContent = review.pick
    files.replaceChildren(...review.files.map(({ path }) => new Option(path, path)))
    label.hidden = !review.files.length
    if (!review.files.length) {
      output.textContent = review.empty
      return
    }
    let request = 0
    const read = async () => {
      const mine = ++request
      output.textContent = ''
      try {
        const patch = await runner.review_patch(worktree, files.value)
        if (current() && mine === request) output.textContent = patch
      } catch (error) {
        if (current() && mine === request) output.textContent = failure(error as Error)
      }
    }
    files.addEventListener('change', read, { signal: visit.signal })
    await read()
  } catch (error) {
    if (current()) output.textContent = failure(error as Error)
  }
}
