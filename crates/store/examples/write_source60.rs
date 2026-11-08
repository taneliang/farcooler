//! Write a runner database at schema 60, the one every install before the
//! engine cutover is on, for a scratch runner to start in recovery from
//! (ov-984's end to end). `cargo run -p farcooler-store --features testing
//! --example write_source60 -- <home>/farcooler.db`
fn main() {
    let path = std::env::args().nth(1).expect("the database path to write");
    assert!(!std::path::Path::new(&path).exists(), "{path} exists; this writes a new database only");
    farcooler_store::testing::write_source60(std::path::Path::new(&path));
}
