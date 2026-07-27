use std::io::Result;

fn main() -> Result<()> {
    // Use a vendored protoc so the build has no system dependency. The proto
    // files are the canonical protocol source of truth and ship with every
    // daemon/client release.
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc");
    unsafe {
        std::env::set_var("PROTOC", protoc);
    }

    println!("cargo:rerun-if-changed=../../proto/overnight.proto");

    let mut cfg = prost_build::Config::new();
    cfg.bytes(["."]);
    cfg.compile_protos(&["../../proto/overnight.proto"], &["../../proto"])?;
    Ok(())
}
