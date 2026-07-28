//! The header is the contract every renderer compiles against, and nothing in
//! the compiler checks it against the Rust side. A function renamed here and
//! not there fails at link time — but a CONSTANT that drifts fails silently and
//! at runtime, sending the wrong key or painting the wrong colour. So the two
//! are compared here instead.

use std::collections::BTreeMap;

const HEADER: &str = include_str!("../include/overnight_vt.h");
const FFI: &str = include_str!("../src/ffi.rs");

/// Every `overnight_vt_*` the header declares.
fn declared_functions() -> Vec<String> {
    let mut found = Vec::new();
    for line in HEADER.lines() {
        let Some(start) = line.find("overnight_vt_") else { continue };
        let rest = &line[start..];
        let name: String =
            rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        // A declaration is followed by its parameter list.
        if rest[name.len()..].starts_with('(') && !found.contains(&name) {
            found.push(name);
        }
    }
    found
}

/// Every `#define OVERNIGHT_VT_<NAME> <value>` in the header.
fn header_constants() -> BTreeMap<String, u32> {
    let mut map = BTreeMap::new();
    for line in HEADER.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("#define OVERNIGHT_VT_") else { continue };
        let mut parts = rest.split_whitespace();
        let (Some(name), Some(value)) = (parts.next(), parts.next()) else { continue };
        if let Some(v) = parse_c_value(&rest[name.len()..]) {
            map.insert(name.to_string(), v);
        } else {
            panic!("cannot parse constant OVERNIGHT_VT_{name} = {value}");
        }
    }
    map
}

/// Handles `0xE000u`, `12u`, and `(1u << 3)`.
fn parse_c_value(text: &str) -> Option<u32> {
    let t = text.trim().trim_start_matches('(').trim_end_matches(')').trim();
    if let Some((lhs, rhs)) = t.split_once("<<") {
        return Some(number(lhs)? << number(rhs)?);
    }
    number(t)
}

fn number(text: &str) -> Option<u32> {
    let t = text.trim().trim_end_matches('u');
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        t.parse().ok()
    }
}

#[test]
fn every_declared_function_is_exported_from_rust() {
    let declared = declared_functions();
    assert!(declared.len() >= 13, "the header lost declarations: {declared:?}");

    for name in &declared {
        let signature = format!("pub unsafe extern \"C\" fn {name}");
        let safe_signature = format!("pub extern \"C\" fn {name}");
        assert!(
            FFI.contains(&signature) || FFI.contains(&safe_signature),
            "the header declares {name}, but the ABI does not export it"
        );
    }
}

#[test]
fn every_exported_function_is_declared_in_the_header() {
    // The other direction: an ABI function no renderer can see is dead weight,
    // and usually means someone forgot to publish it.
    let declared = declared_functions();
    for line in FFI.lines() {
        let Some(idx) = line.find("extern \"C\" fn overnight_vt_") else { continue };
        let rest = &line[idx + "extern \"C\" fn ".len()..];
        let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        assert!(
            declared.contains(&name),
            "the ABI exports {name}, but the header does not declare it"
        );
    }
}

#[test]
fn the_constants_agree() {
    use overnight_vt::ffi::*;

    let header = header_constants();
    let rust: BTreeMap<&str, u32> = BTreeMap::from([
        ("FLAG_BOLD", FLAG_BOLD as u32),
        ("FLAG_ITALIC", FLAG_ITALIC as u32),
        ("FLAG_UNDERLINE", FLAG_UNDERLINE as u32),
        ("FLAG_INVERSE", FLAG_INVERSE as u32),
        ("FLAG_WIDE", FLAG_WIDE as u32),
        ("KEY_ENTER", KEY_ENTER),
        ("KEY_TAB", KEY_TAB),
        ("KEY_BACKSPACE", KEY_BACKSPACE),
        ("KEY_ESCAPE", KEY_ESCAPE),
        ("KEY_UP", KEY_UP),
        ("KEY_DOWN", KEY_DOWN),
        ("KEY_RIGHT", KEY_RIGHT),
        ("KEY_LEFT", KEY_LEFT),
        ("KEY_HOME", KEY_HOME),
        ("KEY_END", KEY_END),
        ("KEY_PAGE_UP", KEY_PAGE_UP),
        ("KEY_PAGE_DOWN", KEY_PAGE_DOWN),
        ("KEY_INSERT", KEY_INSERT),
        ("KEY_DELETE", KEY_DELETE),
        ("KEY_F1", KEY_F1),
        ("MOD_SHIFT", MOD_SHIFT),
        ("MOD_ALT", MOD_ALT),
        ("MOD_CTRL", MOD_CTRL),
        ("MOUSE_LEFT", MOUSE_LEFT),
        ("MOUSE_MIDDLE", MOUSE_MIDDLE),
        ("MOUSE_RIGHT", MOUSE_RIGHT),
        ("MOUSE_WHEEL_UP", MOUSE_WHEEL_UP),
        ("MOUSE_WHEEL_DOWN", MOUSE_WHEEL_DOWN),
        ("MOUSE_PRESS", MOUSE_PRESS),
        ("MOUSE_RELEASE", MOUSE_RELEASE),
        ("MOUSE_MOVE", MOUSE_MOVE),
    ]);

    for (name, expected) in &rust {
        match header.get(*name) {
            Some(actual) => assert_eq!(
                actual, expected,
                "OVERNIGHT_VT_{name} is {actual} in the header but {expected} in Rust"
            ),
            None => panic!("the header is missing OVERNIGHT_VT_{name}"),
        }
    }

    for name in header.keys() {
        assert!(rust.contains_key(name.as_str()), "the header defines an unknown OVERNIGHT_VT_{name}");
    }
}
