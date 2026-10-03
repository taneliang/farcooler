package com.farcooler.net

/**
 * Who gets a swipe on a terminal pane: the program, or this device's own view.
 *
 * The old rule was "whoever will take it": if the core could encode a wheel
 * event, the program had mouse reporting on and the bytes went out. That is
 * wrong on the primary screen. A program can turn mouse reporting on and then
 * do nothing visible with a wheel, and with no other scroll affordance on a
 * phone that pane could not be scrolled at all. The iPhone reported it from a
 * real device and fixed it in `TerminalSession.scroll` (03636b8a).
 *
 * The rule is about what is behind the screen. On the alternate screen there
 * is no scrollback to reach, so the wheel can only mean something to the
 * program, which still falls back to the local view if it declines the event.
 * On the primary screen there is history, reaching it is what a swipe means on
 * a phone, and it wins, even over a program that asked for the mouse.
 */
object WheelRoute {
    /** Whether the wheel goes to the program, given whether it would take one at all. */
    fun toProgram(alternateScreen: Boolean, programTakesWheel: Boolean): Boolean =
        alternateScreen && programTakesWheel
}
