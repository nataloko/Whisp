//! Paper backgrounds behind the note text, after Whisp's paper themes:
//! dots or grid lines in the text colour at low opacity.
//!
//! Pixels are `0x00RRGGBB`, top down, as in a 32 bit DIB.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Paper {
    #[default]
    Blank,
    Dotted,
    Grid,
    LargeGrid,
}

/// Dots: radius 1 px every 20 px, at 15% opacity.
const DOT_STEP: f64 = 20.0;
const DOT_DIAMETER: f64 = 2.0;
const DOT_ALPHA: f64 = 0.15;
/// Grid lines: 1 px every 14 px (36 px for the large grid), at 10% opacity.
const GRID_STEP: f64 = 14.0;
const LARGE_GRID_STEP: f64 = 36.0;
const LINE_WIDTH: f64 = 1.0;
const LINE_ALPHA: f64 = 0.1;

impl Paper {
    pub const ALL: [Paper; 4] = [Paper::Blank, Paper::Dotted, Paper::Grid, Paper::LargeGrid];

    /// The name used in settings, as in Whisp.
    pub fn name(self) -> &'static str {
        match self {
            Paper::Blank => "blank",
            Paper::Dotted => "dotted",
            Paper::Grid => "grid",
            Paper::LargeGrid => "large_grid",
        }
    }

    /// The paper for a settings value; unknown values give blank.
    pub fn parse(name: &str) -> Paper {
        Paper::ALL
            .into_iter()
            .find(|p| p.name() == name.trim())
            .unwrap_or_default()
    }

    /// The next paper in the Alt+P cycle.
    pub fn next(self) -> Paper {
        let i = Paper::ALL.iter().position(|&p| p == self).unwrap_or(0);
        Paper::ALL[(i + 1) % Paper::ALL.len()]
    }
}

/// `ink` over `background` at opacity `alpha`.
fn blend(background: u32, ink: u32, alpha: f64) -> u32 {
    let channel = |shift: u32| {
        let b = f64::from((background >> shift) & 0xff);
        let i = f64::from((ink >> shift) & 0xff);
        ((b + (i - b) * alpha).round() as u32) << shift
    };
    channel(16) | channel(8) | channel(0)
}

/// The pattern for a `width` x `height` area at `scale` (DPI / 96).
pub fn render(
    paper: Paper,
    width: usize,
    height: usize,
    scale: f64,
    background: u32,
    ink: u32,
) -> Vec<u32> {
    let mut pixels = vec![background & 0xff_ffff; width * height];
    match paper {
        Paper::Blank => {}
        Paper::Dotted => {
            let step = DOT_STEP * scale;
            let size = (DOT_DIAMETER * scale).round().max(2.0) as usize;
            let radius = size as f64 / 2.0;
            let color = blend(background, ink, DOT_ALPHA);
            let mut cy = step / 2.0;
            while cy < height as f64 + radius {
                let mut cx = step / 2.0;
                while cx < width as f64 + radius {
                    // Centre on a pixel corner, so the dot is symmetric.
                    let (x0, y0) = (
                        cx.round() as i64 - size as i64 / 2,
                        cy.round() as i64 - size as i64 / 2,
                    );
                    for dy in 0..size as i64 {
                        for dx in 0..size as i64 {
                            let (x, y) = (x0 + dx, y0 + dy);
                            let ex = dx as f64 + 0.5 - radius;
                            let ey = dy as f64 + 0.5 - radius;
                            let inside = ex * ex + ey * ey <= radius * radius;
                            if inside
                                && x >= 0
                                && y >= 0
                                && (x as usize) < width
                                && (y as usize) < height
                            {
                                pixels[y as usize * width + x as usize] = color;
                            }
                        }
                    }
                    cx += step;
                }
                cy += step;
            }
        }
        Paper::Grid | Paper::LargeGrid => {
            let step = scale
                * if paper == Paper::Grid {
                    GRID_STEP
                } else {
                    LARGE_GRID_STEP
                };
            let line = (LINE_WIDTH * scale).round().max(1.0) as usize;
            let on_line = |limit: usize| -> Vec<bool> {
                let mut marks = vec![false; limit];
                let mut at = 0.0_f64;
                while (at as usize) < limit {
                    let start = at.round() as usize;
                    for m in marks.iter_mut().skip(start).take(line) {
                        *m = true;
                    }
                    at += step;
                }
                marks
            };
            let columns = on_line(width);
            let rows = on_line(height);
            // Where lines cross, the two 10% layers add up as in CSS.
            let one = blend(background, ink, LINE_ALPHA);
            let two = blend(
                background,
                ink,
                1.0 - (1.0 - LINE_ALPHA) * (1.0 - LINE_ALPHA),
            );
            for (y, &row) in rows.iter().enumerate() {
                for (x, &column) in columns.iter().enumerate() {
                    let p = &mut pixels[y * width + x];
                    match (row, column) {
                        (true, true) => *p = two,
                        (true, false) | (false, true) => *p = one,
                        _ => {}
                    }
                }
            }
        }
    }
    pixels
}

