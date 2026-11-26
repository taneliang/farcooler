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
        Some("/board") => {
            let (board, scope): (_, String) = serde_json::from_str(&body).unwrap();
            print!("{}", farcooler_client::web_board::board(&board, &scope));
        }
        Some("/ticket") => {
            let (task, scope, guard, links): (_, String, Option<String>, _) = serde_json::from_str(&body).unwrap();
            print!("{}", farcooler_client::web_board::ticket(&task, &scope, true, guard.as_deref(), links));
        }
        Some("/ticket-form") => print!("{}", farcooler_client::web_board::form(None)),
        Some("/layout") => print!("{}", farcooler_client::workspace_layout::json(&body)),
        _ => panic!("unknown projection"),
    }
}
