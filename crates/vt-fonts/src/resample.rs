//! Resampling glyphs onto another cell size.
//!
//! Each axis is reduced by merging neighbouring dot columns (or rows) into
//! groups, a dot being lit when any dot of its group is. For every glyph
//! the grouping is chosen from all candidates so that separate strokes stay
//! separate: `m` keeps three stems and `e` three bars. Ties go to a fixed
//! grouping that keeps the line-drawing centre in place, and characters
//! that join their neighbours (line drawing, large-symbol pieces) always
//! use it, so they still meet across cells.

use crate::Glyph;

/// Largest number of source dots merged into one: three when a cell loses
/// nearly half its dots (10 → 6), otherwise two.
fn max_group(to: usize) -> usize {
    if to < 8 { 3 } else { 2 }
}

/// Lines of dots as bit masks, bit 0 first.
type Lines = Vec<u16>;

pub(crate) fn glyph(g: &Glyph, from: (u8, u8), to: (u8, u8)) -> Glyph {
    let (fw, fh) = (usize::from(from.0), usize::from(from.1));
    let (tw, th) = (usize::from(to.0), usize::from(to.1));
    let joins = joins_neighbours(g.ch);
    let rows: Lines = (0..fh)
        .map(|y| g.rows.get(y).copied().unwrap_or(0))
        .collect();
    let narrowed = reduce(&rows, fw, tw, joins);
    let shortened = transpose(&reduce(&transpose(&narrowed, tw), fh, th, joins), th);
    Glyph {
        ch: g.ch,
        rows: shortened,
    }
}

/// Line drawing, scan lines and DEC Technical large-symbol pieces.
fn joins_neighbours(ch: char) -> bool {
    matches!(
        ch as u32,
        0x2500..=0x257F | 0x23A1..=0x23BD | 0x2320..=0x2321 | 0xF7E0..=0xF7FF | 0x2592
    )
}

/// Swaps rows and columns; `width` is the number of dots per line.
fn transpose(lines: &[u16], width: usize) -> Lines {
    (0..width)
        .map(|x| {
            lines.iter().enumerate().fold(
                0,
                |acc, (y, &l)| if l & 1 << x != 0 { acc | 1 << y } else { acc },
            )
        })
        .collect()
}

/// Reduces each line from `from` dots to `to`. Growing repeats dots.
fn reduce(lines: &[u16], from: usize, to: usize, joins: bool) -> Lines {
    if from == to {
        return lines.to_vec();
    }
    if to > from {
        return lines
            .iter()
            .map(|&l| {
                (0..to).fold(0, |acc, x| {
                    if l & 1 << (x * from / to) != 0 {
                        acc | 1 << x
                    } else {
                        acc
                    }
                })
            })
            .collect();
    }
    let canonical = masks(&canonical_groups(from, to));
    let chosen = if joins {
        canonical
    } else {
        let mut best = (cost(lines, &canonical, from) * 16, canonical.clone());
        let mut sizes = Vec::with_capacity(to);
        for_each_grouping(from, to, &mut sizes, &mut |sizes| {
            let m = masks(sizes);
            let score = cost(lines, &m, from) * 16 + distance(&m, &canonical);
            if score < best.0 {
                best = (score, m);
            }
        });
        best.1
    };
    lines.iter().map(|&l| apply(l, &chosen)).collect()
}

fn masks(sizes: &[usize]) -> Vec<u16> {
    let mut start = 0;
    sizes
        .iter()
        .map(|&n| {
            let m = ((1u32 << n) - 1) << start;
            start += n;
            m as u16
        })
        .collect()
}

fn apply(line: u16, masks: &[u16]) -> u16 {
    masks.iter().enumerate().fold(
        0,
        |acc, (i, &m)| if line & m != 0 { acc | 1 << i } else { acc },
    )
}

/// Separate strokes on a line.
fn runs(line: u16) -> u32 {
    (line & !(line << 1)).count_ones()
}

