//! Pure geometry and color math: LED layout, edge zone means, chain
//! ordering, and color shaping. No I/O, mirrors the Python
//! implementation exactly.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    Bl,
    Tl,
    Tr,
    Br,
}

impl Corner {
    pub fn parse(s: &str) -> Option<Corner> {
        match s {
            "bl" => Some(Corner::Bl),
            "tl" => Some(Corner::Tl),
            "tr" => Some(Corner::Tr),
            "br" => Some(Corner::Br),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Corner::Bl => "bl",
            Corner::Tl => "tl",
            Corner::Tr => "tr",
            Corner::Br => "br",
        }
    }
}

/// Why a Layout was rejected.
#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("layout has zero LEDs")]
    ZeroLeds,
    #[error("{count} LEDs exceed one DRGB packet (490)")]
    TooManyLeds { count: usize },
}

/// LED counts per edge plus the strip's physical orientation. A Layout
/// only exists if its total fits one DRGB packet (490 LEDs).
#[derive(Clone, Debug)]
pub struct Layout {
    top: usize,
    right: usize,
    bottom: usize,
    left: usize,
    start: Corner,
    clockwise: bool,
}

impl Layout {
    pub fn new(
        top: usize,
        right: usize,
        bottom: usize,
        left: usize,
        start: Corner,
        clockwise: bool,
    ) -> Result<Layout, LayoutError> {
        let count = top + right + bottom + left;
        if count == 0 {
            return Err(LayoutError::ZeroLeds);
        }
        if count > 490 {
            return Err(LayoutError::TooManyLeds { count });
        }
        Ok(Layout {
            top,
            right,
            bottom,
            left,
            start,
            clockwise,
        })
    }

    pub fn top(&self) -> usize {
        self.top
    }

    pub fn right(&self) -> usize {
        self.right
    }

    pub fn bottom(&self) -> usize {
        self.bottom
    }

    pub fn left(&self) -> usize {
        self.left
    }

    pub fn start(&self) -> Corner {
        self.start
    }

    pub fn clockwise(&self) -> bool {
        self.clockwise
    }

    pub fn count(&self) -> usize {
        self.top + self.right + self.bottom + self.left
    }
}

/// A borrowed BGRx frame plus the subsampling step. With gamescope's
/// requested_size in effect the frame is already small and step is 1;
/// the step covers the full-resolution fallback.
pub struct Frame<'a> {
    pub data: &'a [u8],
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub step: usize,
}

pub struct EdgeColors {
    pub top: Vec<[f32; 3]>,
    pub right: Vec<[f32; 3]>,
    pub bottom: Vec<[f32; 3]>,
    pub left: Vec<[f32; 3]>,
}

fn sampled_count(len: usize, step: usize) -> usize {
    len.div_ceil(step)
}

fn pixel(f: &Frame, sx: usize, sy: usize) -> [f32; 3] {
    // BGRx byte order in memory; returned as RGB.
    let i = sy * f.step * f.stride + sx * f.step * 4;
    [f.data[i + 2] as f32, f.data[i + 1] as f32, f.data[i] as f32]
}

fn zones_x(f: &Frame, rows: Range<usize>, n: usize, sw: usize) -> Vec<[f32; 3]> {
    let mut out = Vec::with_capacity(n);
    for z in 0..n {
        let x0 = z * sw / n;
        let x1 = ((z + 1) * sw / n).max(x0 + 1);
        let (mut acc, mut cnt) = ([0f32; 3], 0f32);
        for sy in rows.clone() {
            for sx in x0..x1 {
                let p = pixel(f, sx, sy);
                acc[0] += p[0];
                acc[1] += p[1];
                acc[2] += p[2];
                cnt += 1.0;
            }
        }
        out.push([acc[0] / cnt, acc[1] / cnt, acc[2] / cnt]);
    }
    out
}

fn zones_y(f: &Frame, cols: Range<usize>, n: usize, sh: usize) -> Vec<[f32; 3]> {
    let mut out = Vec::with_capacity(n);
    for z in 0..n {
        let y0 = z * sh / n;
        let y1 = ((z + 1) * sh / n).max(y0 + 1);
        let (mut acc, mut cnt) = ([0f32; 3], 0f32);
        for sy in y0..y1 {
            for sx in cols.clone() {
                let p = pixel(f, sx, sy);
                acc[0] += p[0];
                acc[1] += p[1];
                acc[2] += p[2];
                cnt += 1.0;
            }
        }
        out.push([acc[0] / cnt, acc[1] / cnt, acc[2] / cnt]);
    }
    out
}

