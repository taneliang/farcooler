//! tmux's layout string, read just far enough to scale it.
//!
//! `resize-window` does not keep a window's proportions. tmux takes every cell
//! the window loses from the cells at its right and bottom edges, so a window
//! of an agent at 78 columns beside two stacked shells at 26, narrowed from 105
//! columns to 55, comes out as the agent at 53 and the shells at ONE column
//! each. Nothing in tmux puts the ratio back.
//!
//! So a resize that should keep the arrangement reads the split tree first,
//! scales every cell to the new size, and hands the result back to
//! `select-layout`. This is the only place Far Cooler parses the string; it
//! parses nothing it does not write straight back, pane ids included.
//!
//! The format, as tmux's `layout_dump` writes it:
//!
//! ```text
//! b549,55x36,0,0{40x36,0,0,0,14x36,41,0[14x18,41,0,1,14x17,41,19,2]}
//! ^^^^ checksum  WxH,X,Y then {…} side by side, […] stacked, or ,ID for a pane
//! ```
//!
//! Siblings are separated by a one-cell divider, which is part of neither.

/// The narrowest a pane is scaled to, where the window has room for it.
///
/// Ten columns shows a prompt and a word or two; the one column tmux leaves
/// shows nothing at all. Where the window cannot give every pane this much,
/// the floor drops until it can.
pub const MIN_COLUMNS: u32 = 10;

/// The shortest a pane is scaled to, where the window has room for it.
pub const MIN_ROWS: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Cell {
    columns: u32,
    rows: u32,
    x: u32,
    y: u32,
    kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    /// A pane, and its id as tmux wrote it (without the `%`).
    Pane(u32),
    /// Children side by side, `{…}`.
    Across(Vec<Cell>),
    /// Children stacked, `[…]`.
    Down(Vec<Cell>),
}

/// tmux's `layout_checksum`: a 16-bit rotate-and-add over the body.
///
/// `select-layout` refuses a string whose checksum does not match, so a scaled
/// layout needs a new one.
pub fn checksum(body: &str) -> u16 {
    body.bytes().fold(0u16, |sum, byte| sum.rotate_right(1).wrapping_add(u16::from(byte)))
}

/// The same arrangement at `columns` x `rows`, every pane keeping its share.
///
/// `None` when the string is not one this can read, when the window is too
/// small for every pane to have even one cell, or when there is nothing to do
/// (the size is unchanged, or the window holds one pane, which tmux sizes
/// right on its own). A caller then leaves tmux's own resize in place.
pub fn scale(layout: &str, columns: u32, rows: u32) -> Option<String> {
    let (_, body) = layout.split_once(',')?;
    let mut root = Parser { s: body.as_bytes(), at: 0 }.whole()?;
    if matches!(root.kind, Kind::Pane(_)) || (root.columns, root.rows) == (columns, rows) {
        return None;
    }
    let floor_columns = (1..=MIN_COLUMNS).rev().find(|f| min_size(&root, true, *f) <= columns)?;
    let floor_rows = (1..=MIN_ROWS).rev().find(|f| min_size(&root, false, *f) <= rows)?;
    resize(&mut root, columns, rows, floor_columns, floor_rows);
    place(&mut root, 0, 0);
    let mut out = String::new();
    dump(&root, &mut out);
    Some(format!("{:04x},{out}", checksum(&out)))
}

/// The least `cell` can shrink to along one axis with panes no smaller than
/// `floor` along it.
fn min_size(cell: &Cell, horizontal: bool, floor: u32) -> u32 {
    match &cell.kind {
        Kind::Pane(_) => floor,
        Kind::Across(children) | Kind::Down(children) => {
            let along = matches!(cell.kind, Kind::Across(_)) == horizontal;
            let sizes = children.iter().map(|c| min_size(c, horizontal, floor));
            if along {
                sizes.sum::<u32>() + children.len() as u32 - 1
            } else {
                sizes.max().unwrap_or(floor)
            }
        }
    }
}

