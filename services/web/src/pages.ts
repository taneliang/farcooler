/// The Worker's own pages: complete HTML with no script, served under
/// `PAGE_CSP` (headers.ts). None of them carries a value from a request or the
/// relay, so none needs escaping.

const STYLE = `
:root { color-scheme: dark; --bg: #000; --text: #f5f5f7; --text-2: #a1a1a6; --button-bg: #0071e3; --button-text: #fff; --rule: #2c2c2e; }
@media (prefers-color-scheme: light) { :root { color-scheme: light; --bg: #fff; --text: #1d1d1f; --text-2: #6e6e73; --rule: #d2d2d7; } }
* { box-sizing: border-box; }
body { margin: 0; min-height: 100vh; display: grid; place-items: center; padding: 24px 16px; background: var(--bg); color: var(--text);
  font: 17px/1.47 -apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", Helvetica, Arial, sans-serif; }
main { max-width: 440px; text-align: center; }
h1 { font-size: 28px; line-height: 1.14; letter-spacing: 0.007em; margin: 0 0 12px; font-weight: 600; }
p { margin: 0 0 24px; color: var(--text-2); }
.button { display: inline-block; border: 0; border-radius: 980px; padding: 11px 22px; background: var(--button-bg); color: var(--button-text);
  font: inherit; text-decoration: none; cursor: pointer; }
.quiet { background: none; color: var(--text-2); border: 1px solid var(--rule); }
form { margin: 0; }
`

function shell(title: string, body: string): string {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex">
<title>${title} · Far Cooler</title>
<style>${STYLE}</style>
</head>
<body>
<main>
${body}
</main>
</body>
</html>
`
}

const SIGN_OUT = '<form method="post" action="/sign-out"><button class="button quiet" type="submit">Sign Out</button></form>'

/// The flag is off for this visitor. Signed in, they can sign out; signed out
/// (or with the client off for everyone) there's nothing to do here.
export function notAvailablePage(signedIn: boolean): string {
  return shell(
    'Not available yet',
    `<h1>Not available yet</h1>
<p>Far Cooler in a browser isn't open to this account yet. Your runners are a tap away in the Mac, iPhone and Android apps.</p>
${signedIn ? SIGN_OUT : ''}`,
  )
}

export function signInPage(): string {
  return shell(
    'Sign in',
    `<h1>Far Cooler</h1>
<p>Sign in to reach your runners' terminals from this browser.</p>
<a class="button" href="/sign-in">Sign In</a>`,
  )
}

export function signInFailedPage(): string {
  return shell(
    "Sign-in didn't finish",
    `<h1>Sign-in didn't finish</h1>
<p>It may have taken longer than ten minutes, or started in another tab.</p>
<a class="button" href="/sign-in">Try Again</a>`,
  )
}

export function unreachablePage(): string {
  return shell(
    "Can't reach Far Cooler",
    `<h1>Can't reach Far Cooler</h1>
<p>The server didn't answer. Check your connection, then try again.</p>
<a class="button" href="/">Try Again</a>`,
  )
}

export function limitedPage(): string {
  return shell(
    'Too many attempts',
    `<h1>Too many attempts</h1>
<p>Wait a minute, then try again.</p>
<a class="button" href="/">Try Again</a>`,
  )
}

export function notFoundPage(): string {
  return shell(
    'Not found',
    `<h1>Not found</h1>
<p>There's nothing at this address.</p>
<a class="button" href="/">Go to Far Cooler</a>`,
  )
}