/// Per-edge zone colors in canonical reading order: top and bottom run
/// left to right, left and right run top to bottom.
pub fn edge_means(f: &Frame, band_frac: f64, l: &Layout) -> EdgeColors {
    let sw = sampled_count(f.width, f.step);
    let sh = sampled_count(f.height, f.step);
    let band = ((band_frac * sh as f64).round() as usize).clamp(1, sh);
    let band_w = band.min(sw);
    EdgeColors {
        top: zones_x(f, 0..band, l.top, sw),
        bottom: zones_x(f, sh - band..sh, l.bottom, sw),
        left: zones_y(f, 0..band_w, l.left, sh),
        right: zones_y(f, sw - band_w..sw, l.right, sh),
    }
}

#[derive(Clone, Copy)]
enum Edge {
    Top,
    Right,
    Bottom,
    Left,
}

/// Order edge zones along the physical strip, first LED first.
///
/// Base sequence is clockwise as seen from the front, anchored at the
/// bottom-left corner: left edge upward, top left-to-right, right edge
/// downward, bottom right-to-left. flip=true walks an edge against its
/// canonical reading order.
pub fn chain(e: &EdgeColors, l: &Layout) -> Vec<[f32; 3]> {
    let base = [
        (Edge::Left, true),
        (Edge::Top, false),
        (Edge::Right, false),
        (Edge::Bottom, true),
    ];
    let offset = match l.start() {
        Corner::Bl => 0,
        Corner::Tl => 1,
        Corner::Tr => 2,
        Corner::Br => 3,
    };
    let rotated: Vec<(Edge, bool)> = (0..4).map(|i| base[(offset + i) % 4]).collect();
    let seq: Vec<(Edge, bool)> = if l.clockwise() {
        rotated
    } else {
        rotated
            .into_iter()
            .rev()
            .map(|(edge, flip)| (edge, !flip))
            .collect()
    };

    let mut out = Vec::with_capacity(l.count());
    for (edge, flip) in seq {
        let v = match edge {
            Edge::Top => &e.top,
            Edge::Right => &e.right,
            Edge::Bottom => &e.bottom,
            Edge::Left => &e.left,
        };
        if flip {
            out.extend(v.iter().rev().copied());
        } else {
            out.extend(v.iter().copied());
        }
    }
    out
}

/// The color grading, on one pixel: saturation gain then gamma, both
/// no-ops at 1.0. The saturation compensates the muted look of
/// gamescope's HDR-to-SDR capture; the gamma lifts its crushed
/// shadows. Kept in one place so the LED output and every diagnostic
/// (dump, preview, log) grade identically.
fn grade_one(mut r: f32, mut g: f32, mut b: f32, saturation: f32, gamma: f32) -> [f32; 3] {
    if saturation != 1.0 {
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        r = y + (r - y) * saturation;
        g = y + (g - y) * saturation;
        b = y + (b - y) * saturation;
    }
    r = r.clamp(0.0, 255.0);
    g = g.clamp(0.0, 255.0);
    b = b.clamp(0.0, 255.0);
    if gamma != 1.0 {
        r = 255.0 * (r / 255.0).powf(gamma);
        g = 255.0 * (g / 255.0).powf(gamma);
        b = 255.0 * (b / 255.0).powf(gamma);
    }
    [r, g, b]
}

fn to_u8(c: [f32; 3]) -> [u8; 3] {
    [(c[0] + 0.5) as u8, (c[1] + 0.5) as u8, (c[2] + 0.5) as u8]
}

/// Grade a color for display, float 0..255 in, u8 out.
pub fn grade_color(c: [f32; 3], saturation: f32, gamma: f32) -> [u8; 3] {
    to_u8(grade_one(c[0], c[1], c[2], saturation, gamma))
}

/// The per-LED colors as bytes for the wire.
pub fn shape_colors(vals: &[[f32; 3]], saturation: f32, gamma: f32) -> Vec<[u8; 3]> {
    vals.iter()
        .map(|&c| grade_color(c, saturation, gamma))
        .collect()
}

