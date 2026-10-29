//! Layouts (spec §4) as a grid of tracks.
//!
//! A layout is a grid of columns and rows; every area covers a rectangle
//! of whole tracks. A drawing gets a track boundary wherever an area edge
//! is, so edges that line up in the drawing line up on screen. A grid
//! layout (§4.1) has one track per grid cell. Sizes belong to tracks: a
//! track is cells, a percentage of the window or `*`; the `*` tracks share
//! what is left in proportion to their extent in the drawing, never going
//! below their minimum size.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl Rect {
    pub fn new(x: usize, y: usize, w: usize, h: usize) -> Self {
        Rect { x, y, w, h }
    }
    pub fn contains(&self, x: usize, y: usize) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Columns,
    Rows,
}

/// One column or row of the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Track {
    /// Written size; `None` fills.
    pub size: Option<Dim>,
    /// Extent in the drawing: filling tracks share space in proportion to
    /// it, and it is their minimum size (the drawing is the smallest size).
    pub weight: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    /// Each area in tracks: `x`/`w` count columns, `y`/`h` rows.
    pub areas: BTreeMap<char, Rect>,
    pub columns: Vec<Track>,
    pub rows: Vec<Track>,
}

/// A layout drawing: each character names an area, and every area must be
/// a rectangle.
pub fn analyze(drawing: &[String]) -> Result<Grid, String> {
    build(drawing, false)
}

/// A grid layout (§4.1): like [`analyze`], but every drawing cell is a
/// track of its own, so `- columns` and `- rows` size each one.
pub fn analyze_grid(drawing: &[String]) -> Result<Grid, String> {
    build(drawing, true)
}

fn build(drawing: &[String], every_cell: bool) -> Result<Grid, String> {
    if drawing.is_empty() {
        return Err("layout has no drawing".into());
    }
    let rows: Vec<Vec<char>> = drawing.iter().map(|l| l.chars().collect()).collect();
    let width = rows[0].len();
    if rows.iter().any(|r| r.len() != width) {
        return Err("layout drawing rows must all have the same width".into());
    }
    if let Some(c) = rows.iter().flatten().find(|c| c.is_whitespace()) {
        return Err(format!("layout drawing may not contain whitespace ({c:?})"));
    }
    let height = rows.len();

    let mut cells: BTreeMap<char, Rect> = BTreeMap::new();
    let mut counts: BTreeMap<char, usize> = BTreeMap::new();
    for (y, row) in rows.iter().enumerate() {
        for (x, &c) in row.iter().enumerate() {
            *counts.entry(c).or_default() += 1;
            let r = cells.entry(c).or_insert(Rect::new(x, y, 1, 1));
            let x0 = r.x.min(x);
            let y0 = r.y.min(y);
            let x1 = (r.x + r.w).max(x + 1);
            let y1 = (r.y + r.h).max(y + 1);
            *r = Rect::new(x0, y0, x1 - x0, y1 - y0);
        }
    }
    for (c, r) in &cells {
        if counts[c] != r.w * r.h {
            return Err(format!("area `{c}` is not a rectangle"));
        }
    }
    let cuts = |len: usize, edges: Vec<usize>| -> Vec<usize> {
        if every_cell { (0..=len).collect() } else { edges.into_iter().chain([0, len]).collect::<BTreeSet<_>>().into_iter().collect() }
    };
    let xs = cuts(width, cells.values().flat_map(|r| [r.x, r.x + r.w]).collect());
    let ys = cuts(height, cells.values().flat_map(|r| [r.y, r.y + r.h]).collect());
    let at = |cuts: &[usize], v: usize| cuts.iter().position(|&c| c == v).unwrap_or(0);
    let areas = cells
        .iter()
        .map(|(c, r)| {
            let (x, y) = (at(&xs, r.x), at(&ys, r.y));
            (*c, Rect::new(x, y, at(&xs, r.x + r.w) - x, at(&ys, r.y + r.h) - y))
        })
        .collect();
    let tracks = |cuts: &[usize]| cuts.windows(2).map(|w| Track { size: None, weight: w[1] - w[0] }).collect();
    Ok(Grid { areas, columns: tracks(&xs), rows: tracks(&ys) })
}

