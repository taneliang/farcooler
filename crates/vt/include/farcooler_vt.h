/*
 * Far Cooler terminal core — the C ABI.
 *
 * This header is the contract between the Rust terminal core and every
 * platform renderer: Swift on macOS and iOS, Kotlin over JNI on Android. It is
 * Far Cooler's own interface, not a passthrough of whatever emulator sits
 * behind it, so the emulator can be replaced without touching a renderer.
 *
 * A renderer's whole job is: feed bytes in, ask for a snapshot, draw it, and
 * send encoded input back. It never parses an escape sequence and never
 * decides what an arrow key means.
 *
 * Threading: a handle is NOT thread-safe. Confine each handle to one thread —
 * on Apple platforms, the main actor, which is where drawing happens anyway.
 *
 * Lifetime: pointers returned by farcooler_vt_snapshot and farcooler_vt_title
 * borrow buffers owned by the handle. They stay valid until the next call on
 * that handle. Copy anything you intend to keep.
 *
 * Null safety: every function tolerates a NULL handle, returning zero, false
 * or NULL. A renderer bug must not take down the app.
 */

#ifndef FARCOOLER_VT_H
#define FARCOOLER_VT_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* MARK: - Cells */

#define FARCOOLER_VT_FLAG_BOLD      (1u << 0)
#define FARCOOLER_VT_FLAG_ITALIC    (1u << 1)
#define FARCOOLER_VT_FLAG_UNDERLINE (1u << 2)
#define FARCOOLER_VT_FLAG_INVERSE   (1u << 3)
/* A double-width character. The next column is its spacer; skip it. */
#define FARCOOLER_VT_FLAG_WIDE      (1u << 4)

/*
 * One character cell. Colours arrive already resolved to packed 0xRRGGBB, so
 * the named and 256-colour palettes are decided once in the core rather than
 * three times in three renderers.
 */
typedef struct {
    uint32_t ch;   /* Unicode scalar value. */
    uint32_t fg;   /* 0xRRGGBB */
    uint32_t bg;   /* 0xRRGGBB */
    uint16_t flags;
    uint16_t _pad;
} FarCoolerVtCell;

/*
 * The screen. `cells` is row-major, `rows * columns` long, and borrowed from
 * the handle.
 */
typedef struct {
    const FarCoolerVtCell *cells;
    uint16_t columns;
    uint16_t rows;
    uint16_t cursor_row;
    uint16_t cursor_column;
    bool cursor_visible;
    /* How far the view is scrolled back, in lines. Zero means live. */
    uint32_t display_offset;
    /* Lines available above the screen, for drawing a scrollbar. */
    uint32_t history_size;
} FarCoolerVtSnapshot;

/*
 * Where a URL sits on screen, in display coordinates — the same space
 * FarCoolerVtSnapshot reports in, so a renderer can underline the span without
 * converting anything. A span can cover two rows: long URLs wrap.
 */
typedef struct {
    uint16_t start_row;
    uint16_t start_column;
    uint16_t end_row;
    uint16_t end_column;
} FarCoolerVtUrlSpan;

/* MARK: - Lifecycle */

/* Create a terminal. Dimensions are clamped to the protocol's range. */
void *farcooler_vt_new(uint16_t columns, uint16_t rows);

/* Destroy a terminal. Safe with NULL; never call twice on the same handle. */
void farcooler_vt_free(void *handle);

/* MARK: - Output */

/*
 * Feed program output.
 *
 * Chunk boundaries do not matter: a sequence split across calls parses exactly
 * as one call would, which is what makes this safe to drive straight from a
 * socket read.
 */
void farcooler_vt_feed(void *handle, const uint8_t *bytes, size_t len);

/* Resize the grid. The core reflows; the renderer does not. */
void farcooler_vt_resize(void *handle, uint16_t columns, uint16_t rows);

/*
 * A counter that changes whenever the screen may have changed. Cache it; if it
 * has not moved, skip the frame. This is what keeps an idle terminal from
 * burning a redraw on every tick.
 */
uint64_t farcooler_vt_revision(void *handle);

/*
 * Read the screen into `out`. Returns false only on a NULL argument.
 *
 * The cell buffer is reused between calls, so a steady redraw allocates
 * nothing.
 */
bool farcooler_vt_snapshot(void *handle, FarCoolerVtSnapshot *out);

/*
 * Scroll the view. Positive goes back into history, negative returns toward the
 * live screen; both ends are clamped.
 *
 * Scrollback is the client's own view — the program is never told about it, so
 * this produces no bytes and no reflow.
 */
void farcooler_vt_scroll(void *handle, int32_t lines);

/*
 * Jump back to the live screen. Call this on input: typing into a scrolled-back
 * view would show the user nothing of what they typed.
 */
void farcooler_vt_scroll_to_bottom(void *handle);

/// Recolour the terminal. `colors` is 19 packed 0x00RRGGBB values: 16 ANSI,
/// then foreground, background, cursor. False if the length is not 19.
bool farcooler_vt_set_palette(void *handle, const uint32_t *colors, size_t len);

/*
 * Take bytes the program wants written back to the pty and send them on.
 *
 * Cursor-position reports and mouse replies must reach the program or a
 * full-screen agent will sit waiting for an answer that never comes. Returns
 * the number copied; call again while the result equals `capacity`.
 */
size_t farcooler_vt_take_writes(void *handle, uint8_t *out, size_t capacity);

/* Take the bell flag, clearing it. */
bool farcooler_vt_take_bell(void *handle);

