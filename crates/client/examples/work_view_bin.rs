//! Host build of the same pure projection used by the browser WASM bindings.
use std::io::{self, Read};

fn main() {
    let mut body = String::new();
    io::stdin().read_to_string(&mut body).unwrap();
    match std::env::args().nth(1).as_deref() {
        Some("/projection") => {
            let (inventory, selection) = serde_json::from_str(&body).unwrap();
            print!("{}", farcooler_client::work_view::view(&inventory, &selection));
        }
        Some("/failure") => {
            let [word, code, what]: [String; 3] = serde_json::from_str(&body).unwrap();
            print!("{}", farcooler_client::work_view::failure(&word, &code, &what));
        }
        _ => panic!("expected /projection or /failure"),
    }
}
