//! What a terminal and the app around it are coloured with.
//!
//! One definition, shared by the VT core that resolves cell colours and by
//! three apps that colour their own surfaces. The alternative — a palette in
//! `vt` and a separate idea of "dark" in each client — is how a light terminal
//! ends up inside black chrome, which is the "two applications" failure both
//! phone apps forced dark to avoid in the first place.

/// A packed `0x00RRGGBB` colour, the same form `vt`'s snapshot uses.
pub type Rgb = u32;

/// A complete colouring: the terminal's palette, plus which way the app's own
/// surfaces should go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub name: String,
    /// Whether the app's own surfaces go dark.
    ///
    /// Carried rather than derived from the background's luminance. A theme
    /// author picking a mid-grey ground gets to say which way the chrome
    /// should go, and a computed answer would flip the entire app across a
    /// one-point change in one colour.
    pub dark: bool,
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Rgb,
    /// The sixteen ANSI colours in the order `SGR 30-37` and `90-97` name
    /// them: eight normal, then eight bright. Everything above 16 is the xterm
    /// cube, computed rather than stored — no theme hand-picks 240 shades.
    pub ansi: [Rgb; 16],
}

impl Theme {
    /// The nineteen values the VT core needs, in the order its FFI expects:
    /// sixteen ANSI, then foreground, background, cursor.
    ///
    /// Positional rather than a struct across the boundary, because that
    /// boundary is POD-only and a struct would need a declaration maintained
    /// by hand in Swift and again in Kotlin.
    pub fn packed(&self) -> [Rgb; 19] {
        let mut out = [0; 19];
        out[..16].copy_from_slice(&self.ansi);
        out[16] = self.foreground;
        out[17] = self.background;
        out[18] = self.cursor;
        out
    }

    /// The contrast ratio between foreground and background, by WCAG's
    /// definition.
    ///
    /// Here rather than in a test helper because it is the one property that
    /// decides whether a theme is shippable at all, and a built-in nobody can
    /// read is a bug the moment it is added rather than the moment someone
    /// selects it.
    pub fn contrast(&self) -> f64 {
        let l1 = relative_luminance(self.foreground);
        let l2 = relative_luminance(self.background);
        let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }
}