/// Grade a packed RGB frame in place-equivalent, for the dump: it lets
/// a saved frame preview what the knobs do, at full spatial detail.
pub fn grade_rgb8(rgb: &[u8], saturation: f32, gamma: f32) -> Vec<u8> {
    if saturation == 1.0 && gamma == 1.0 {
        return rgb.to_vec();
    }
    rgb.as_chunks::<3>()
        .0
        .iter()
        .flat_map(|p| {
            to_u8(grade_one(
                p[0] as f32,
                p[1] as f32,
                p[2] as f32,
                saturation,
                gamma,
            ))
        })
        .collect()
}

/// The subsampled frame as packed RGB, for --dump.
pub fn sampled_rgb(f: &Frame) -> (Vec<u8>, usize, usize) {
    let sw = sampled_count(f.width, f.step);
    let sh = sampled_count(f.height, f.step);
    let mut out = Vec::with_capacity(sw * sh * 3);
    for sy in 0..sh {
        for sx in 0..sw {
            let p = pixel(f, sx, sy);
            out.push(p[0] as u8);
            out.push(p[1] as u8);
            out.push(p[2] as u8);
        }
    }
    (out, sw, sh)
}

#[cfg(test)]
mod tests {
    // The expected behavior below is fixed from the project's goal --
    // an ambient backlight where each LED shows the color of the screen
    // region physically nearest it -- and from the color-grading
    // definitions, never from this implementation. A failing assertion
    // means the code is wrong, not that the assertion should change.
    use super::*;

