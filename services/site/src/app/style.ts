/// The web app's stylesheet, inline in every page so /app needs no asset.
///
/// The marketing page's tokens (public/index.html), unchanged: dark by
/// default, light when the system asks for it, the same type and the same blue.
/// Lists are the Mac app's grouped rows: one rounded card, hairline
/// separators inset past the leading edge, secondary text in the second gray.

export const STYLE = `
:root {
  color-scheme: dark;
  --bg: #000000;
  --bg-alt: #111113;
  --text: #f5f5f7;
  --text-2: #a1a1a6;
  --text-3: #86868b;
  --rule: #2c2c2e;
  --hover: rgba(255, 255, 255, 0.05);
  --avatar: #3a3a3c;
  --link: #2997ff;
  --button-bg: #0071e3;
  --button-text: #ffffff;
  --danger: #ff453a;
  --danger-bg: #d70015;
  --field: #1c1c1e;
}
@media (prefers-color-scheme: light) {
  :root {
    color-scheme: light;
    --bg: #ffffff;
    --bg-alt: #f5f5f7;
    --text: #1d1d1f;
    --text-2: #6e6e73;
    --text-3: #86868b;
    --rule: #d2d2d7;
    --hover: rgba(0, 0, 0, 0.035);
    --avatar: #e5e5ea;
    --link: #0066cc;
    --danger: #d70015;
    --field: #ffffff;
  }
}
*, *::before, *::after { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; }
body {
  margin: 0;
  background: var(--bg);
  color: var(--text);
  font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", Helvetica, Arial, sans-serif;
  font-size: 17px;
  line-height: 1.47;
  letter-spacing: -0.01em;
  -webkit-font-smoothing: antialiased;
}
a { color: var(--link); text-decoration: none; }
a:hover { text-decoration: underline; }
.wrap { width: 100%; max-width: 800px; margin: 0 auto; padding: 0 20px; }

.bar { border-bottom: 1px solid var(--rule); }
.bar .wrap { display: flex; align-items: center; justify-content: space-between; gap: 16px; height: 52px; }
.mark { font-weight: 600; font-size: 19px; letter-spacing: -0.02em; color: var(--text); white-space: nowrap; }
.mark:hover { text-decoration: none; }
.account { display: flex; align-items: center; gap: 14px; min-width: 0; font-size: 14px; }
.account .who { color: var(--text-2); min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.account form { margin: 0; flex: none; }
.plain {
  font: inherit; color: var(--link); background: none; border: 0; padding: 0; cursor: pointer; white-space: nowrap;
}
.plain:hover { text-decoration: underline; }

h1 {
  font-family: -apple-system, BlinkMacSystemFont, "SF Pro Display", "Helvetica Neue", Helvetica, Arial, sans-serif;
  font-weight: 700;
  margin: 0;
  text-wrap: balance;
  overflow-wrap: anywhere;
}
.page { padding-top: 48px; padding-bottom: 96px; }
.page h1 { font-size: 34px; line-height: 1.12; letter-spacing: -0.022em; }
.lede { color: var(--text-2); margin: 6px 0 0; text-wrap: pretty; }
.back { display: inline-block; font-size: 15px; margin: 0 0 14px; }

.list { list-style: none; margin: 28px 0 0; padding: 0; background: var(--bg-alt); border-radius: 12px; overflow: hidden; }
.list li + li .row::before {
  content: ""; position: absolute; top: 0; left: 18px; right: 0; border-top: 1px solid var(--rule);
}
.row { position: relative; display: flex; align-items: center; gap: 14px; padding: 13px 18px; min-height: 64px; color: var(--text); }
a.row:hover { text-decoration: none; background: var(--hover); }
a.row:focus-visible { outline: 2px solid var(--link); outline-offset: -2px; border-radius: 12px; }
.row .main { flex: 1; min-width: 0; display: flex; flex-direction: column; }
.row .name { font-weight: 500; overflow-wrap: anywhere; }
.row .meta { font-size: 14px; color: var(--text-2); margin-top: 1px; overflow-wrap: anywhere; }
.row .end { flex: none; font-size: 15px; color: var(--text-2); }
.row .chevron { flex: none; width: 8px; height: 14px; color: var(--text-3); }
.members li + li .row::before { left: 68px; }
.avatar {
  flex: none; width: 36px; height: 36px; border-radius: 50%; background: var(--avatar); color: var(--text-2);
  display: grid; place-items: center; font-size: 15px; font-weight: 600; text-transform: uppercase;
}
.note { font-size: 14px; color: var(--text-3); margin: 14px 18px 0; text-wrap: pretty; }


h2 { font-size: 22px; line-height: 1.2; font-weight: 700; letter-spacing: -0.02em; margin: 40px 0 0; }
.page h2 + .list, .page h2 + .card, .page h2 + details { margin-top: 12px; }
.danger { color: var(--danger); }
.button.danger { background: var(--danger-bg); color: #fff; }
.row form.end { margin: 0; flex: none; }
.row .plain { font-size: 15px; }
.row a.end { color: var(--link); font-size: 15px; }
.row .plain.danger { color: var(--danger); }
.cancel { display: inline-block; padding: 12px 8px; }

.banner { margin: 22px 0 0; padding: 12px 18px; border-radius: 12px; background: var(--bg-alt); font-size: 15px; text-wrap: pretty; }
.banner.bad { color: var(--danger); }

.actions { display: flex; flex-wrap: wrap; align-items: center; justify-content: center; gap: 8px 18px; }
.actions form { margin: 0; }
.actions.left { justify-content: flex-start; margin-top: 24px; }
.card { margin: 12px 0 0; padding: 18px; background: var(--bg-alt); border-radius: 12px; }
.card .actions { justify-content: flex-start; margin-top: 18px; }
.fields { display: grid; grid-template-columns: 1fr 1fr; gap: 16px 18px; }
.field { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
.field span { font-size: 14px; color: var(--text-2); }
.field input, .field select {
  font: inherit; font-size: 17px; color: var(--text); background: var(--field); border: 1px solid var(--rule);
  border-radius: 10px; padding: 10px 12px; width: 100%; min-width: 0; appearance: none; -webkit-appearance: none;
}
.field select {
  padding-right: 34px;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='6' viewBox='0 0 10 6'%3E%3Cpath d='M1 1l4 4 4-4' fill='none' stroke='%2386868b' stroke-width='1.6' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E");
  background-repeat: no-repeat; background-position: right 12px center;
}
.field input::placeholder { color: var(--text-3); }
.field input:focus-visible, .field select:focus-visible { outline: 2px solid var(--link); outline-offset: 1px; }
.make { margin-top: 12px; }
.make summary { list-style: none; cursor: pointer; }
.make summary::-webkit-details-marker { display: none; }
.make[open] summary { display: none; }
.make[open] > .card { margin-top: 0; }
.link {
  margin: 22px 0 0; padding: 16px 18px; background: var(--bg-alt); border-radius: 12px; user-select: all; -webkit-user-select: all;
  font: 15px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace; overflow-wrap: anywhere; word-break: break-all;
}
.held { list-style: none; margin: 8px auto 0; padding: 0; max-width: 28em; text-align: center; }
.held li { padding: 2px 0; overflow-wrap: anywhere; }
.held .meta { display: block; color: var(--text-2); font-size: 14px; }
.held li + li { margin-top: 8px; }

.message { text-align: center; padding-top: 120px; padding-bottom: 120px; max-width: 560px; }
.message h1 { font-size: 40px; line-height: 1.1; letter-spacing: -0.025em; }
.message .lede { font-size: 19px; line-height: 1.45; margin-top: 12px; text-wrap: balance; }
.message .actions { margin-top: 28px; }
.message .fine { font-size: 14px; color: var(--text-3); margin: 18px auto 0; max-width: 28em; text-wrap: balance; }
.button {
  display: inline-block; background: var(--button-bg); color: var(--button-text); font: inherit; font-size: 17px; font-weight: 500;
  padding: 12px 26px; border: 0; border-radius: 980px; cursor: pointer;
}
.button:hover { text-decoration: none; filter: brightness(1.08); }
.button:focus-visible, .plain:focus-visible { outline: 2px solid var(--link); outline-offset: 3px; }

@media (max-width: 520px) {
  .page { padding-top: 28px; }
  .page h1 { font-size: 28px; }
  .message { padding-top: 72px; }
  .message h1 { font-size: 32px; }
  .message .lede { font-size: 17px; }
  .row { padding: 12px 16px; }
  .list li + li .row::before { left: 16px; }
  .members li + li .row::before { left: 66px; }
  .account .who { display: none; }
  .fields { grid-template-columns: 1fr; }
  h2 { margin-top: 32px; }
}
`