/*
 * Take text the program asked to put on the clipboard (OSC 52).
 *
 * Returns the byte length needed and drains only when it fits, so a short
 * buffer cannot truncate a copy. Put the result on the platform clipboard.
 *
 * There is deliberately no read counterpart: a program asking for the
 * clipboard's CONTENTS is refused by the parser and never reaches the handle.
 * Copy is a program handing you something; paste is a program taking
 * something, and Far Cooler runs agents on machines nobody is watching.
 */
size_t farcooler_vt_take_clipboard(void *handle, uint8_t *out, size_t capacity);

/*
 * The title the program last set, or NULL if it never set one. Borrowed;
 * valid until the next feed.
 */
const char *farcooler_vt_title(void *handle);

/*
 * True when the program has taken over the whole screen. Use it to decide
 * whether the wheel scrolls your own scrollback or belongs to the program.
 */
bool farcooler_vt_alt_screen(void *handle);

/*
 * The URL under a cell, or 0 if there is none.
 *
 * Returns the byte length the URL needs and writes NOTHING when that exceeds
 * `capacity` — call again with a buffer at least that large. Same contract as
 * farcooler_vt_encode_paste, for a sharper reason: a truncated URL is a
 * different URL, and opening one is worse than opening none.
 *
 * `span` is filled whenever a URL is found, sizing calls included, so a
 * renderer can underline the match before it has anywhere to put the text.
 *
 * An OSC 8 hyperlink wins over the text under it. Only a fixed allowlist of
 * schemes is ever returned — terminal output is not trusted input, and the list
 * lives in the core so three renderers cannot widen it independently.
 */
size_t farcooler_vt_url_at(void *handle, uint16_t row, uint16_t column,
                           FarCoolerVtUrlSpan *span, uint8_t *out,
                           size_t capacity);

/* MARK: - Input */

/*
 * Key codes.
 *
 * A printable key is its own Unicode scalar value: pass through whatever the
 * platform's keyboard layout produced. The core never maps scancodes, because
 * only the OS knows the user's layout.
 *
 * Special keys live in the Unicode private use area, which no keyboard layout
 * can produce, so the two spaces cannot collide.
 */
#define FARCOOLER_VT_KEY_ENTER     0xE000u
#define FARCOOLER_VT_KEY_TAB       0xE001u
#define FARCOOLER_VT_KEY_BACKSPACE 0xE002u
#define FARCOOLER_VT_KEY_ESCAPE    0xE003u
#define FARCOOLER_VT_KEY_UP        0xE004u
#define FARCOOLER_VT_KEY_DOWN      0xE005u
#define FARCOOLER_VT_KEY_RIGHT     0xE006u
#define FARCOOLER_VT_KEY_LEFT      0xE007u
#define FARCOOLER_VT_KEY_HOME      0xE008u
#define FARCOOLER_VT_KEY_END       0xE009u
#define FARCOOLER_VT_KEY_PAGE_UP   0xE00Au
#define FARCOOLER_VT_KEY_PAGE_DOWN 0xE00Bu
#define FARCOOLER_VT_KEY_INSERT    0xE00Cu
#define FARCOOLER_VT_KEY_DELETE    0xE00Du
/* F1..F12 are FARCOOLER_VT_KEY_F1 + n - 1. */
#define FARCOOLER_VT_KEY_F1        0xE010u

#define FARCOOLER_VT_MOD_SHIFT (1u << 0)
#define FARCOOLER_VT_MOD_ALT   (1u << 1)
#define FARCOOLER_VT_MOD_CTRL  (1u << 2)

#define FARCOOLER_VT_MOUSE_LEFT       0u
#define FARCOOLER_VT_MOUSE_MIDDLE     1u
#define FARCOOLER_VT_MOUSE_RIGHT      2u
#define FARCOOLER_VT_MOUSE_WHEEL_UP   3u
#define FARCOOLER_VT_MOUSE_WHEEL_DOWN 4u

#define FARCOOLER_VT_MOUSE_PRESS   0u
#define FARCOOLER_VT_MOUSE_RELEASE 1u
#define FARCOOLER_VT_MOUSE_MOVE    2u

/*
 * Encode a keystroke into `out`. Returns the number of bytes written, or 0 if
 * the buffer is too small — never a partial escape sequence, which the program
 * would read as garbage typing. 16 bytes is enough for every sequence.
 *
 * This takes the handle because the answer depends on modes the program set.
 */
size_t farcooler_vt_encode_key(void *handle, uint32_t key, uint32_t modifiers,
                               uint8_t *out, size_t capacity);

/*
 * Encode a mouse event. Coordinates are zero-based cells.
 *
 * Returns 0 when the program does not want the event. That is not an error: it
 * means handle it locally instead (select text, scroll your own scrollback).
 */
size_t farcooler_vt_encode_mouse(void *handle, uint32_t button, uint32_t action,
                                 uint16_t column, uint16_t row, uint32_t modifiers,
                                 uint8_t *out, size_t capacity);

/*
 * Encode pasted text, bracketing it if the program asked for that — without
 * which an editor auto-indents every pasted line and a shell runs each newline
 * as a command.
 *
 * Returns the number of bytes the encoding NEEDS. If that exceeds `capacity`,
 * nothing is written: call again with a buffer at least that large. A paste is
 * arbitrarily long, and silently truncating one would corrupt it.
 */
size_t farcooler_vt_encode_paste(void *handle, const uint8_t *text, size_t len,
                                 uint8_t *out, size_t capacity);

#ifdef __cplusplus
}
#endif

#endif /* FARCOOLER_VT_H */
