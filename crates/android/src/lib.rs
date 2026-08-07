//! The JNI boundary, for the Android app.
//!
//! Android is the one platform that cannot speak the C ABI directly. Swift
//! imports a C header and calls the function; Kotlin has no such thing, and the
//! ways around it — JNA's reflection at every call, or the Panama FFI, which is
//! not on Android at all — cost more than a shim does. So this crate is the
//! shim: a thin translation between `jni`'s types and the two C ABIs the other
//! platforms already use.
//!
//! **It adds no behaviour.** Every function here calls straight through to
//! `farcooler_client::ffi` or `farcooler_vt::ffi`, which is the whole point:
//! the SSH transport, the protocol, the emulator, the key encoder and the
//! colour resolution are the same code the Mac and the phone run, not a second
//! implementation that agrees with them today. The only judgement calls made in
//! this file are about how a value crosses into the JVM — a JSON string, a byte
//! array, one flat `int[]` for a screen — and those are stated where they are
//! made.
//!
//! ## Threading
//!
//! Both C ABIs document that a handle is not thread-safe and must be confined
//! to one thread. That contract is not weakened here and cannot be enforced
//! here: it is the Kotlin side's job, and `ClientCore`/`VtCore` each confine
//! their handle to a single dispatcher for exactly this reason.
//!
//! ## Null and zero
//!
//! A zero handle is what Kotlin holds after `free`, and every function tolerates
//! it — the underlying C functions all null-check — so a lifecycle bug in the UI
//! is a no-op rather than a process death. An Android app that crashes in native
//! code leaves no Kotlin stack trace worth reading, which makes the cheap
//! defence worth far more here than the equivalent is on a Mac.

use std::ffi::{CStr, CString, c_void};

use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JIntArray, JString};
use jni::sys::{jboolean, jbyteArray, jint, jintArray, jlong, jstring};

use farcooler_client::ffi as client;
use farcooler_vt::ffi as vt;

// MARK: - Helpers

/// A Java string as an owned Rust `CString`, or `None` if it was null or not
/// convertible.
///
/// `CString` rather than `String` because everything downstream is a C ABI
/// expecting NUL termination, and building that once per call here is cheaper
/// than every call site remembering to.
fn c_string(env: &mut JNIEnv, value: &JString) -> Option<CString> {
    if value.is_null() {
        return None;
    }
    let text: String = env.get_string(value).ok()?.into();
    CString::new(text).ok()
}

