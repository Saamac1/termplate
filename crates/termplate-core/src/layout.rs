//! Layout drawings (spec §4).
//!
//! Each character of the drawing names an area. Areas must be rectangles and
//! the drawing must be divisible by straight cuts that run across the whole
//! remaining region (a guillotine partition). Sizes: `size.width N` /
//! `size.height N` fix a dimension in terminal cells; the other areas share
//! what is left in proportion to their extent in the drawing, never going
//! below their minimum size.

use std::collections::BTreeMap;

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
    /// Children side by side (cut by vertical lines).
    Columns,
    /// Children stacked (cut by horizontal lines).
    Rows,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Leaf(char),
    Split(Axis, Vec<Node>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Region in drawing cells.
    pub region: Rect,
    pub kind: NodeKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub areas: BTreeMap<char, Rect>,
    pub tree: Node,
    pub width: usize,
    pub height: usize,
}

pub fn analyze(drawing: &[String]) -> Result<Grid, String> {
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

    let mut areas: BTreeMap<char, Rect> = BTreeMap::new();
    let mut counts: BTreeMap<char, usize> = BTreeMap::new();
    for (y, row) in rows.iter().enumerate() {
        for (x, &c) in row.iter().enumerate() {
            *counts.entry(c).or_default() += 1;
            let r = areas.entry(c).or_insert(Rect::new(x, y, 1, 1));
            let x0 = r.x.min(x);
            let y0 = r.y.min(y);
            let x1 = (r.x + r.w).max(x + 1);
            let y1 = (r.y + r.h).max(y + 1);
            *r = Rect::new(x0, y0, x1 - x0, y1 - y0);
        }
    }
    for (c, r) in &areas {
        if counts[c] != r.w * r.h {
            return Err(format!("area `{c}` is not a rectangle"));
        }
    }
    let tree = split(Rect::new(0, 0, width, height), &areas)?;
    Ok(Grid { areas, tree, width, height })
}