/// A written size (D 2.4): cells, or a percentage of the window. `*` or a
/// missing size fills the remaining space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dim {
    Cells(usize),
    Percent(u8),
}

impl Dim {
    /// `20`, `25%`, `*` (→ `None`).
    pub fn parse(s: &str) -> Result<Option<Dim>, String> {
        if s == "*" {
            return Ok(None);
        }
        if let Some(p) = s.strip_suffix('%') {
            return match p.parse::<u8>() {
                Ok(n) if (1..=100).contains(&n) => Ok(Some(Dim::Percent(n))),
                _ => Err(format!("bad percentage `{s}` (1%..100%)")),
            };
        }
        match s.parse::<usize>() {
            Ok(n) if n > 0 => Ok(Some(Dim::Cells(n))),
            _ => Err(format!("bad size `{s}` (cells, N% or *)")),
        }
    }

    /// `20x10`, `24x*`.
    pub fn parse_pair(s: &str) -> Result<(Option<Dim>, Option<Dim>), String> {
        let (w, h) = s.split_once('x').ok_or_else(|| format!("bad size `{s}` (WIDTHxHEIGHT, e.g. 20x10 or 24x*)"))?;
        Ok((Dim::parse(w)?, Dim::parse(h)?))
    }

    pub fn resolve(self, total: usize) -> usize {
        match self {
            Dim::Cells(n) => n,
            Dim::Percent(p) => total * p as usize / 100,
        }
    }
}

impl fmt::Display for Dim {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Dim::Cells(n) => write!(f, "{n}"),
            Dim::Percent(p) => write!(f, "{p}%"),
        }
    }
}

/// Written sizes of each area: (width, height).
pub type Dims = BTreeMap<char, (Option<Dim>, Option<Dim>)>;

fn span(r: &Rect, axis: Axis) -> (usize, usize) {
    match axis {
        Axis::Columns => (r.x, r.w),
        Axis::Rows => (r.y, r.h),
    }
}

impl Grid {
    pub fn tracks(&self, axis: Axis) -> &[Track] {
        match axis {
            Axis::Columns => &self.columns,
            Axis::Rows => &self.rows,
        }
    }

    /// Turns the areas' written sizes (drawing legend) into track sizes.
    /// An area over several tracks fixes the ones still free, sharing in
    /// proportion to their drawing extent; sizes that disagree are an error.
    pub fn set_sizes(&mut self, dims: &Dims) -> Result<(), String> {
        for axis in [Axis::Columns, Axis::Rows] {
            let wanted: Vec<(char, usize, usize, Dim)> = dims
                .iter()
                .filter_map(|(c, d)| {
                    let dim = if axis == Axis::Columns { d.0 } else { d.1 }?;
                    let (from, len) = span(self.areas.get(c)?, axis);
                    Some((*c, from, len, dim))
                })
                .collect();
            let what = if axis == Axis::Columns { "widths" } else { "heights" };
            let tracks = match axis {
                Axis::Columns => &mut self.columns,
                Axis::Rows => &mut self.rows,
            };
            assign(tracks, wanted, what)?;
        }
        Ok(())
    }
}

