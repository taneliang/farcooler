//! Reads a plan-view request on stdin and prints `plan_view::json`'s answer: the program the web capture scripts
//! name in `PLAN_VIEW_BIN` (`services/web/test/browser/plan-capture.mjs`), since a page there can't load the wasm core.

use std::io::Read;

fn main() {
    let mut request = String::new();
    std::io::stdin().read_to_string(&mut request).expect("a request on stdin");
    print!("{}", farcooler_client::plan_view::json(&request));
}