fn split(region: Rect, areas: &BTreeMap<char, Rect>) -> Result<Node, String> {
    let inside: Vec<(char, Rect)> = areas
        .iter()
        .filter(|(_, r)| r.x >= region.x && r.y >= region.y && r.x + r.w <= region.x + region.w && r.y + r.h <= region.y + region.h)
        .map(|(c, r)| (*c, *r))
        .collect();
    if inside.len() == 1 {
        return Ok(Node { region, kind: NodeKind::Leaf(inside[0].0) });
    }
    for axis in [Axis::Columns, Axis::Rows] {
        let (start, len) = match axis {
            Axis::Columns => (region.x, region.w),
            Axis::Rows => (region.y, region.h),
        };
        let cuts: Vec<usize> = (start + 1..start + len)
            .filter(|&cut| {
                inside.iter().all(|(_, r)| {
                    let (a, l) = match axis {
                        Axis::Columns => (r.x, r.w),
                        Axis::Rows => (r.y, r.h),
                    };
                    !(a < cut && cut < a + l)
                })
            })
            .collect();
        if cuts.is_empty() {
            continue;
        }
        let mut bounds = vec![start];
        bounds.extend(cuts);
        bounds.push(start + len);
        let children = bounds
            .windows(2)
            .map(|w| {
                let sub = match axis {
                    Axis::Columns => Rect::new(w[0], region.y, w[1] - w[0], region.h),
                    Axis::Rows => Rect::new(region.x, w[0], region.w, w[1] - w[0]),
                };
                split(sub, areas)
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(Node { region, kind: NodeKind::Split(axis, children) });
    }
    Err("layout cannot be divided into its areas with straight lines".into())
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

/// Written sizes of each area: (width, height).
pub type Dims = BTreeMap<char, (Option<Dim>, Option<Dim>)>;

impl Grid {
    /// Conflicting sizes in the same column or row are an error (D 11.1):
    /// areas stacked on top of each other must not ask for different
    /// widths, and areas side by side must not ask for different heights.
    pub fn check_conflicts(&self, dims: &Dims) -> Result<(), String> {
        check_node(&self.tree, dims)
    }
}

fn dim_of(node: &Node, axis: Axis, dims: &Dims) -> Option<Dim> {
    match &node.kind {
        NodeKind::Leaf(c) => dims.get(c).and_then(|d| if axis == Axis::Columns { d.0 } else { d.1 }),
        NodeKind::Split(a, children) if *a == axis => {
            let mut sum = 0;
            for c in children {
                match dim_of(c, axis, dims)? {
                    Dim::Cells(n) => sum += n,
                    Dim::Percent(_) => return None,
                }
            }
            Some(Dim::Cells(sum))
        }
        NodeKind::Split(_, _) => None,
    }
}

fn check_node(node: &Node, dims: &Dims) -> Result<(), String> {
    let NodeKind::Split(axis, children) = &node.kind else { return Ok(()) };
    let cross = match axis {
        Axis::Columns => Axis::Rows,
        Axis::Rows => Axis::Columns,
    };
    let given: Vec<(char, Dim)> = children
        .iter()
        .filter_map(|c| dim_of(c, cross, dims).map(|d| (first_leaf(c), d)))
        .collect();
    if let Some(((a, da), (b, db))) = given.iter().zip(given.iter().skip(1)).find(|(x, y)| x.1 != y.1) {
        let what = if cross == Axis::Columns { "widths" } else { "heights" };
        return Err(format!("conflicting {what} for areas `{a}` ({da:?}) and `{b}` ({db:?})"));
    }
    children.iter().try_for_each(|c| check_node(c, dims))
}

fn first_leaf(node: &Node) -> char {
    match &node.kind {
        NodeKind::Leaf(c) => *c,
        NodeKind::Split(_, children) => first_leaf(&children[0]),
    }
}

/// Size constraints of one area, in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AreaSize {
    pub width: Option<usize>,
    pub height: Option<usize>,
    /// Minimum size of the content (e.g. a panel drawing).
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
    pub fn min_size(&self, sizes: &BTreeMap<char, AreaSize>) -> (usize, usize) {
        (min_of(&self.tree, Axis::Columns, sizes), min_of(&self.tree, Axis::Rows, sizes))
    }

    pub fn solve(&self, sizes: &BTreeMap<char, AreaSize>, width: usize, height: usize) -> Solved {
        let (min_width, min_height) = self.min_size(sizes);
        let mut rects = BTreeMap::new();
        distribute(&self.tree, Rect::new(0, 0, width, height), sizes, &mut rects);
        Solved { rects, too_small: width < min_width || height < min_height, min_width, min_height }
    }
}

fn leaf_fixed(c: char, axis: Axis, sizes: &BTreeMap<char, AreaSize>) -> Option<usize> {
    let s = sizes.get(&c)?;
    match axis {
        Axis::Columns => s.width,
        Axis::Rows => s.height,
    }
}

fn span(node: &Node, axis: Axis) -> usize {
    match axis {
        Axis::Columns => node.region.w,
        Axis::Rows => node.region.h,
    }
}

fn min_of(node: &Node, axis: Axis, sizes: &BTreeMap<char, AreaSize>) -> usize {
    match &node.kind {
        NodeKind::Leaf(c) => leaf_fixed(*c, axis, sizes).unwrap_or_else(|| {
            let content = sizes.get(c).map(|s| match axis {
                Axis::Columns => s.min_width,
                Axis::Rows => s.min_height,
            });
            content.unwrap_or(0).max(span(node, axis))
        }),
        NodeKind::Split(a, children) if *a == axis => children.iter().map(|c| min_of(c, axis, sizes)).sum(),
        NodeKind::Split(_, children) => children.iter().map(|c| min_of(c, axis, sizes)).max().unwrap_or(0),
    }
}

fn fixed_of(node: &Node, axis: Axis, sizes: &BTreeMap<char, AreaSize>) -> Option<usize> {
    match &node.kind {
        NodeKind::Leaf(c) => leaf_fixed(*c, axis, sizes),
        NodeKind::Split(a, children) if *a == axis => {
            children.iter().map(|c| fixed_of(c, axis, sizes)).sum::<Option<usize>>()
        }
        NodeKind::Split(_, children) => children.iter().filter_map(|c| fixed_of(c, axis, sizes)).max(),
    }
}

fn distribute(node: &Node, rect: Rect, sizes: &BTreeMap<char, AreaSize>, out: &mut BTreeMap<char, Rect>) {
    let (axis, children) = match &node.kind {
        NodeKind::Leaf(c) => {
            out.insert(*c, rect);
            return;
        }
        NodeKind::Split(axis, children) => (*axis, children),
    };
    let total = match axis {
        Axis::Columns => rect.w,
        Axis::Rows => rect.h,
    };
    let fixed: Vec<Option<usize>> = children.iter().map(|c| fixed_of(c, axis, sizes)).collect();
    let mut lens: Vec<usize> = fixed.iter().map(|f| f.unwrap_or(0)).collect();
    let flex: Vec<usize> = (0..children.len()).filter(|&i| fixed[i].is_none()).collect();
    let used: usize = lens.iter().sum();
    let remaining = total.saturating_sub(used);
    // Water-filling: share in proportion to drawing extent; a child whose
    // share would fall below its minimum is pinned to the minimum and the
    // others share the rest.
    let mins: Vec<usize> = children.iter().map(|c| min_of(c, axis, sizes)).collect();
    let mut pinned = vec![false; children.len()];
    loop {
        let rest = remaining.saturating_sub(flex.iter().filter(|&&i| pinned[i]).map(|&i| mins[i]).sum());
        let free: Vec<usize> = flex.iter().copied().filter(|&i| !pinned[i]).collect();
        let weight: usize = free.iter().map(|&i| span(&children[i], axis)).sum();
        let mut changed = false;
        for &i in &free {
            let share = rest * span(&children[i], axis) / weight.max(1);
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
    // Make the lengths add up to `total`: extra goes to the last flexible
    // child; a shortage is taken from the end.
    let sum: usize = lens.iter().sum();
    if sum < total {
        let i = flex.last().copied().unwrap_or(children.len() - 1);
        lens[i] += total - sum;
    } else if sum > total {
        let mut excess = sum - total;
        let order: Vec<usize> = flex.iter().rev().copied().chain((0..children.len()).rev()).collect();
        for i in order {
            let take = excess.min(lens[i]);
            lens[i] -= take;
            excess -= take;
            if excess == 0 {
                break;
            }
        }
    }
    let mut pos = 0;
    for (child, len) in children.iter().zip(lens) {
        let r = match axis {
            Axis::Columns => Rect::new(rect.x + pos, rect.y, len, rect.h),
            Axis::Rows => Rect::new(rect.x, rect.y + pos, rect.w, len),
        };
        distribute(child, r, sizes, out);
        pos += len;
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

    fn sizes(list: &[(char, Option<usize>, Option<usize>)]) -> BTreeMap<char, AreaSize> {
        list.iter().map(|(c, w, h)| (*c, AreaSize { width: *w, height: *h, ..Default::default() })).collect()
    }

    #[test]
    fn spec_example_areas() {
        let g = analyze(&d(&["sssmm", "sssmm", "lllll"])).unwrap();
        assert_eq!(g.areas[&'s'], Rect::new(0, 0, 3, 2));
        assert_eq!(g.areas[&'m'], Rect::new(3, 0, 2, 2));
        assert_eq!(g.areas[&'l'], Rect::new(0, 2, 5, 1));
        let NodeKind::Split(Axis::Rows, ch) = &g.tree.kind else { panic!("{:?}", g.tree) };
        assert_eq!(ch.len(), 2);
        assert!(matches!(&ch[0].kind, NodeKind::Split(Axis::Columns, c) if c.len() == 2));
    }

    #[test]
    fn non_rectangle_rejected() {
        let e = analyze(&d(&["aab", "abb"])).unwrap_err();
        assert!(e.contains("not a rectangle"), "{e}");
        let e = analyze(&d(&["aba"])).unwrap_err();
        assert!(e.contains("`a` is not a rectangle"), "{e}");
    }

    #[test]
    fn pinwheel_is_not_guillotine() {
        // Four areas around a centre: no straight cut separates them.
        let e = analyze(&d(&["aab", "dcb", "dee"])).unwrap_err();
        assert!(e.contains("straight lines"), "{e}");
    }

    #[test]
    fn ragged_or_spaced_drawings_rejected() {
        assert!(analyze(&d(&["aa", "a"])).unwrap_err().contains("same width"));
        assert!(analyze(&d(&["a b"])).unwrap_err().contains("whitespace"));
        assert!(analyze(&[]).is_err());
    }

    #[test]
    fn solve_spec_example() {
        let g = analyze(&d(&["sssmm", "sssmm", "lllll"])).unwrap();
        let s = sizes(&[('s', Some(30), None), ('l', None, Some(6))]);
        let r = g.solve(&s, 100, 30);
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
        let g = analyze(&d(&["ab"])).unwrap();
        let r = g.solve(&sizes(&[('a', Some(10), None), ('b', Some(10), None)]), 25, 1);
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
        let g = analyze(&d(&["sssmm", "lllll"])).unwrap();
        let s = sizes(&[('s', Some(30), None), ('l', None, Some(6))]);
        let r = g.solve(&s, 20, 5);
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
        assert!(g.check_conflicts(&dims).is_ok());
        dims.insert('i', (Some(Dim::Cells(25)), None));
        let e = g.check_conflicts(&dims).unwrap_err();
        assert!(e.contains("conflicting widths for areas `s`"), "{e}");
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