fn assign(tracks: &mut [Track], mut wanted: Vec<(char, usize, usize, Dim)>, what: &str) -> Result<(), String> {
    let mut owner: Vec<Option<char>> = vec![None; tracks.len()];
    let free_in = |tracks: &[Track], from: usize, len: usize| tracks[from..from + len].iter().filter(|t| t.size.is_none()).count();
    while !wanted.is_empty() {
        // the area with the fewest free tracks first
        let i = (0..wanted.len()).min_by_key(|&i| free_in(tracks, wanted[i].1, wanted[i].2)).unwrap_or(0);
        let (c, from, len, dim) = wanted.remove(i);
        let percent = matches!(dim, Dim::Percent(_));
        let amount = |d: Dim| match d {
            Dim::Cells(n) => n,
            Dim::Percent(p) => p as usize,
        };
        let other = (from..from + len).find_map(|t| owner[t]).unwrap_or(c);
        let conflict = || format!("conflicting {what} for areas `{other}` and `{c}`");
        let known: Vec<Dim> = tracks[from..from + len].iter().filter_map(|t| t.size).collect();
        if known.iter().any(|d| matches!(d, Dim::Percent(_)) != percent) {
            return Err(format!("{} for areas `{other}` and `{c}` mix cells and percentages", what));
        }
        let used: usize = known.into_iter().map(amount).sum();
        let free: Vec<usize> = (from..from + len).filter(|&t| tracks[t].size.is_none()).collect();
        if free.is_empty() {
            if used != amount(dim) {
                return Err(conflict());
            }
            continue;
        }
        let rest = amount(dim).checked_sub(used).filter(|&r| r > 0).ok_or_else(conflict)?;
        let weight: usize = free.iter().map(|&t| tracks[t].weight).sum();
        let mut left = rest;
        for (k, &t) in free.iter().enumerate() {
            let share = if k + 1 == free.len() { left } else { rest * tracks[t].weight / weight.max(1) };
            if share == 0 {
                return Err(format!("area `{c}` ({dim}) is too small to share among its {} free tracks", free.len()));
            }
            left -= share;
            tracks[t].size = Some(if percent { Dim::Percent(share.min(100) as u8) } else { Dim::Cells(share) });
            owner[t] = Some(c);
        }
    }
    Ok(())
}

/// The content's minimum size of one area, in cells (e.g. a panel drawing
/// and its border).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AreaSize {
    pub min_width: usize,
    pub min_height: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Solved {
    pub rects: BTreeMap<char, Rect>,
    pub too_small: bool,
    pub min_width: usize,
    pub min_height: usize,
}

impl Grid {
    pub fn min_size(&self, sizes: &BTreeMap<char, AreaSize>, width: usize, height: usize) -> (usize, usize) {
        (self.mins(Axis::Columns, sizes, width).iter().sum(), self.mins(Axis::Rows, sizes, height).iter().sum())
    }

    pub fn solve(&self, sizes: &BTreeMap<char, AreaSize>, width: usize, height: usize) -> Solved {
        let (min_width, min_height) = self.min_size(sizes, width, height);
        let cols = self.lengths(Axis::Columns, sizes, width);
        let rows = self.lengths(Axis::Rows, sizes, height);
        let place = |lens: &[usize], from: usize, len: usize| (lens[..from].iter().sum::<usize>(), lens[from..from + len].iter().sum::<usize>());
        let rects = self
            .areas
            .iter()
            .map(|(c, r)| {
                let (x, w) = place(&cols, r.x, r.w);
                let (y, h) = place(&rows, r.y, r.h);
                (*c, Rect::new(x, y, w, h))
            })
            .collect();
        Solved { rects, too_small: width < min_width || height < min_height, min_width, min_height }
    }

    /// Minimum length of each track: a sized track is its size; a filling
    /// track its drawing extent, grown to fit the contents of the areas on
    /// it (an area over several tracks grows the last filling one).
    fn mins(&self, axis: Axis, sizes: &BTreeMap<char, AreaSize>, total: usize) -> Vec<usize> {
        let tracks = self.tracks(axis);
        let mut mins: Vec<usize> = tracks.iter().map(|t| t.size.map_or(t.weight, |d| d.resolve(total))).collect();
        let mut needs: Vec<(usize, usize, usize)> = self
            .areas
            .iter()
            .map(|(c, r)| {
                let (from, len) = span(r, axis);
                let s = sizes.get(c).copied().unwrap_or_default();
                (from, len, if axis == Axis::Columns { s.min_width } else { s.min_height })
            })
            .collect();
        needs.sort_by_key(|n| n.1);
        for (from, len, need) in needs {
            let have: usize = mins[from..from + len].iter().sum();
            if have < need
                && let Some(t) = (from..from + len).rev().find(|&t| tracks[t].size.is_none()) {
                    mins[t] += need - have;
                }
        }
        mins
    }