    // Build a BGRx frame (the capture's pixel format) whose screen
    // color at (x, y) is given by `rgb`, so tests state colors in RGB
    // and this encodes them the way the capture delivers them.
    fn frame(w: usize, h: usize, rgb: impl Fn(usize, usize) -> [u8; 3]) -> Vec<u8> {
        let mut data = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let [r, g, b] = rgb(x, y);
                let i = (y * w + x) * 4;
                data[i] = b;
                data[i + 1] = g;
                data[i + 2] = r;
            }
        }
        data
    }

    // ---- Corner ---------------------------------------------------------

    #[test]
    fn corner_parses_the_four_codes() {
        assert_eq!(Corner::parse("bl"), Some(Corner::Bl));
        assert_eq!(Corner::parse("tl"), Some(Corner::Tl));
        assert_eq!(Corner::parse("tr"), Some(Corner::Tr));
        assert_eq!(Corner::parse("br"), Some(Corner::Br));
    }

    #[test]
    fn corner_rejects_anything_else() {
        for s in ["", "BL", "Bl", "b", "bottom-left", "bl ", "x"] {
            assert_eq!(Corner::parse(s), None, "{s:?} should not parse");
        }
    }

    // ---- Layout ---------------------------------------------------------

    #[test]
    fn layout_keeps_its_edges_and_sums_the_count() {
        let l = Layout::new(132, 75, 132, 75, Corner::Bl, true).unwrap();
        assert_eq!(
            (l.top(), l.right(), l.bottom(), l.left()),
            (132, 75, 132, 75)
        );
        assert_eq!(l.count(), 414);
        assert_eq!(l.start(), Corner::Bl);
        assert!(l.clockwise());
    }

    #[test]
    fn layout_accepts_up_to_one_packet_and_rejects_beyond() {
        // 490 LEDs is the most a single DRGB packet carries.
        assert!(Layout::new(490, 0, 0, 0, Corner::Bl, true).is_ok());
        assert!(Layout::new(123, 122, 123, 122, Corner::Bl, true).is_ok()); // 490
        assert!(Layout::new(491, 0, 0, 0, Corner::Bl, true).is_err());
        assert!(Layout::new(123, 123, 123, 122, Corner::Bl, true).is_err()); // 491
    }

    #[test]
    fn layout_rejects_no_leds_but_allows_a_missing_edge() {
        assert!(Layout::new(0, 0, 0, 0, Corner::Bl, true).is_err());
        // A three-sided strip (no bottom) is a legitimate setup.
        assert!(Layout::new(10, 5, 0, 5, Corner::Bl, true).is_ok());
    }

    // ---- edge_means -----------------------------------------------------

    #[test]
    fn edge_means_of_a_solid_frame_is_that_color_everywhere() {
        // Every zone of every edge must report the one screen color,
        // which also proves the BGRx bytes are read back as RGB.
        let data = frame(40, 40, |_, _| [10, 20, 30]);
        let f = Frame {
            data: &data,
            width: 40,
            height: 40,
            stride: 160,
            step: 1,
        };
        let l = Layout::new(4, 3, 4, 3, Corner::Bl, true).unwrap();
        let e = edge_means(&f, 0.1, &l);
        for zones in [&e.top, &e.right, &e.bottom, &e.left] {
            for c in zones {
                assert_eq!(c.map(|v| v as u8), [10, 20, 30]);
            }
        }
    }

    #[test]
    fn edge_means_orders_top_and_bottom_left_to_right() {
        // Left half red, right half blue. The horizontal edges run
        // left-to-right, so zone 0 is red and zone 1 is blue; the left
        // edge sees only red, the right edge only blue.
        let (red, blue) = ([200u8, 0, 0], [0u8, 0, 200]);
        let data = frame(40, 40, |x, _| {
            if x < 20 {
                red
            } else {
                blue
            }
        });
        let f = Frame {
            data: &data,
            width: 40,
            height: 40,
            stride: 160,
            step: 1,
        };
        let l = Layout::new(2, 2, 2, 2, Corner::Bl, true).unwrap();
        let e = edge_means(&f, 0.1, &l);
        assert_eq!(e.top[0].map(|v| v as u8), red);
        assert_eq!(e.top[1].map(|v| v as u8), blue);
        assert_eq!(e.bottom[0].map(|v| v as u8), red);
        assert_eq!(e.bottom[1].map(|v| v as u8), blue);
        assert_eq!(e.left[0].map(|v| v as u8), red);
        assert_eq!(e.left[1].map(|v| v as u8), red);
        assert_eq!(e.right[0].map(|v| v as u8), blue);
        assert_eq!(e.right[1].map(|v| v as u8), blue);
    }

    #[test]
    fn edge_means_orders_left_and_right_top_to_bottom() {
        // Top half green, bottom half red. The vertical edges run
        // top-to-bottom, so zone 0 is green and zone 1 is red.
        let (green, red) = ([0u8, 200, 0], [200u8, 0, 0]);
        let data = frame(40, 40, |_, y| {
            if y < 20 {
                green
            } else {
                red
            }
        });
        let f = Frame {
            data: &data,
            width: 40,
            height: 40,
            stride: 160,
            step: 1,
        };
        let l = Layout::new(2, 2, 2, 2, Corner::Bl, true).unwrap();
        let e = edge_means(&f, 0.1, &l);
        assert_eq!(e.left[0].map(|v| v as u8), green);
        assert_eq!(e.left[1].map(|v| v as u8), red);
        assert_eq!(e.right[0].map(|v| v as u8), green);
        assert_eq!(e.right[1].map(|v| v as u8), red);
        assert_eq!(e.top[0].map(|v| v as u8), green);
        assert_eq!(e.top[1].map(|v| v as u8), green);
        assert_eq!(e.bottom[0].map(|v| v as u8), red);
        assert_eq!(e.bottom[1].map(|v| v as u8), red);
    }

    // ---- chain ----------------------------------------------------------

    // Two zones per edge, each tagged [edge_id, position, 0] so the
    // chained order can be read back. Edge ids: top 10, right 20,
    // bottom 30, left 40; position is the canonical index (top/bottom
    // left-to-right, left/right top-to-bottom).
    fn tagged_edges() -> EdgeColors {
        let e = |id: f32| vec![[id, 0.0, 0.0], [id, 1.0, 0.0]];
        EdgeColors {
            top: e(10.0),
            right: e(20.0),
            bottom: e(30.0),
            left: e(40.0),
        }
    }

    fn tags(chained: &[[f32; 3]]) -> Vec<(u8, u8)> {
        chained.iter().map(|c| (c[0] as u8, c[1] as u8)).collect()
    }

    #[test]
    fn chain_clockwise_from_bottom_left() {
        // Clockwise from bottom-left, seen from the front: up the left
        // edge, across the top, down the right, back across the bottom.
        let l = Layout::new(2, 2, 2, 2, Corner::Bl, true).unwrap();
        let got = tags(&chain(&tagged_edges(), &l));
        assert_eq!(
            got,
            vec![
                (40, 1),
                (40, 0), // left edge, bottom to top
                (10, 0),
                (10, 1), // top edge, left to right
                (20, 0),
                (20, 1), // right edge, top to bottom
                (30, 1),
                (30, 0), // bottom edge, right to left
            ],
        );
    }

    #[test]
    fn chain_counterclockwise_from_bottom_left() {
        // Counterclockwise from bottom-left: along the bottom, up the
        // right, across the top, down the left.
        let l = Layout::new(2, 2, 2, 2, Corner::Bl, false).unwrap();
        let got = tags(&chain(&tagged_edges(), &l));
        assert_eq!(
            got,
            vec![
                (30, 0),
                (30, 1), // bottom edge, left to right
                (20, 1),
                (20, 0), // right edge, bottom to top
                (10, 1),
                (10, 0), // top edge, right to left
                (40, 0),
                (40, 1), // left edge, top to bottom
            ],
        );
    }

    #[test]
    fn chain_clockwise_from_top_left() {
        // Clockwise from top-left: across the top, down the right,
        // across the bottom, up the left.
        let l = Layout::new(2, 2, 2, 2, Corner::Tl, true).unwrap();
        let got = tags(&chain(&tagged_edges(), &l));
        assert_eq!(
            got,
            vec![
                (10, 0),
                (10, 1), // top edge, left to right
                (20, 0),
                (20, 1), // right edge, top to bottom
                (30, 1),
                (30, 0), // bottom edge, right to left
                (40, 1),
                (40, 0), // left edge, bottom to top
            ],
        );
    }

    #[test]
    fn chain_length_matches_the_led_count() {
        let l = Layout::new(4, 3, 4, 3, Corner::Br, true).unwrap();
        let e = EdgeColors {
            top: vec![[0.0; 3]; 4],
            right: vec![[0.0; 3]; 3],
            bottom: vec![[0.0; 3]; 4],
            left: vec![[0.0; 3]; 3],
        };
        assert_eq!(chain(&e, &l).len(), l.count());
    }

    // ---- color grading --------------------------------------------------

    #[test]
    fn grading_is_identity_at_the_defaults() {
        assert_eq!(
            shape_colors(&[[10.0, 20.0, 30.0]], 1.0, 1.0),
            vec![[10, 20, 30]]
        );
        assert_eq!(grade_color([200.0, 100.0, 50.0], 1.0, 1.0), [200, 100, 50]);
    }

    #[test]
    fn saturation_leaves_a_gray_unchanged() {
        // A gray has every channel equal to its own luma, so no
        // saturation gain can move it, whatever the luma weights are.
        assert_eq!(
            grade_color([100.0, 100.0, 100.0], 0.0, 1.0),
            [100, 100, 100]
        );
        assert_eq!(
            grade_color([100.0, 100.0, 100.0], 5.0, 1.0),
            [100, 100, 100]
        );
    }

    #[test]
    fn zero_saturation_makes_gray() {
        // Fully desaturated, every channel collapses to the shared luma.
        let [r, g, b] = grade_color([200.0, 100.0, 50.0], 0.0, 1.0);
        assert_eq!((r, g), (g, b));
    }

    #[test]
    fn saturation_above_one_widens_the_spread() {
        // A channel above the luma rises, one below it falls.
        let [r, _g, b] = grade_color([200.0, 100.0, 60.0], 1.5, 1.0);
        assert!(r > 200, "channel above luma should rise, got {r}");
        assert!(b < 60, "channel below luma should fall, got {b}");
    }

    #[test]
    fn gamma_below_one_brightens_and_above_one_darkens() {
        // out = 255 * (v/255)^gamma. For v = 100: gamma 0.5 -> 160,
        // gamma 2.0 -> 39. Computed from the definition, not the code.
        assert_eq!(
            grade_color([100.0, 100.0, 100.0], 1.0, 0.5),
            [160, 160, 160]
        );
        assert_eq!(grade_color([100.0, 100.0, 100.0], 1.0, 2.0), [39, 39, 39]);
    }

    #[test]
    fn grading_clamps_out_of_range_channels() {
        // Saturation drives red past 255 and the others below 0; the
        // result must clamp, never wrap.
        assert_eq!(grade_color([250.0, 0.0, 0.0], 2.0, 1.0), [255, 0, 0]);
    }

    #[test]
    fn grade_rgb8_passes_through_at_the_defaults() {
        assert_eq!(
            grade_rgb8(&[1, 2, 3, 4, 5, 6], 1.0, 1.0),
            vec![1, 2, 3, 4, 5, 6]
        );
    }

    #[test]
    fn grade_rgb8_grades_every_pixel() {
        // Two gray pixels under gamma 0.5: 100 -> 160, 50 -> 113.
        assert_eq!(
            grade_rgb8(&[100, 100, 100, 50, 50, 50], 1.0, 0.5),
            vec![160, 160, 160, 113, 113, 113],
        );
    }
}
