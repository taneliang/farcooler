//! The header is the contract Swift and Kotlin compile against, and nothing in
//! the compiler checks it against the Rust side. A function declared here but
//! not exported fails at link time; one exported and NOT declared is invisible
//! to every client, which is how a feature ships and nobody can call it.
//!
//! Both directions are checked, because both have happened.

const HEADER: &str = include_str!("../include/farcooler_client.h");
/// The ABI, which is `ffi.rs` and the files beside it that export entry
/// points of their own.
const FFI: &str = concat!(include_str!("../src/ffi.rs"), include_str!("../src/ffi/teammate.rs"));

fn declared() -> Vec<String> {
    let mut found = Vec::new();
    for line in HEADER.lines() {
        let Some(start) = line.find("farcooler_client_") else { continue };
        let rest = &line[start..];
        let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if rest[name.len()..].starts_with('(') && !found.contains(&name) {
            found.push(name);
        }
    }
    found
}

#[test]
fn every_declared_function_is_exported() {
    let declared = declared();
    assert!(declared.len() >= 6, "the header lost declarations: {declared:?}");
    for name in &declared {
        assert!(
            FFI.contains(&format!("pub unsafe extern \"C\" fn {name}"))
                || FFI.contains(&format!("pub extern \"C\" fn {name}")),
            "the header declares {name}, but the ABI does not export it"
        );
    }
}

#[test]
fn every_exported_function_is_declared() {
    let declared = declared();
    for line in FFI.lines() {
        let Some(index) = line.find("extern \"C\" fn farcooler_client_") else { continue };
        let rest = &line[index + "extern \"C\" fn ".len()..];
        let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        assert!(
            declared.contains(&name),
            "the ABI exports {name}, but no client can see it: the header does not declare it"
        );
    }
}