    fn lengths(&self, axis: Axis, sizes: &BTreeMap<char, AreaSize>, total: usize) -> Vec<usize> {
        let tracks = self.tracks(axis);
        let mins = self.mins(axis, sizes, total);
        let mut lens: Vec<usize> = tracks.iter().map(|t| t.size.map_or(0, |d| d.resolve(total))).collect();
        let flex: Vec<usize> = (0..tracks.len()).filter(|&i| tracks[i].size.is_none()).collect();
        let remaining = total.saturating_sub(lens.iter().sum());
        // Water-filling: share in proportion to drawing extent; a track
        // whose share would fall below its minimum is pinned to the minimum
        // and the others share the rest.
        let mut pinned = vec![false; tracks.len()];
        loop {
            let rest = remaining.saturating_sub(flex.iter().filter(|&&i| pinned[i]).map(|&i| mins[i]).sum());
            let free: Vec<usize> = flex.iter().copied().filter(|&i| !pinned[i]).collect();
            let weight: usize = free.iter().map(|&i| tracks[i].weight).sum();
            let mut changed = false;
            for &i in &free {
                let share = rest * tracks[i].weight / weight.max(1);
                if share < mins[i] {
                    pinned[i] = true;
                    changed = true;
                }
                lens[i] = share;
            }
            if !changed {
                break;
            }
        }
        for &i in &flex {
            if pinned[i] {
                lens[i] = mins[i];
            }
        }
        // Make the lengths add up to `total`: extra goes to the last filling
        // track; a shortage is taken from the end.
        let sum: usize = lens.iter().sum();
        if sum < total {
            let i = flex.last().copied().unwrap_or(tracks.len() - 1);
            lens[i] += total - sum;
        } else if sum > total {
            let mut excess = sum - total;
            let order: Vec<usize> = flex.iter().rev().copied().chain((0..tracks.len()).rev()).collect();
            for i in order {
                let take = excess.min(lens[i]);
                lens[i] -= take;
                excess -= take;
                if excess == 0 {
                    break;
                }
            }
        }
        lens
    }
}

/// A condition on the terminal size for a layout variant:
/// `width < 80`, `height >= 20 and width < 100`.
#[derive(Debug, Clone, PartialEq)]
pub struct SizeCond(Vec<(bool, char, String, usize)>);

impl SizeCond {
    pub fn parse(s: &str) -> Result<SizeCond, String> {
        let words: Vec<&str> = s.split_whitespace().collect();
        let mut terms = Vec::new();
        for chunk in words.split(|w| *w == "and") {
            let [dim, op, n] = chunk else {
                return Err(format!("bad size condition `{s}` (expected e.g. `width < 80`)"));
            };
            let is_width = match *dim {
                "width" => true,
                "height" => false,
                _ => return Err(format!("unknown dimension `{dim}` (use width or height)")),
            };
            if !matches!(*op, "<" | "<=" | ">" | ">=" | "==") {
                return Err(format!("unknown operator `{op}`"));
            }
            let n: usize = n.parse().map_err(|_| format!("`{n}` is not a number"))?;
            terms.push((is_width, op.chars().next().unwrap(), op.to_string(), n));
        }
        Ok(SizeCond(terms))
    }

