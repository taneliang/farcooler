//! Layout preset names accepted by the CLI.

use farcooler_protocol::v1::LayoutPreset;

pub fn parse_preset(text: &str) -> Option<LayoutPreset> {
    Some(match text.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "even-horizontal" | "columns" | "cols" | "row" => LayoutPreset::EvenHorizontal,
        "even-vertical" | "rows" | "stack" | "column" => LayoutPreset::EvenVertical,
        "main-vertical" | "main" | "main-v" => LayoutPreset::MainVertical,
        "main-horizontal" | "main-h" => LayoutPreset::MainHorizontal,
        "tiled" | "grid" | "tile" => LayoutPreset::Tiled,
        _ => return None,
    })
}