/// Hand a Rust string to the JVM, or null on failure.
///
/// Null rather than an exception: every caller of this treats null as "nothing
/// to report", which is a shape Kotlin already has to handle, and throwing
/// would turn a missing title into a crash.
fn jstring_of(env: &mut JNIEnv, text: &str) -> jstring {
    env.new_string(text).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

fn byte_array_of(env: &mut JNIEnv, bytes: &[u8]) -> jbyteArray {
    match env.byte_array_from_slice(bytes) {
        Ok(array) => array.into_raw(),
        // Null on an allocation failure the JVM already threw for. Returning a
        // dangling or empty array instead would hand Kotlin bytes that were
        // never encoded, which for a keystroke means silently typing nothing.
        Err(_) => std::ptr::null_mut(),
    }
}

fn handle_of(value: jlong) -> *mut c_void {
    value as usize as *mut c_void
}

// MARK: - Client

/// # Safety
/// Called only by the JVM, which supplies valid `env` and `class` references.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeClient_nativeNew(
    _env: JNIEnv,
    _class: JClass,
) -> jlong {
    client::farcooler_client_new() as usize as jlong
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeClient_nativeFree(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    unsafe { client::farcooler_client_free(handle_of(handle)) }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeClient_nativeConnect(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    config: JString,
) -> jlong {
    let Some(config) = c_string(&mut env, &config) else { return 0 };
    unsafe { client::farcooler_client_connect(handle_of(handle), config.as_ptr()) as jlong }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeClient_nativeCall(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    method: JString,
    args: JString,
) -> jlong {
    let Some(method) = c_string(&mut env, &method) else { return 0 };
    let Some(args) = c_string(&mut env, &args) else { return 0 };
    unsafe {
        client::farcooler_client_call(handle_of(handle), method.as_ptr(), args.as_ptr()) as jlong
    }
}

/// Paste an image into a terminal.
///
/// Takes the bytes as a `byte[]` rather than through `nativeCall`'s JSON,
/// because the payload is megabytes of binary and base64 in both directions
/// would be a third more bytes and two more copies to describe something no
/// Kotlin here ever looks at.
///
/// The core copies the bytes before returning, so the JNI borrow ends with this
/// call and the array is the JVM's again immediately.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeClient_nativePasteImage(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    terminal: JString,
    mime: JString,
    data: JByteArray,
) -> jlong {
    let Some(terminal) = c_string(&mut env, &terminal) else { return 0 };
    let Some(mime) = c_string(&mut env, &mime) else { return 0 };
    let Ok(bytes) = env.convert_byte_array(&data) else { return 0 };
    if bytes.is_empty() {
        return 0;
    }
    unsafe {
        client::farcooler_client_paste_image(
            handle_of(handle),
            terminal.as_ptr(),
            mime.as_ptr(),
            bytes.as_ptr(),
            bytes.len(),
        ) as jlong
    }
}

/// The oldest finished result, or null when nothing is ready.
///
/// Copied into a Java string immediately, before anything else can touch the
/// handle. The C ABI states the pointer is only valid until the next call on
/// that handle, and "the next call" on Android includes whatever the pump does
/// on its very next loop iteration — so borrowing it any longer would be a
/// use-after-free that only shows up under load.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeClient_nativePoll(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    let raw = unsafe { client::farcooler_client_poll(handle_of(handle)) };
    if raw.is_null() {
        return std::ptr::null_mut();
    }
    let Ok(text) = (unsafe { CStr::from_ptr(raw) }).to_str() else {
        return std::ptr::null_mut();
    };
    jstring_of(&mut env, text)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeClient_nativeConnected(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jboolean {
    u8::from(unsafe { client::farcooler_client_connected(handle_of(handle)) })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeClient_nativeStreamStart(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    terminal: JString,
) -> jboolean {
    let Some(terminal) = c_string(&mut env, &terminal) else { return 0 };
    u8::from(unsafe { client::farcooler_client_stream_start(handle_of(handle), terminal.as_ptr()) })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeClient_nativeStreamStop(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    terminal: JString,
) {
    let Some(terminal) = c_string(&mut env, &terminal) else { return };
    unsafe { client::farcooler_client_stream_stop(handle_of(handle), terminal.as_ptr()) }
}

/// Generate this device's SSH identity, as `{"private_key":…,"public_key":…}`.
///
/// The two-call "ask for the size, then ask for the bytes" dance the C ABI uses
/// does not cross into Kotlin: a Java string carries its own length, so the
/// reason for the dance — a caller with a fixed buffer — does not exist on this
/// side. 8 KiB is several times what an ed25519 pair encodes to; the length is
/// still checked rather than assumed, because a silently truncated private key
/// would be a device that authenticates with a key nobody can revoke.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeClient_nativeGenerateKey(
    mut env: JNIEnv,
    _class: JClass,
    comment: JString,
) -> jstring {
    let comment = c_string(&mut env, &comment);
    let pointer = comment.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());

    let mut buffer = vec![0u8; 8192];
    let written = unsafe {
        client::farcooler_client_generate_key(pointer, buffer.as_mut_ptr(), buffer.len())
    };
    if written == 0 || written > buffer.len() {
        return std::ptr::null_mut();
    }
    match std::str::from_utf8(&buffer[..written]) {
        Ok(text) => jstring_of(&mut env, text),
        Err(_) => std::ptr::null_mut(),
    }
}

/// The public key belonging to a private key, as one OpenSSH line.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeClient_nativePublicKey(
    mut env: JNIEnv,
    _class: JClass,
    private_key: JString,
) -> jstring {
    let Some(private_key) = c_string(&mut env, &private_key) else {
        return std::ptr::null_mut();
    };
    let mut buffer = vec![0u8; 4096];
    let written = unsafe {
        client::farcooler_client_public_key(
            private_key.as_ptr(),
            buffer.as_mut_ptr(),
            buffer.len(),
        )
    };
    if written == 0 || written > buffer.len() {
        return std::ptr::null_mut();
    }
    match std::str::from_utf8(&buffer[..written]) {
        Ok(text) => jstring_of(&mut env, text),
        Err(_) => std::ptr::null_mut(),
    }
}

/// The themes compiled into this build, as JSON.
///
/// Session-free, like the key helpers above: a phone that has never reached a
/// host still needs a theme to render with, and every other client call needs
/// a live ssh connection.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeClient_nativeBuiltinThemes(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    // Asked for the size first rather than guessing a buffer: eleven themes of
    // nineteen colours is a few kilobytes today and is exactly the sort of
    // number that grows without anyone revisiting a constant.
    let needed = unsafe { client::farcooler_client_builtin_themes(std::ptr::null_mut(), 0) };
    if needed == 0 {
        return std::ptr::null_mut();
    }
    let mut buffer = vec![0u8; needed];
    let written =
        unsafe { client::farcooler_client_builtin_themes(buffer.as_mut_ptr(), buffer.len()) };
    if written == 0 || written > buffer.len() {
        return std::ptr::null_mut();
    }
    match std::str::from_utf8(&buffer[..written]) {
        Ok(text) => jstring_of(&mut env, text),
        Err(_) => std::ptr::null_mut(),
    }
}

// MARK: - Terminal core

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeVt_nativeNew(
    _env: JNIEnv,
    _class: JClass,
    columns: jint,
    rows: jint,
) -> jlong {
    vt::farcooler_vt_new(clamp_u16(columns), clamp_u16(rows)) as usize as jlong
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeVt_nativeFree(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    unsafe { vt::farcooler_vt_free(handle_of(handle)) }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeVt_nativeFeed(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    bytes: JByteArray,
) {
    let Ok(bytes) = env.convert_byte_array(&bytes) else { return };
    if bytes.is_empty() {
        return;
    }
    unsafe { vt::farcooler_vt_feed(handle_of(handle), bytes.as_ptr(), bytes.len()) }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeVt_nativeResize(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    columns: jint,
    rows: jint,
) {
    unsafe { vt::farcooler_vt_resize(handle_of(handle), clamp_u16(columns), clamp_u16(rows)) }
}

/// Recolour the terminal. Nineteen packed values: sixteen ANSI, then
/// foreground, background, cursor. False if the array is any other length.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeVt_nativeSetPalette(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    colors: JIntArray,
) -> jboolean {
    let Ok(len) = env.get_array_length(&colors) else {
        return 0;
    };
    let mut values = vec![0i32; len as usize];
    if env.get_int_array_region(&colors, 0, &mut values).is_err() {
        return 0;
    }
    // Kotlin has no unsigned int, so the colours arrive as a signed bit
    // pattern. The bits are the same; only the interpretation differs.
    let packed: Vec<u32> = values.into_iter().map(|v| v as u32).collect();
    let ok = unsafe {
        vt::farcooler_vt_set_palette(handle_of(handle), packed.as_ptr(), packed.len())
    };
    u8::from(ok)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeVt_nativeRevision(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jlong {
    unsafe { vt::farcooler_vt_revision(handle_of(handle)) as jlong }
}

/// The whole screen, as one flat `int[]`.
///
/// Seven header values, then four per cell — character, foreground, background,
/// flags — laid out row-major:
///
/// ```text
/// [columns, rows, cursorRow, cursorColumn, cursorVisible, displayOffset, historySize,
///  ch, fg, bg, flags,  ch, fg, bg, flags,  …]
/// ```
///
/// One array rather than an array of objects, for the reason the C ABI packs a
/// flat cell buffer: an 80×24 screen is 1,920 cells, and allocating that many
/// short-lived JVM objects several times a second is exactly the shape of
/// garbage that makes a scrolling terminal stutter. One `int[]` is one
/// allocation and one bulk copy.
///
/// Colours arrive already resolved to 0xRRGGBB by the core, so the palette is
/// decided once in Rust rather than a third time in Kotlin.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeVt_nativeSnapshot(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jintArray {
    let mut snapshot = vt::VtSnapshot {
        cells: std::ptr::null(),
        columns: 0,
        rows: 0,
        cursor_row: 0,
        cursor_column: 0,
        cursor_visible: false,
        display_offset: 0,
        history_size: 0,
    };
    let ok = unsafe { vt::farcooler_vt_snapshot(handle_of(handle), &mut snapshot) };
    if !ok || snapshot.cells.is_null() {
        return std::ptr::null_mut();
    }

    let count = snapshot.rows as usize * snapshot.columns as usize;
    let cells = unsafe { std::slice::from_raw_parts(snapshot.cells, count) };

    const HEADER: usize = 7;
    let mut out = Vec::with_capacity(HEADER + count * 4);
    out.push(snapshot.columns as jint);
    out.push(snapshot.rows as jint);
    out.push(snapshot.cursor_row as jint);
    out.push(snapshot.cursor_column as jint);
    out.push(jint::from(snapshot.cursor_visible));
    out.push(snapshot.display_offset as jint);
    out.push(snapshot.history_size as jint);
    for cell in cells {
        out.push(cell.ch as jint);
        out.push(cell.fg as jint);
        out.push(cell.bg as jint);
        out.push(cell.flags as jint);
    }

    let Ok(array) = env.new_int_array(out.len() as i32) else {
        return std::ptr::null_mut();
    };
    if env.set_int_array_region(&array, 0, &out).is_err() {
        return std::ptr::null_mut();
    }
    array.into_raw()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeVt_nativeScroll(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    lines: jint,
) {
    unsafe { vt::farcooler_vt_scroll(handle_of(handle), lines) }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeVt_nativeScrollToBottom(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    unsafe { vt::farcooler_vt_scroll_to_bottom(handle_of(handle)) }
}

/// Bytes the program wants written back to the pty, drained.
///
/// Empty is the ordinary answer and is not an error: most feeds produce no
/// reply at all. It has to be asked for after every feed anyway, because a
/// cursor-position report that never reaches the program leaves a full-screen
/// agent waiting for an answer that is sitting in this buffer.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeVt_nativeTakeWrites(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jbyteArray {
    let mut collected: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n =
            unsafe { vt::farcooler_vt_take_writes(handle_of(handle), chunk.as_mut_ptr(), chunk.len()) };
        if n == 0 {
            break;
        }
        collected.extend_from_slice(&chunk[..n]);
        // The C ABI's contract: call again while the result equals the
        // capacity, because a full buffer means there may be more behind it.
        if n < chunk.len() {
            break;
        }
    }
    byte_array_of(&mut env, &collected)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeVt_nativeTakeBell(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jboolean {
    u8::from(unsafe { vt::farcooler_vt_take_bell(handle_of(handle)) })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeVt_nativeTitle(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    let raw = unsafe { vt::farcooler_vt_title(handle_of(handle)) };
    if raw.is_null() {
        return std::ptr::null_mut();
    }
    match (unsafe { CStr::from_ptr(raw) }).to_str() {
        Ok(text) => jstring_of(&mut env, text),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Text the program asked to put on the clipboard (OSC 52), or null.
///
/// Sized then taken, because the core writes nothing when the buffer is short
/// rather than truncating — half a copied command is worse than no copy.
///
/// There is no read counterpart, deliberately: a program asking for the
/// clipboard's CONTENTS is refused inside the core.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeVt_nativeTakeClipboard(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    let h = handle_of(handle);
    let needed = unsafe { vt::farcooler_vt_take_clipboard(h, std::ptr::null_mut(), 0) };
    if needed == 0 {
        return std::ptr::null_mut();
    }
    let mut buffer = vec![0u8; needed];
    let written =
        unsafe { vt::farcooler_vt_take_clipboard(h, buffer.as_mut_ptr(), buffer.len()) };
    if written != needed {
        return std::ptr::null_mut();
    }
    match std::str::from_utf8(&buffer) {
        Ok(text) => jstring_of(&mut env, text),
        Err(_) => std::ptr::null_mut(),
    }
}

/// The URL under a cell, or null.
///
/// The core decides what counts as a URL and which schemes may be opened.
/// Terminal output is not trusted input — an agent prints whatever it read — so
/// keeping the allowlist there makes it one list rather than three.
///
/// Only the text crosses, not the span: this renderer has no ⌘-hover to
/// underline, so the rectangle would be carried for nobody.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeVt_nativeUrlAt(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    row: jint,
    column: jint,
) -> jstring {
    if row < 0 || column < 0 {
        return std::ptr::null_mut();
    }
    let h = handle_of(handle);
    let (row, column) = (clamp_u16(row), clamp_u16(column));
    let mut span = vt::VtUrlSpan { start_row: 0, start_column: 0, end_row: 0, end_column: 0 };
    let needed =
        unsafe { vt::farcooler_vt_url_at(h, row, column, &mut span, std::ptr::null_mut(), 0) };
    if needed == 0 {
        return std::ptr::null_mut();
    }
    let mut buffer = vec![0u8; needed];
    let written = unsafe {
        vt::farcooler_vt_url_at(h, row, column, &mut span, buffer.as_mut_ptr(), buffer.len())
    };
    if written != needed {
        return std::ptr::null_mut();
    }
    match std::str::from_utf8(&buffer) {
        Ok(text) => jstring_of(&mut env, text),
        Err(_) => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeVt_nativeAltScreen(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jboolean {
    u8::from(unsafe { vt::farcooler_vt_alt_screen(handle_of(handle)) })
}

/// Encode a keystroke. Empty when the emulator produces nothing for it.
///
/// 16 bytes is what the C ABI documents as ample for every sequence it can
/// produce, and a short buffer there returns zero rather than a partial escape
/// sequence — which the program would read as garbage typing.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeVt_nativeEncodeKey(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    key: jint,
    modifiers: jint,
) -> jbyteArray {
    let mut buffer = [0u8; 16];
    let n = unsafe {
        vt::farcooler_vt_encode_key(
            handle_of(handle),
            key as u32,
            modifiers as u32,
            buffer.as_mut_ptr(),
            buffer.len(),
        )
    };
    byte_array_of(&mut env, &buffer[..n])
}

/// Encode a mouse event, or null when the program does not want it.
///
/// Null rather than an empty array, because the two mean different things: an
/// empty encoding would be "send nothing", and null is "this event is yours to
/// handle locally" — scroll your own scrollback, select some text. Collapsing
/// them would silently disable scrolling in every pane that is not on the
/// alternate screen.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeVt_nativeEncodeMouse(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    button: jint,
    action: jint,
    column: jint,
    row: jint,
    modifiers: jint,
) -> jbyteArray {
    let mut buffer = [0u8; 32];
    let n = unsafe {
        vt::farcooler_vt_encode_mouse(
            handle_of(handle),
            button as u32,
            action as u32,
            clamp_u16(column),
            clamp_u16(row),
            modifiers as u32,
            buffer.as_mut_ptr(),
            buffer.len(),
        )
    };
    if n == 0 {
        return std::ptr::null_mut();
    }
    byte_array_of(&mut env, &buffer[..n])
}

/// Encode pasted text, bracketed if the program asked for that.
///
/// Sized from the answer rather than guessed at: a paste is arbitrarily long,
/// and the C ABI deliberately writes nothing rather than truncate one. So this
/// asks how much room it needs, allocates exactly that, and calls again.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeVt_nativeEncodePaste(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    text: JString,
) -> jbyteArray {
    let Ok(text) = env.get_string(&text) else { return std::ptr::null_mut() };
    let text: String = text.into();
    let bytes = text.as_bytes();

    let needed = unsafe {
        vt::farcooler_vt_encode_paste(
            handle_of(handle),
            bytes.as_ptr(),
            bytes.len(),
            std::ptr::null_mut(),
            0,
        )
    };
    if needed == 0 {
        return byte_array_of(&mut env, &[]);
    }
    let mut buffer = vec![0u8; needed];
    let written = unsafe {
        vt::farcooler_vt_encode_paste(
            handle_of(handle),
            bytes.as_ptr(),
            bytes.len(),
            buffer.as_mut_ptr(),
            buffer.len(),
        )
    };
    byte_array_of(&mut env, &buffer[..written.min(buffer.len())])
}

/// A Java `int` narrowed to the `u16` the cores take.
///
/// Saturating rather than wrapping: a negative or enormous dimension is a bug
/// in a layout pass, and clamping it produces a strange-looking terminal, while
/// wrapping it would produce a 1-column one from a value of 65,537.
fn clamp_u16(value: jint) -> u16 {
    value.clamp(0, u16::MAX as jint) as u16
}
