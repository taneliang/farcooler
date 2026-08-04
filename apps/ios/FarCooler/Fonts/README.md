# Bundled fonts

**Iosevka Nerd Font Mono**, Regular and Bold, under the SIL Open Font License
1.1 — see `IOSEVKA-LICENSE.md`, which the license requires be distributed with
the font.

The Nerd Font build rather than plain Iosevka, and the extra 12 MB is the reason
to choose it: coding agents print box-drawing, powerline separators and status
glyphs constantly, and a font without them renders those as tofu. A terminal that
cannot draw what the program sent is not showing you the program.

Committed rather than downloaded on demand. A font the app cannot draw with is
not an optional resource — the first launch, offline, on a plane, is exactly when
someone opens this — and a download that can fail is a terminal that can fail to
have a typeface.