fn resize(cell: &mut Cell, columns: u32, rows: u32, floor_columns: u32, floor_rows: u32) {
    cell.columns = columns;
    cell.rows = rows;
    let horizontal = matches!(cell.kind, Kind::Across(_));
    let (Kind::Across(children) | Kind::Down(children)) = &mut cell.kind else { return };
    let dividers = children.len() as u32 - 1;
    let (total, floor) =
        if horizontal { (columns, floor_columns) } else { (rows, floor_rows) };
    let old: Vec<u32> =
        children.iter().map(|c| if horizontal { c.columns } else { c.rows }).collect();
    let mins: Vec<u32> = children.iter().map(|c| min_size(c, horizontal, floor)).collect();
    let sizes = share(&old, total - dividers, &mins);
    for (child, size) in children.iter_mut().zip(sizes) {
        let (c, r) = if horizontal { (size, rows) } else { (columns, size) };
        resize(child, c, r, floor_columns, floor_rows);
    }
}

/// Split `total` in proportion to `old`, never giving a share less than its
/// minimum. `mins` must fit in `total`, which `scale` has already made sure of.
///
/// A share whose proportion falls below its minimum is pinned there and the
/// rest are shared again without it, until none falls short. Rounding then
/// goes by largest remainder, first come first served on a tie, so the shares
/// always add up exactly.
fn share(old: &[u32], total: u32, mins: &[u32]) -> Vec<u32> {
    let mut pinned: Vec<Option<u32>> = vec![None; old.len()];
    loop {
        let free = total - pinned.iter().flatten().sum::<u32>();
        let weight: u32 = (0..old.len()).filter(|&i| pinned[i].is_none()).map(|i| old[i]).sum();
        let short: Vec<usize> = (0..old.len())
            .filter(|&i| pinned[i].is_none())
            .filter(|&i| f64::from(free) * f64::from(old[i]) / f64::from(weight.max(1)) < f64::from(mins[i]))
            .collect();
        if short.is_empty() {
            let ideal: Vec<f64> = (0..old.len())
                .map(|i| match pinned[i] {
                    Some(size) => f64::from(size),
                    None => f64::from(free) * f64::from(old[i]) / f64::from(weight.max(1)),
                })
                .collect();
            let mut sizes: Vec<u32> = ideal.iter().map(|v| v.floor() as u32).collect();
            let mut left = total - sizes.iter().sum::<u32>();
            let mut order: Vec<usize> = (0..old.len()).collect();
            order.sort_by(|&a, &b| (ideal[b] - ideal[b].floor()).total_cmp(&(ideal[a] - ideal[a].floor())));
            for i in order {
                if left == 0 {
                    break;
                }
                sizes[i] += 1;
                left -= 1;
            }
            return sizes;
        }
        for i in short {
            pinned[i] = Some(mins[i]);
        }
    }
}

fn place(cell: &mut Cell, x: u32, y: u32) {
    cell.x = x;
    cell.y = y;
    let horizontal = matches!(cell.kind, Kind::Across(_));
    let (Kind::Across(children) | Kind::Down(children)) = &mut cell.kind else { return };
    let (mut cx, mut cy) = (x, y);
    for child in children.iter_mut() {
        place(child, cx, cy);
        if horizontal {
            cx += child.columns + 1;
        } else {
            cy += child.rows + 1;
        }
    }
}