/// WCAG 2.x relative luminance.
fn relative_luminance(color: Rgb) -> f64 {
    let channel = |shift: u32| {
        let raw = ((color >> shift) & 0xFF) as f64 / 255.0;
        if raw <= 0.039_28 {
            raw / 12.92
        } else {
            ((raw + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

/// The theme every client starts on.
///
/// Nord, not the palette this app shipped with. Asked for directly — "choose a
/// standard popular dark theme as the default" — and Nord is the one of the
/// popular darks that best answers the complaint that started this: a soft
/// blue-grey ground at `#2E3440` rather than the near-black `#121419` that
/// hurt to look at, and 9.25:1 on its body text, which is comfortably readable
/// without the glare of white on true black.
///
/// The old palette stays as a built-in named "Far Cooler", so anyone who liked
/// it keeps it one tap away. Existing terminals DO change colour on upgrade;
/// that is the point of the request rather than a side effect of it.
pub const DEFAULT_THEME: &str = "Nord";

/// Every theme compiled in, in the order a picker should show them.
///
/// Built in rather than fetched, so a phone that has never reached a host
/// still has a light mode, and so choosing one costs no round trip.
pub fn built_in() -> Vec<Theme> {
    vec![
        // ---- light ----
        // `base01`, not `base00`, as the foreground.
        //
        // Solarized names `base00` as body text on the light ground, and that
        // pairing measures 4.13:1 — under AA, which the readability test
        // caught. `base01` is Solarized's own "emphasized content" tone on the
        // same ground and clears it, so this stays inside the palette rather
        // than inventing a colour for it. A deviation, recorded: this is
        // Solarized Light one step darker on the text.
        theme("Solarized Light", false, 0xFD_F6_E3, 0x58_6E_75, 0x58_6E_75, SOLARIZED),
        theme("GitHub Light", false, 0xFF_FF_FF, 0x24_29_2F, 0x24_29_2F, [
            0x24_29_2F, 0xCF_22_2E, 0x11_63_29, 0x4D_2D_00, 0x09_69_DA, 0x82_50_DF, 0x1B_7C_83,
            0x6E_77_81, 0x57_60_6A, 0xA4_0E_26, 0x1A_7F_37, 0x63_3C_01, 0x21_8B_FF, 0xA4_75_F9,
            0x31_92_AA, 0x8C_95_9F,
        ]),
        theme("Tomorrow", false, 0xFF_FF_FF, 0x4D_4D_4C, 0x4D_4D_4C, [
            0x00_00_00, 0xC8_28_29, 0x71_8C_00, 0xEA_B7_00, 0x42_71_AE, 0x89_59_A8, 0x3E_99_9F,
            0x70_78_80, 0x00_00_00, 0xC8_28_29, 0x71_8C_00, 0xEA_B7_00, 0x42_71_AE, 0x89_59_A8,
            0x3E_99_9F, 0x30_30_30,
        ]),
        // ---- dark ----
        theme("Solarized Dark", true, 0x00_2B_36, 0x83_94_96, 0x93_A1_A1, SOLARIZED),
        theme("Nord", true, 0x2E_34_40, 0xD8_DE_E9, 0xD8_DE_E9, [
            0x3B_42_52, 0xBF_61_6A, 0xA3_BE_8C, 0xEB_CB_8B, 0x81_A1_C1, 0xB4_8E_AD, 0x88_C0_D0,
            0xE5_E9_F0, 0x4C_56_6A, 0xBF_61_6A, 0xA3_BE_8C, 0xEB_CB_8B, 0x81_A1_C1, 0xB4_8E_AD,
            0x8F_BC_BB, 0xEC_EF_F4,
        ]),
        theme("Dracula", true, 0x28_2A_36, 0xF8_F8_F2, 0xF8_F8_F2, [
            0x21_22_2C, 0xFF_55_55, 0x50_FA_7B, 0xF1_FA_8C, 0xBD_93_F9, 0xFF_79_C6, 0x8B_E9_FD,
            0xF8_F8_F2, 0x62_72_A4, 0xFF_6E_6E, 0x69_FF_94, 0xFF_FF_A5, 0xD6_AC_FF, 0xFF_92_DF,
            0xA4_FF_FF, 0xFF_FF_FF,
        ]),
        theme("Gruvbox Dark", true, 0x28_28_28, 0xEB_DB_B2, 0xEB_DB_B2, [
            0x28_28_28, 0xCC_24_1D, 0x98_97_1A, 0xD7_99_21, 0x45_85_88, 0xB1_62_86, 0x68_9D_6A,
            0xA8_99_84, 0x92_83_74, 0xFB_49_34, 0xB8_BB_26, 0xFA_BD_2F, 0x83_A5_98, 0xD3_86_9B,
            0x8E_C0_7C, 0xEB_DB_B2,
        ]),
        theme("Catppuccin Mocha", true, 0x1E_1E_2E, 0xCD_D6_F4, 0xF5_E0_DC, [
            0x45_47_5A, 0xF3_8B_A8, 0xA6_E3_A1, 0xF9_E2_AF, 0x89_B4_FA, 0xF5_C2_E7, 0x94_E2_D5,
            0xBA_C2_DE, 0x58_5B_70, 0xF3_8B_A8, 0xA6_E3_A1, 0xF9_E2_AF, 0x89_B4_FA, 0xF5_C2_E7,
            0x94_E2_D5, 0xA6_AD_C8,
        ]),
        // The palette this app shipped with, kept so anyone who liked it can
        // have it back in one tap. No longer the default — see `DEFAULT_THEME`.
        theme("Far Cooler", true, 0x12_14_19, 0xDB_E0_E8, 0xDB_E0_E8, [
            0x42_47_54, 0xF0_61_66, 0x6B_D1_82, 0xE6_C0_5C, 0x70_A9_F2, 0xCA_8C_F0, 0x5C_C9_D1,
            0xD4_D9_E0, 0x6B_72_82, 0xFF_82_85, 0x8F_EB_A1, 0xFA_DB_7D, 0x94_C2_FF, 0xE1_AE_FF,
            0x82_E5_EB, 0xF5_F7_FA,
        ]),
        // ---- contrast ----
        //
        // Ours rather than borrowed. Every well-known theme above is tuned for
        // taste and lands somewhere between 4.5:1 and 12:1; these two exist to
        // clear AAA (7:1) on the body text and to keep every ANSI colour
        // legible on their own ground, which is a different goal and not one
        // any of them was designed for.
        theme("High Contrast Dark", true, 0x00_00_00, 0xFF_FF_FF, 0xFF_FF_00, [
            0x00_00_00, 0xFF_6B_6B, 0x5C_FF_5C, 0xFF_F0_5C, 0x6B_B8_FF, 0xFF_7D_FF, 0x5C_F0_FF,
            0xE0_E0_E0, 0x8A_8A_8A, 0xFF_A8_A8, 0xA8_FF_A8, 0xFF_FA_A8, 0xA8_D4_FF, 0xFF_B8_FF,
            0xA8_FA_FF, 0xFF_FF_FF,
        ]),
        theme("High Contrast Light", false, 0xFF_FF_FF, 0x00_00_00, 0x00_00_00, [
            0x00_00_00, 0xA5_00_00, 0x00_5C_00, 0x5C_47_00, 0x00_33_C7, 0x8A_00_8A, 0x00_57_5C,
            0x40_40_40, 0x5C_5C_5C, 0xC7_00_00, 0x00_75_00, 0x75_5C_00, 0x00_45_E0, 0xA5_00_A5,
            0x00_70_75, 0x00_00_00,
        ]),
    ]
}

/// Solarized's sixteen, shared by its light and dark variants — which is the
/// whole idea of that palette: one set of accent colours over two grounds.
const SOLARIZED: [Rgb; 16] = [
    0x07_36_42, 0xDC_32_2F, 0x85_99_00, 0xB5_89_00, 0x26_8B_D2, 0xD3_36_82, 0x2A_A1_98, 0xEE_E8_D5,
    0x00_2B_36, 0xCB_4B_16, 0x58_6E_75, 0x65_7B_83, 0x83_94_96, 0x6C_71_C4, 0x93_A1_A1, 0xFD_F6_E3,
];

fn theme(name: &str, dark: bool, bg: Rgb, fg: Rgb, cursor: Rgb, ansi: [Rgb; 16]) -> Theme {
    Theme { name: name.to_string(), dark, background: bg, foreground: fg, cursor, ansi }
}

/// The theme a name refers to among the built-ins, or `None`.
pub fn built_in_named(name: &str) -> Option<Theme> {
    built_in().into_iter().find(|t| t.name == name)
}

/// The one every client falls back to.
pub fn default_theme() -> Theme {
    built_in_named(DEFAULT_THEME).expect("the default theme is built in")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_a_soft_dark_rather_than_near_black() {
        // The request that started this: a popular dark theme as the default,
        // because the near-black one hurt to look at. Pinned by its properties
        // rather than only its name, so swapping the choice later has to keep
        // clearing the bar that made it the answer.
        let d = default_theme();
        assert!(d.dark, "the default is dark");
        assert_ne!(d.background, 0x12_14_19, "still the old near-black ground");
        assert!(
            relative_luminance(d.background) > relative_luminance(0x12_14_19),
            "the default ground must be lighter than the near-black it replaced"
        );
        assert!(d.contrast() >= 7.0, "readable without glare: {:.1}:1", d.contrast());
    }

    #[test]
    fn the_old_palette_is_still_available_unchanged() {
        // Changing the default changes what people are looking at. That was
        // asked for; taking the old colours away was not, so they keep their
        // exact values under a name. These are the constants that were in
        // `vt`'s `grid.rs`.
        let old = built_in_named("Far Cooler").expect("the old palette is still offered");
        assert_eq!(old.background, 0x12_14_19);
        assert_eq!(old.foreground, 0xDB_E0_E8);
        assert_eq!(old.ansi[0], 0x42_47_54);
        assert_eq!(old.ansi[15], 0xF5_F7_FA);
    }

    #[test]
    fn every_built_in_is_readable() {
        // A shipped theme nobody can read is a bug when it is added, not when
        // somebody selects it. 4.5:1 is WCAG AA for body text, and terminal
        // output is body text at a small size.
        for theme in built_in() {
            assert!(
                theme.contrast() >= 4.5,
                "{} has {:.1}:1 contrast, below AA",
                theme.name,
                theme.contrast()
            );
        }
    }

    #[test]
    fn the_contrast_themes_clear_aaa() {
        // Otherwise they are just two more themes with a promise in the name.
        for name in ["High Contrast Dark", "High Contrast Light"] {
            let theme = built_in_named(name).expect(name);
            assert!(theme.contrast() >= 7.0, "{name} has {:.1}:1", theme.contrast());
        }
    }

    #[test]
    fn light_and_dark_are_both_offered() {
        // The whole reason this exists: there was no light anything.
        assert!(built_in().iter().any(|t| !t.dark), "no light theme");
        assert!(built_in().iter().any(|t| t.dark), "no dark theme");
    }

    #[test]
    fn names_are_unique() {
        // A picker keys on the name and a client stores it — two themes
        // sharing one would make the stored choice ambiguous.
        let mut names: Vec<String> = built_in().into_iter().map(|t| t.name).collect();
        names.sort();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate theme names");
    }

    #[test]
    fn packing_puts_the_three_specials_after_the_sixteen() {
        // The FFI contract, pinned: everything downstream indexes this by
        // position, in Swift and in Kotlin, with no struct to keep them honest.
        let d = default_theme();
        let packed = d.packed();
        assert_eq!(packed[..16], d.ansi);
        assert_eq!(packed[16], d.foreground);
        assert_eq!(packed[17], d.background);
        assert_eq!(packed[18], d.cursor);
    }

    #[test]
    fn luminance_matches_the_wcag_anchors() {
        // Black is 0, white is 1, and the two together are 21:1 — the fixed
        // points the ratio is defined against.
        assert!(relative_luminance(0x00_00_00).abs() < 1e-9);
        assert!((relative_luminance(0xFF_FF_FF) - 1.0).abs() < 1e-9);
        let bw = Theme { foreground: 0xFF_FF_FF, background: 0x00_00_00, ..default_theme() };
        assert!((bw.contrast() - 21.0).abs() < 0.01);
    }
}
