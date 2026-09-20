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
    ) -> Result<Layout, String> {
        let count = top + right + bottom + left;
        if count == 0 {
            return Err("layout has zero LEDs".into());
        }
        if count > 490 {
            return Err(format!("{count} LEDs exceed one DRGB packet (490)"));
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
    rgb.chunks_exact(3)
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