    pub fn eval(&self, width: usize, height: usize) -> bool {
        self.0.iter().all(|(is_width, _, op, n)| {
            let v = if *is_width { width } else { height };
            match op.as_str() {
                "<" => v < *n,
                "<=" => v <= *n,
                ">" => v > *n,
                ">=" => v >= *n,
                _ => v == *n,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    /// A drawing with the written sizes of its areas.
    fn sized(lines: &[&str], list: &[(char, Option<usize>, Option<usize>)]) -> Grid {
        let mut g = analyze(&d(lines)).unwrap();
        let dims = list.iter().map(|(c, w, h)| (*c, (w.map(Dim::Cells), h.map(Dim::Cells)))).collect();
        g.set_sizes(&dims).unwrap();
        g
    }

    fn track(size: Option<Dim>, weight: usize) -> Track {
        Track { size, weight }
    }

    #[test]
    fn spec_example_tracks() {
        let g = analyze(&d(&["sssmm", "sssmm", "lllll"])).unwrap();
        assert_eq!(g.areas[&'s'], Rect::new(0, 0, 1, 1));
        assert_eq!(g.areas[&'m'], Rect::new(1, 0, 1, 1));
        assert_eq!(g.areas[&'l'], Rect::new(0, 1, 2, 1));
        assert_eq!(g.columns, [track(None, 3), track(None, 2)]);
        assert_eq!(g.rows, [track(None, 2), track(None, 1)]);
    }

    #[test]
    fn grid_form_has_a_track_per_cell() {
        let g = analyze_grid(&d(&["AA", "BB"])).unwrap();
        assert_eq!(g.columns.len(), 2, "no area edge between the columns, still two tracks");
        assert_eq!(g.areas[&'A'], Rect::new(0, 0, 2, 1));
    }

    #[test]
    fn non_rectangle_rejected() {
        let e = analyze(&d(&["aab", "abb"])).unwrap_err();
        assert!(e.contains("not a rectangle"), "{e}");
        let e = analyze(&d(&["aba"])).unwrap_err();
        assert!(e.contains("`a` is not a rectangle"), "{e}");
    }

    #[test]
    fn pinwheel_needs_no_straight_cuts() {
        // Four areas around a centre: no straight cut separates them.
        let g = analyze(&d(&["aab", "dcb", "dee"])).unwrap();
        let r = g.solve(&BTreeMap::new(), 30, 9);
        assert_eq!(r.rects[&'c'], Rect::new(10, 3, 10, 3));
        assert_eq!(r.rects[&'a'], Rect::new(0, 0, 20, 3));
        assert_eq!(r.rects[&'b'], Rect::new(20, 0, 10, 6));
    }

    #[test]
    fn ragged_or_spaced_drawings_rejected() {
        assert!(analyze(&d(&["aa", "a"])).unwrap_err().contains("same width"));
        assert!(analyze(&d(&["a b"])).unwrap_err().contains("whitespace"));
        assert!(analyze(&[]).is_err());
    }

    #[test]
    fn solve_spec_example() {
        let g = sized(&["sssmm", "sssmm", "lllll"], &[('s', Some(30), None), ('l', None, Some(6))]);
        let r = g.solve(&BTreeMap::new(), 100, 30);
        assert!(!r.too_small);
        assert_eq!(r.rects[&'s'], Rect::new(0, 0, 30, 24));
        assert_eq!(r.rects[&'m'], Rect::new(30, 0, 70, 24));
        assert_eq!(r.rects[&'l'], Rect::new(0, 24, 100, 6));
    }

    #[test]
    fn flexible_areas_share_by_drawing_proportion() {
        let g = analyze(&d(&["aaab"])).unwrap();
        let r = g.solve(&BTreeMap::new(), 80, 10);
        assert_eq!(r.rects[&'a'].w, 60);
        assert_eq!(r.rects[&'b'].w, 20);
        assert_eq!(r.rects[&'a'].h, 10);
    }

    #[test]
    fn rounding_remainder_goes_to_last_flexible() {
        let g = analyze(&d(&["abc"])).unwrap();
        let r = g.solve(&BTreeMap::new(), 10, 1);
        let ws: Vec<usize> = r.rects.values().map(|r| r.w).collect();
        assert_eq!(ws, [3, 3, 4]);
    }

    #[test]
    fn all_fixed_extra_space_goes_to_last() {
        let g = sized(&["ab"], &[('a', Some(10), None), ('b', Some(10), None)]);
        let r = g.solve(&BTreeMap::new(), 25, 1);
        assert_eq!(r.rects[&'b'].w, 15);
    }

    #[test]
    fn content_minimum_is_respected() {
        let g = analyze(&d(&["aaaab"])).unwrap();
        let mut s = BTreeMap::new();
        s.insert('b', AreaSize { min_width: 30, ..Default::default() });
        let r = g.solve(&s, 50, 5);
        assert_eq!(r.rects[&'b'].w, 30);
        assert_eq!(r.rects[&'a'].w, 20);
        assert_eq!(r.min_width, 34);
    }

    #[test]
    fn too_small_terminal_flagged_and_clipped() {
        let g = sized(&["sssmm", "lllll"], &[('s', Some(30), None), ('l', None, Some(6))]);
        let r = g.solve(&BTreeMap::new(), 20, 5);
        assert!(r.too_small);
        assert_eq!(r.min_width, 32);
        assert_eq!(r.min_height, 7);
        let total: usize = [r.rects[&'s'].w, r.rects[&'m'].w].iter().sum();
        assert_eq!(total, 20);
    }

    #[test]
    fn dims_parse_and_resolve() {
        assert_eq!(Dim::parse("20"), Ok(Some(Dim::Cells(20))));
        assert_eq!(Dim::parse("25%"), Ok(Some(Dim::Percent(25))));
        assert_eq!(Dim::parse("*"), Ok(None));
        assert!(Dim::parse("0").is_err());
        assert!(Dim::parse("150%").is_err());
        assert!(Dim::parse("wide").is_err());
        assert_eq!(Dim::parse_pair("20x10"), Ok((Some(Dim::Cells(20)), Some(Dim::Cells(10)))));
        assert_eq!(Dim::parse_pair("24x*"), Ok((Some(Dim::Cells(24)), None)));
        assert!(Dim::parse_pair("24").is_err());
        assert_eq!(Dim::Percent(25).resolve(120), 30);
    }

    #[test]
    fn conflicting_sizes_in_one_column() {
        // `s` and `i` are stacked in the same column
        let g = analyze(&d(&["mms", "mmi", "hhh"])).unwrap();
        let mut dims = Dims::new();
        dims.insert('s', (Some(Dim::Cells(20)), None));
        dims.insert('i', (Some(Dim::Cells(20)), Some(Dim::Cells(10))));
        assert!(g.clone().set_sizes(&dims).is_ok());
        dims.insert('i', (Some(Dim::Cells(25)), None));
        let e = g.clone().set_sizes(&dims).unwrap_err();
        assert!(e.contains("conflicting widths for areas `i` and `s`"), "{e}");
        dims.insert('i', (Some(Dim::Percent(25)), None));
        assert!(g.clone().set_sizes(&dims).unwrap_err().contains("mix cells and percentages"));
    }

    #[test]
    fn a_size_over_several_tracks_fills_the_free_ones() {
        // `t` is 40 wide over both columns; `a` fixes the first to 10
        let g = sized(&["ab", "tt"], &[('a', Some(10), None), ('t', Some(40), None)]);
        assert_eq!(g.columns, [track(Some(Dim::Cells(10)), 1), track(Some(Dim::Cells(30)), 1)]);
        // with no other sizes it is shared by drawing extent
        let g = sized(&["aaab", "tttt"], &[('t', Some(40), None)]);
        assert_eq!(g.columns, [track(Some(Dim::Cells(30)), 3), track(Some(Dim::Cells(10)), 1)]);
    }

    #[test]
    fn edges_that_line_up_in_the_drawing_stay_lined_up() {
        // `b` needs 30 columns: the column grows, and `d` under it too
        let g = analyze(&d(&["ab", "cd"])).unwrap();
        let mut s = BTreeMap::new();
        s.insert('b', AreaSize { min_width: 30, ..Default::default() });
        let r = g.solve(&s, 40, 4);
        assert_eq!((r.rects[&'b'].x, r.rects[&'d'].x), (10, 10));
        // an area over a sized and a filling column ends where they end
        let mut g = analyze_grid(&d(&["AB", "CC"])).unwrap();
        g.columns[0].size = Some(Dim::Cells(30));
        let r = g.solve(&BTreeMap::new(), 100, 4);
        assert_eq!(r.rects[&'B'], Rect::new(30, 0, 70, 2));
        assert_eq!(r.rects[&'C'], Rect::new(0, 2, 100, 2));
    }

    #[test]
    fn size_conditions() {
        let c = SizeCond::parse("width < 80").unwrap();
        assert!(c.eval(79, 10));
        assert!(!c.eval(80, 10));
        let c = SizeCond::parse("width >= 40 and height < 20").unwrap();
        assert!(c.eval(40, 19));
        assert!(!c.eval(40, 20));
        assert!(SizeCond::parse("depth < 3").is_err());
        assert!(SizeCond::parse("width ~ 3").is_err());
        assert!(SizeCond::parse("width <").is_err());
    }

    #[test]
    fn rect_contains() {
        let r = Rect::new(2, 2, 3, 1);
        assert!(r.contains(2, 2));
        assert!(r.contains(4, 2));
        assert!(!r.contains(5, 2));
        assert!(!r.contains(2, 3));
    }
}
