package com.farcooler.ui

import android.content.Context
import android.text.InputType
import android.view.KeyEvent
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import com.farcooler.core.Vt

/**
 * The view the software keyboard is attached to.
 *
 * It carries no visible state of its own and draws nothing: every character it
 * reports goes straight to the host and comes back through the stream, which is
 * the only place the terminal ever draws what was typed. That is the same
 * contract the Apple renderers keep — this file never decides what a key means,
 * it only reports which one was pressed.
 *
 * ## Why not an EditText
 *
 * A text field holds text, and a terminal has none to hold: there is no cursor
 * to place, no selection to track, and no undo stack that would mean anything.
 * `View` plus [onCreateInputConnection] is the smaller surface that still gets
 * a keyboard, and it is what stops the IME from trying to reconcile a buffer
 * with a screen the host owns.
 *
 * ## Why the input type says "visible password"
 *
 * Every keyboard on Android treats that type as "do not help": no autocorrect
 * rewriting a flag, no smart quotes producing a character no shell recognises,
 * no autocapitalisation upper-casing a command nobody typed that way, and no
 * learning from what is typed here. The alternative — asking politely with
 * `TYPE_TEXT_FLAG_NO_SUGGESTIONS` — is honoured by some keyboards and ignored
 * by others, and a flag silently rewritten mid-command is a very bad way to
 * find that out. Gboard, Samsung's keyboard and every terminal emulator on the
 * platform have converged on this for the same reason.
 */
class TerminalInputView(context: Context) : View(context) {
    /** Printable text the keyboard committed. */
    var onText: ((String) -> Unit)? = null

    /** A special key, already translated to the core's own key space. */
    var onKey: ((Int, Int) -> Unit)? = null

    init {
        isFocusable = true
        isFocusableInTouchMode = true
    }

    override fun onCheckIsTextEditor(): Boolean = true

    fun showKeyboard() {
        requestFocus()
        val imm = context.getSystemService(InputMethodManager::class.java)
        imm?.showSoftInput(this, 0)
    }

    fun hideKeyboard() {
        val imm = context.getSystemService(InputMethodManager::class.java)
        imm?.hideSoftInputFromWindow(windowToken, 0)
        clearFocus()
    }

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection {
        outAttrs.inputType =
            InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
        outAttrs.imeOptions = EditorInfo.IME_ACTION_NONE or
            EditorInfo.IME_FLAG_NO_FULLSCREEN or
            EditorInfo.IME_FLAG_NO_EXTRACT_UI or
            EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING
        outAttrs.initialSelStart = 0
        outAttrs.initialSelEnd = 0

        // `fullEditor = false`: this view has no Editable behind it, so the IME
        // must not believe it can read one back.
        return object : BaseInputConnection(this, false) {
            override fun commitText(text: CharSequence?, newCursorPosition: Int): Boolean {
                text?.takeIf { it.isNotEmpty() }?.let { deliver(it.toString()) }
                return true
            }

            /**
             * Composition, delivered as it is typed.
             *
             * A terminal has nothing to compose into — there is no buffer to
             * revise — so a partially-composed word is treated as typing. This
             * is what makes a swipe-typing keyboard usable at all: it commits
             * only at the end of a word, and waiting for that would mean
             * nothing reached the shell until a space was typed.
             */
            override fun setComposingText(text: CharSequence?, newCursorPosition: Int): Boolean {
                return true
            }

            override fun finishComposingText(): Boolean {
                return true
            }

            override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
                repeat(beforeLength) { onKey?.invoke(Vt.KEY_BACKSPACE, 0) }
                return true
            }

            override fun sendKeyEvent(event: KeyEvent?): Boolean {
                if (event == null || event.action != KeyEvent.ACTION_DOWN) return true
                return handleKeyEvent(event)
            }

            override fun performEditorAction(actionCode: Int): Boolean {
                onKey?.invoke(Vt.KEY_ENTER, 0)
                return true
            }
        }
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        if (handleKeyEvent(event)) return true
        return super.onKeyDown(keyCode, event)
    }

    /**
     * Text goes as text; anything else is looked up.
     *
     * Split because they answer different questions. A printable key is its own
     * Unicode scalar and passes straight through — only the OS knows the user's
     * layout, so nothing here maps scancodes. A named key has no scalar, and
     * what it encodes to depends on modes the emulator holds, which is why it
     * crosses as a code rather than as bytes.
     */
    private fun handleKeyEvent(event: KeyEvent): Boolean {
        val modifiers = modifiersOf(event)
        val special = specialKey(event.keyCode)
        if (special != null) {
            onKey?.invoke(special, modifiers)
            return true
        }
        // `unicodeChar` with the meta state stripped of Ctrl: Ctrl-C has no
        // printable character, and asking for one returns 0. The core turns the
        // scalar plus the Ctrl modifier into the control code, because whether
        // it should is a question about the program's modes.
        val scalar = event.getUnicodeChar(event.metaState and KeyEvent.META_CTRL_MASK.inv())
        if (scalar != 0) {
            onKey?.invoke(scalar, modifiers)
            return true
        }
        return false
    }

    private fun deliver(text: String) {
        // The Enter the keyboard's own return key produces. Passing the scalar
        // straight through would send a bare 0x0A — a literal newline character
        // rather than the Enter keystroke a program expects, and the two are
        // not interchangeable under a raw pty.
        if (text == "\n") {
            onKey?.invoke(Vt.KEY_ENTER, 0)
            return
        }
        onText?.invoke(text)
    }

    private fun modifiersOf(event: KeyEvent): Int {
        var modifiers = 0
        if (event.isShiftPressed) modifiers = modifiers or Vt.MOD_SHIFT
        if (event.isAltPressed) modifiers = modifiers or Vt.MOD_ALT
        if (event.isCtrlPressed) modifiers = modifiers or Vt.MOD_CTRL
        return modifiers
    }

    private fun specialKey(keyCode: Int): Int? = when (keyCode) {
        KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> Vt.KEY_ENTER
        KeyEvent.KEYCODE_TAB -> Vt.KEY_TAB
        KeyEvent.KEYCODE_DEL -> Vt.KEY_BACKSPACE
        KeyEvent.KEYCODE_FORWARD_DEL -> Vt.KEY_DELETE
        KeyEvent.KEYCODE_ESCAPE -> Vt.KEY_ESCAPE
        KeyEvent.KEYCODE_DPAD_UP -> Vt.KEY_UP
        KeyEvent.KEYCODE_DPAD_DOWN -> Vt.KEY_DOWN
        KeyEvent.KEYCODE_DPAD_LEFT -> Vt.KEY_LEFT
        KeyEvent.KEYCODE_DPAD_RIGHT -> Vt.KEY_RIGHT
        KeyEvent.KEYCODE_MOVE_HOME -> Vt.KEY_HOME
        KeyEvent.KEYCODE_MOVE_END -> Vt.KEY_END
        KeyEvent.KEYCODE_PAGE_UP -> Vt.KEY_PAGE_UP
        KeyEvent.KEYCODE_PAGE_DOWN -> Vt.KEY_PAGE_DOWN
        KeyEvent.KEYCODE_INSERT -> Vt.KEY_INSERT
        in KeyEvent.KEYCODE_F1..KeyEvent.KEYCODE_F12 -> Vt.KEY_F1 + (keyCode - KeyEvent.KEYCODE_F1)
        else -> null
    }
}
