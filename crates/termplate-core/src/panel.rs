//! Field detection in panel drawings (spec §5).
//!
//! * A horizontal run of a legend character is a one-line field; a filled
//!   rectangle of it is a multi-line field. The same character may mark
//!   several fields, which all show the same definition.
//! * `[name___]` marks a field by name (Informix-4GL style); the brackets
//!   and the fill belong to the field width. Only names in the legend count,
//!   so other brackets stay picture.
//! * Everything else is picture.

use crate::layout::Rect;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldPos {
    pub name: String,
    pub rect: Rect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Detected {
    pub fields: Vec<FieldPos>,
    /// The drawing with field cells blanked, padded to `width`.
    pub picture: Vec<Vec<char>>,
    pub width: usize,
    pub height: usize,
}

pub fn detect(drawing: &[String], names: &[&str]) -> Result<Detected, String> {
    let width = drawing.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let height = drawing.len();
    let mut grid: Vec<Vec<char>> = drawing
        .iter()
        .map(|l| {
            let mut v: Vec<char> = l.chars().collect();
            v.resize(width, ' ');
            v
        })
        .collect();
    let mut taken = vec![vec![false; width]; height];
    let mut fields = Vec::new();

    // 1. bracket fields
    for (y, row) in grid.iter().enumerate() {
        let mut x = 0;
        while x < row.len() {
            if row[x] == '['
                && let Some((name, end)) = bracket_at(row, x)
                    && names.contains(&name.as_str()) {
                        for t in taken[y][x..=end].iter_mut() {
                            *t = true;
                        }
                        fields.push(FieldPos { name, rect: Rect::new(x, y, end - x + 1, 1) });
                        x = end + 1;
                        continue;
                    }
            x += 1;
        }
    }

    // 2. single-character fields
    for name in names.iter().filter(|n| n.chars().count() == 1) {
        let ch = name.chars().next().unwrap();
        let mut seen = vec![vec![false; width]; height];
        for y in 0..height {
            for x in 0..width {
                if grid[y][x] != ch || taken[y][x] || seen[y][x] {
                    continue;
                }
                // Grow right along the row, then down while the whole span matches.
                let mut w = 0;
                while x + w < width && grid[y][x + w] == ch && !taken[y][x + w] {
                    w += 1;
                }
                let mut h = 1;
                while y + h < height && (x..x + w).all(|xx| grid[y + h][xx] == ch && !taken[y + h][xx]) {
                    h += 1;
                }
                for row in seen.iter_mut().skip(y).take(h) {
                    for s in row.iter_mut().skip(x).take(w) {
                        *s = true;
                    }
                }
                // Anything of the same char touching the rectangle makes the
                // shape non-rectangular.
                let touches = |xx: usize, yy: usize| grid[yy][xx] == ch && !taken[yy][xx] && !seen[yy][xx];
                let below = y + h < height && (x..x + w).any(|xx| touches(xx, y + h));
                let left = x > 0 && (y..y + h).any(|yy| touches(x - 1, yy));
                let right = x + w < width && (y..y + h).any(|yy| touches(x + w, yy));
                if below || left || right {
                    return Err(format!("field `{ch}` at line {} is not a rectangle", y + 1));
                }
                fields.push(FieldPos { name: name.to_string(), rect: Rect::new(x, y, w, h) });
            }
        }
    }

    for f in &fields {
        for row in taken.iter_mut().skip(f.rect.y).take(f.rect.h) {
            for t in row.iter_mut().skip(f.rect.x).take(f.rect.w) {
                *t = true;
            }
        }
    }
    for name in names {
        if !fields.iter().any(|f| f.name == *name) {
            return Err(format!("legend entry `{name}` does not appear in the drawing"));
        }
    }
    for (y, row) in grid.iter_mut().enumerate() {
        for (x, c) in row.iter_mut().enumerate() {
            if taken[y][x] {
                *c = ' ';
            }
        }
    }
    fields.sort_by_key(|f| (f.rect.y, f.rect.x));
    Ok(Detected { fields, picture: grid, width, height })
}

/// Recognizes `[name___]` starting at `row[x] == '['`. Returns the name and
/// the index of `]`. Names are alphanumeric; `_` is fill.
fn bracket_at(row: &[char], x: usize) -> Option<(String, usize)> {
    let mut i = x + 1;
    let mut name = String::new();
    while i < row.len() && row[i].is_alphanumeric() {
        name.push(row[i]);
        i += 1;
    }
    if name.is_empty() {
        return None;
    }
    while i < row.len() && row[i] == '_' {
        i += 1;
    }
    (i < row.len() && row[i] == ']').then_some((name, i))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    fn spec_panel() -> Vec<String> {
        d(&[
            "+----------------------------+",
            "| TTTTTTTTTTTTTTTTTTTTTTTTTT |",
            "| Levy:   dddddddd           |",
            "| Kuorma: llllll             |",
            "| pppppppppppppppppppppppppp |",
            "+----------------------------+",
        ])
    }

    #[test]
    fn spec_example_fields() {
        let det = detect(&spec_panel(), &["T", "d", "l", "p"]).unwrap();
        let get = |n: &str| det.fields.iter().find(|f| f.name == n).unwrap().rect;
        assert_eq!(get("T"), Rect::new(2, 1, 26, 1));
        assert_eq!(get("d"), Rect::new(10, 2, 8, 1));
        assert_eq!(get("l"), Rect::new(10, 3, 6, 1));
        assert_eq!(get("p"), Rect::new(2, 4, 26, 1));
        assert_eq!(det.width, 30);
        assert_eq!(det.height, 6);
        let row2: String = det.picture[2].iter().collect();
        assert_eq!(row2, "| Levy:                      |");
    }

    #[test]
    fn multi_line_rectangle_field() {
        let det = detect(&d(&["+----+", "|xxxx|", "|xxxx|", "+----+"]), &["x"]).unwrap();
        assert_eq!(det.fields, [FieldPos { name: "x".into(), rect: Rect::new(1, 1, 4, 2) }]);
    }

    #[test]
    fn non_rectangular_field_rejected() {
        let e = detect(&d(&["xxx", "xx "]), &["x"]).unwrap_err();
        assert!(e.contains("not a rectangle"), "{e}");
        let e = detect(&d(&["xx ", "xxx"]), &["x"]).unwrap_err();
        assert!(e.contains("not a rectangle"), "{e}");
    }

    #[test]
    fn same_char_in_separate_places_gives_two_fields() {
        let det = detect(&d(&["aa  aa"]), &["a"]).unwrap();
        assert_eq!(det.fields.len(), 2);
        assert_eq!(det.fields[1].rect, Rect::new(4, 0, 2, 1));
    }

    #[test]
    fn unused_chars_are_picture() {
        let det = detect(&d(&["Disk: dd"]), &["d"]).unwrap();
        assert_eq!(det.fields.len(), 1);
        let row: String = det.picture[0].iter().collect();
        assert_eq!(row, "Disk:   ");
    }

    #[test]
    fn letter_in_picture_word_collides_with_marker() {
        // Documented spec problem: a marker letter inside picture text becomes
        // a field too (here the `l` in "load").
        let det = detect(&d(&["load lll"]), &["l"]).unwrap();
        assert_eq!(det.fields.len(), 2);
    }

    #[test]
    fn bracket_fields() {
        let det = detect(&d(&["| Disk: [disk____] |"]), &["disk"]).unwrap();
        assert_eq!(det.fields, [FieldPos { name: "disk".into(), rect: Rect::new(8, 0, 10, 1) }]);
        let row: String = det.picture[0].iter().collect();
        assert_eq!(row, "| Disk:            |");
    }

    #[test]
    fn bracket_without_fill_and_unknown_names() {
        let det = detect(&d(&["[ab] [other] [ab__]"]), &["ab"]).unwrap();
        assert_eq!(det.fields.len(), 2);
        assert_eq!(det.fields[0].rect.w, 4);
        assert_eq!(det.fields[1].rect, Rect::new(13, 0, 6, 1));
    }

    #[test]
    fn bracket_cells_are_not_char_fields() {
        // the `d` inside [disk] must not count as a `d` field
        let det = detect(&d(&["[disk] dd"]), &["disk", "d"]).unwrap();
        assert_eq!(det.fields.len(), 2);
        assert_eq!(det.fields.iter().find(|f| f.name == "d").unwrap().rect, Rect::new(7, 0, 2, 1));
    }

    #[test]
    fn missing_legend_entry_is_error() {
        let e = detect(&d(&["abc"]), &["z"]).unwrap_err();
        assert!(e.contains("`z` does not appear"), "{e}");
    }

    #[test]
    fn ragged_lines_are_padded() {
        let det = detect(&d(&["ab", "abcd"]), &[]).unwrap();
        assert_eq!(det.width, 4);
        assert_eq!(det.picture[0], ['a', 'b', ' ', ' ']);
    }
}