fn dump(cell: &Cell, out: &mut String) {
    out.push_str(&format!("{}x{},{},{}", cell.columns, cell.rows, cell.x, cell.y));
    let (open, close, children) = match &cell.kind {
        Kind::Pane(id) => {
            out.push_str(&format!(",{id}"));
            return;
        }
        Kind::Across(children) => ('{', '}', children),
        Kind::Down(children) => ('[', ']', children),
    };
    out.push(open);
    for (i, child) in children.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        dump(child, out);
    }
    out.push(close);
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn whole(mut self) -> Option<Cell> {
        let cell = self.cell()?;
        (self.at == self.s.len()).then_some(cell)
    }

    fn cell(&mut self) -> Option<Cell> {
        let columns = self.number()?;
        self.eat(b'x')?;
        let rows = self.number()?;
        self.eat(b',')?;
        let x = self.number()?;
        self.eat(b',')?;
        let y = self.number()?;
        let kind = match self.s.get(self.at)? {
            b',' => {
                self.at += 1;
                Kind::Pane(self.number()?)
            }
            b'{' => Kind::Across(self.children(b'}')?),
            b'[' => Kind::Down(self.children(b']')?),
            _ => return None,
        };
        // A cell that holds nothing, or holds zero cells, is not a layout tmux
        // wrote, and dividing by it later would be worse than refusing it now.
        if columns == 0 || rows == 0 {
            return None;
        }
        Some(Cell { columns, rows, x, y, kind })
    }

    fn children(&mut self, close: u8) -> Option<Vec<Cell>> {
        self.at += 1;
        let mut children = vec![self.cell()?];
        while self.s.get(self.at) == Some(&b',') {
            self.at += 1;
            children.push(self.cell()?);
        }
        self.eat(close)?;
        Some(children)
    }

    fn number(&mut self) -> Option<u32> {
        let start = self.at;
        while self.s.get(self.at).is_some_and(u8::is_ascii_digit) {
            self.at += 1;
        }
        std::str::from_utf8(&self.s[start..self.at]).ok()?.parse().ok()
    }

    fn eat(&mut self, byte: u8) -> Option<()> {
        (self.s.get(self.at) == Some(&byte)).then(|| self.at += 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read off a real tmux 3.7: an agent at 78 beside two stacked shells.
    const AGENT_AND_TWO_SHELLS: &str =
        "264e,105x36,0,0{78x36,0,0,0,26x36,79,0[26x18,79,0,1,26x17,79,19,2]}";

    #[test]
    fn the_checksum_is_tmuxs() {
        // Both from `display -p '#{window_layout}'` on a live server.
        assert_eq!(checksum("105x36,0,0{78x36,0,0,0,26x36,79,0[26x18,79,0,1,26x17,79,19,2]}"), 0x264e);
        assert_eq!(checksum("55x36,0,0{53x36,0,0,0,1x36,54,0[1x18,54,0,1,1x17,54,19,2]}"), 0x0970);
    }

    #[test]
    fn narrowing_keeps_the_shares_tmux_throws_away() {
        // tmux's own answer at 55 is {53, 1[…]}. 54 usable columns at 78:26 is
        // 40.5:13.5; the tie goes to the first.
        let out = scale(AGENT_AND_TWO_SHELLS, 55, 36).unwrap();
        let body = "55x36,0,0{41x36,0,0,0,13x36,42,0[13x18,42,0,1,13x17,42,19,2]}";
        assert_eq!(out, format!("{:04x},{body}", checksum(body)));
    }

    #[test]
    fn widening_back_comes_back_to_about_where_it_was() {
        let narrow = scale(AGENT_AND_TWO_SHELLS, 55, 36).unwrap();
        let wide = scale(&narrow, 105, 36).unwrap();
        assert!(wide.contains(",0,0{79x36,0,0,0,25x36,80,0["), "{wide}");
    }

    #[test]
    fn rows_scale_too_and_every_offset_follows() {
        let tall = scale(AGENT_AND_TWO_SHELLS, 105, 72).unwrap();
        assert!(tall.ends_with("[26x37,79,0,1,26x34,79,38,2]}"), "{tall}");
        assert_eq!(&tall[..4], format!("{:04x}", checksum(&tall[5..])));
    }

    #[test]
    fn a_small_share_is_held_at_the_floor() {
        // 95:8 of 104 columns at 30 wide would leave the right-hand pane two.
        let lopsided = "0000,104x36,0,0{95x36,0,0,0,8x36,96,0,1}";
        let out = scale(lopsided, 30, 36).unwrap();
        assert!(out.ends_with(",30x36,0,0{19x36,0,0,0,10x36,20,0,1}"), "{out}");
    }

    #[test]
    fn the_floor_gives_way_when_the_window_cannot_hold_it() {
        // Three panes across 20 columns cannot all have ten; they get six each.
        let three = "0000,62x10,0,0{20x10,0,0,0,20x10,21,0,1,20x10,42,0,2}";
        let out = scale(three, 20, 10).unwrap();
        assert!(out.ends_with(",20x10,0,0{6x10,0,0,0,6x10,7,0,1,6x10,14,0,2}"), "{out}");
    }

    #[test]
    fn nothing_to_do_or_nothing_readable_is_none() {
        assert_eq!(scale(AGENT_AND_TWO_SHELLS, 105, 36), None, "same size");
        assert_eq!(scale("b25d,80x24,0,0,0", 40, 24), None, "one pane");
        assert_eq!(scale("0000,5x5,0,0{2x5,0,0,0,2x5,3,0,1}", 1, 5), None, "no room for two");
        assert_eq!(scale("garbage", 40, 24), None);
        assert_eq!(scale("0000,80x24,0,0{40x24,0,0,0", 40, 24), None, "truncated");
    }
}