/// Puts the pattern wherever `pixels` shows exactly `background`, within
/// `left..right` x `top..bottom`. Text, selection and the edges of
/// antialiased glyphs keep their own pixels.
pub fn composite(
    pixels: &mut [u32],
    pattern: &[u32],
    width: usize,
    (left, top, right, bottom): (usize, usize, usize, usize),
    background: u32,
) {
    let background = background & 0xff_ffff;
    for y in top..bottom {
        let row = y * width;
        let (out, pat) = (
            &mut pixels[row + left..row + right],
            &pattern[row + left..row + right],
        );
        for (p, &q) in out.iter_mut().zip(pat) {
            if *p & 0xff_ffff == background {
                *p = q;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BG: u32 = 0x1e1e1e;
    const INK: u32 = 0xe0e0e0;

    #[test]
    fn names_parse_and_cycle() {
        for p in Paper::ALL {
            assert_eq!(Paper::parse(p.name()), p);
        }
        assert_eq!(Paper::parse("nonsense"), Paper::Blank);
        assert_eq!(Paper::parse(" grid "), Paper::Grid);
        assert_eq!(Paper::Blank.next(), Paper::Dotted);
        assert_eq!(Paper::LargeGrid.next(), Paper::Blank);
    }

    #[test]
    fn blending() {
        assert_eq!(blend(0x000000, 0xffffff, 0.1), 0x1a1a1a);
        assert_eq!(blend(BG, INK, 0.0), BG);
        assert_eq!(blend(0x102030, 0x102030, 0.5), 0x102030);
    }

    #[test]
    fn blank_is_the_background() {
        assert!(
            render(Paper::Blank, 4, 3, 1.0, BG, INK)
                .iter()
                .all(|&p| p == BG)
        );
    }

    #[test]
    fn dots_every_20_px() {
        let (w, h) = (45, 45);
        let p = render(Paper::Dotted, w, h, 1.0, BG, INK);
        let dot = blend(BG, INK, DOT_ALPHA);
        // A 2x2 dot around each tile centre (10, 10), (30, 10), ...
        for &(x, y) in &[(9, 9), (10, 10), (9, 10), (29, 9), (30, 30)] {
            assert_eq!(p[y * w + x], dot, "{x},{y}");
        }
        for &(x, y) in &[(0, 0), (8, 9), (11, 10), (20, 20)] {
            assert_eq!(p[y * w + x], BG, "{x},{y}");
        }
        assert_eq!(p.iter().filter(|&&q| q == dot).count(), 4 * 4);
        // Scaled 1.5x: dots every 30 px, 3 px wide.
        let p = render(Paper::Dotted, 60, 60, 1.5, BG, INK);
        assert_eq!(p.iter().filter(|&&q| q == dot).count(), 4 * 9);
        assert_eq!(p[15 * 60 + 15], dot);
        assert_eq!(p[13 * 60 + 13], BG);
    }

    #[test]
    fn grid_lines() {
        let (w, h) = (40, 30);
        let p = render(Paper::Grid, w, h, 1.0, BG, INK);
        let one = blend(BG, INK, LINE_ALPHA);
        let two = blend(BG, INK, 0.19);
        assert_eq!(p[0], two);
        assert_eq!(p[14], two);
        assert_eq!(p[5 * w + 14], one);
        assert_eq!(p[14 * w + 5], one);
        assert_eq!(p[5 * w + 5], BG);
        assert_eq!(p[5 * w + 15], BG);
        // Large grid: every 36 px.
        let p = render(Paper::LargeGrid, w, h, 1.0, BG, INK);
        assert_eq!(p[5 * w + 36], one);
        assert_eq!(p[5 * w + 14], BG);
        // At 150%, lines are 2 px wide every 21 px.
        let p = render(Paper::Grid, w, h, 1.5, BG, INK);
        assert_eq!(p[5 * w + 21], one);
        assert_eq!(p[5 * w + 22], one);
        assert_eq!(p[5 * w + 23], BG);
    }

    #[test]
    fn composite_keeps_everything_but_the_background() {
        let w = 3;
        let pattern = vec![0x111111; 6];
        // Alpha bytes set by GDI are ignored when comparing.
        let mut pixels = vec![BG, 0xff00_0000 | BG, 0xffffff, BG, 0x2a2a2a, BG];
        composite(&mut pixels, &pattern, w, (0, 0, 2, 2), BG);
        assert_eq!(
            pixels,
            [0x111111, 0x111111, 0xffffff, 0x111111, 0x2a2a2a, BG]
        );
    }
}