/// How badly a grouping damages the glyph: strokes that merge or vanish
/// on any line, dots a merge adds (a group whose dots disagree), and a
/// blank margin that fills.
fn cost(lines: &[u16], masks: &[u16], from: usize) -> u32 {
    let last_in = 1u16 << (from - 1);
    let last_out = 1u16 << (masks.len() - 1);
    let (mut total, mut any_in, mut any_out) = (0, 0u16, 0u16);
    for &l in lines {
        let out = apply(l, masks);
        total += runs(l).abs_diff(runs(out)) * 4;
        for &m in masks {
            let lit = (l & m).count_ones();
            if lit != 0 {
                total += m.count_ones() - lit;
            }
        }
        any_in |= l;
        any_out |= out;
    }
    let margin =
        |in_bit: u16, out_bit: u16| u32::from(any_in & in_bit == 0 && any_out & out_bit != 0);
    total + margin(1, 1) + margin(last_in, last_out)
}

/// Distance from the canonical grouping, as the sum of boundary offsets.
fn distance(m: &[u16], canonical: &[u16]) -> u32 {
    m.iter()
        .zip(canonical)
        .map(|(a, b)| (16 - a.leading_zeros()).abs_diff(16 - b.leading_zeros()))
        .sum()
}

/// Calls `f` with every way to split `from` dots into `to` groups of
/// 1..=[`max_group`] dots.
fn for_each_grouping(from: usize, to: usize, sizes: &mut Vec<usize>, f: &mut impl FnMut(&[usize])) {
    let groups_left = to - sizes.len();
    let remaining = from - sizes.iter().sum::<usize>();
    if groups_left == 0 {
        if remaining == 0 {
            f(sizes);
        }
        return;
    }
    let max = max_group(to);
    for n in 1..=max.min(remaining) {
        let rest = remaining - n;
        if rest < groups_left - 1 || rest > (groups_left - 1) * max {
            continue;
        }
        sizes.push(n);
        for_each_grouping(from, to, sizes, f);
        sizes.pop();
    }
}

/// The fixed grouping for the cell sizes DEC fonts use, chosen so the
/// line-drawing centre stays a single dot: column 4 of 10 becomes column 2
/// of 6, row 7 of 16 becomes row 4 of 10, and row 4 of 10 becomes row 3 of 8.
fn canonical_groups(from: usize, to: usize) -> Vec<usize> {
    match (from, to) {
        (10, 6) => vec![2, 2, 1, 2, 1, 2],
        (16, 10) => vec![2, 2, 2, 1, 1, 2, 2, 1, 2, 1],
        (10, 8) => vec![1, 2, 1, 1, 2, 1, 1, 1],
        (16, 8) => vec![2, 3, 1, 1, 3, 2, 2, 2],
        _ => (0..to)
            .map(|i| (i + 1) * from / to - i * from / to)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(rows: &[&str]) -> Glyph {
        Glyph {
            ch: 'x',
            rows: rows
                .iter()
                .map(|r| {
                    r.chars()
                        .enumerate()
                        .fold(0, |acc, (i, c)| if c == '#' { acc | 1 << i } else { acc })
                })
                .collect(),
        }
    }

    fn show(g: &Glyph, w: usize) -> Vec<String> {
        g.rows
            .iter()
            .map(|r| {
                (0..w)
                    .map(|x| if r & 1 << x != 0 { '#' } else { '.' })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn canonical_groupings_cover_the_cell() {
        for (from, to) in [(10, 6), (16, 10), (10, 8), (16, 8), (12, 7)] {
            let g = canonical_groups(from, to);
            assert_eq!((g.len(), g.iter().sum::<usize>()), (to, from));
        }
    }

    #[test]
    fn three_stems_survive_narrowing() {
        let m = parse(&[".###.##...", ".#..#..#..", ".#..#..#.."]);
        let out = glyph(&m, (10, 3), (6, 3));
        for row in &show(&out, 6)[1..] {
            let stems = row.split('.').filter(|s| !s.is_empty()).count();
            assert_eq!(stems, 3, "{row}");
        }
    }

    #[test]
    fn three_bars_survive_shortening() {
        // An `e`-like shape: bars on rows 0, 3 and 6 of 7.
        let m = parse(&[
            "#####", "#...#", "#...#", "#####", "#....", "#...#", "#####",
        ]);
        let out = glyph(&m, (5, 7), (5, 5));
        let bars = show(&out, 5)
            .iter()
            .filter(|r| r.as_str() == "#####")
            .count();
        assert_eq!(bars, 3);
    }
}
