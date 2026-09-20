//! Command-line parsing (clap derive) into the validated Config the rest
//! of the program reads.

use {
    crate::geometry::{Corner, Layout},
    clap::{CommandFactory, Parser, error::ErrorKind},
};

#[derive(Clone)]
pub struct Config {
    pub wled: Option<String>,
    pub port: u16,
    pub timeout_s: u8,
    pub fps: f64,
    pub sample_height: u32,
    pub band_frac: f64,
    pub layout: Layout,
    pub saturation: f32,
    pub gamma: f32,
    pub smooth: f32,
    pub once: bool,
    pub dump: Option<String>,
    pub preview: bool,
}

/// Ambient LED sync for gamescope sessions.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// WLED hostname or IP; omit for a dry run
    #[arg(long, value_name = "HOST")]
    wled: Option<String>,

    /// WLED UDP realtime port
    #[arg(long, default_value_t = 21324)]
    port: u16,

    /// Seconds WLED waits after the stream stops before returning to its preset
    #[arg(long = "timeout", value_name = "S", default_value_t = 2)]
    timeout_s: u8,

    /// Processing and send rate, in frames per second
    #[arg(long, default_value_t = 30.0)]
    fps: f64,

    /// Subsampled frame height for the color math
    #[arg(long, default_value_t = 240)]
    sample_height: u32,

    /// Edge depth each LED averages, as a fraction of screen height
    #[arg(long = "band", value_name = "FRAC", default_value_t = 0.08)]
    band_frac: f64,

    /// LED count, top edge
    #[arg(long, default_value_t = 132)]
    top: usize,

    /// LED count, right edge
    #[arg(long, default_value_t = 75)]
    right: usize,

    /// LED count, bottom edge
    #[arg(long, default_value_t = 132)]
    bottom: usize,

    /// LED count, left edge
    #[arg(long, default_value_t = 75)]
    left: usize,

    /// Corner holding the strip's first LED
    #[arg(long, value_parser = ["bl", "tl", "tr", "br"], default_value = "bl")]
    start: String,

    /// Strip runs counterclockwise, seen from the front
    #[arg(long)]
    counterclockwise: bool,

    /// Saturation gain; above 1 compensates the muted HDR capture
    #[arg(long, default_value_t = 1.0)]
    saturation: f32,

    /// Gamma shaping; below 1 lifts crushed shadows
    #[arg(long, default_value_t = 1.0)]
    gamma: f32,

    /// Smoothing factor per frame; 1 disables it
    #[arg(long, default_value_t = 0.5)]
    smooth: f32,

    /// Process one frame and exit
    #[arg(long)]
    once: bool,

    /// With --once: save the captured frame to this PNG
    #[arg(long, value_name = "PNG")]
    dump: Option<String>,

    /// Draw the edge colors as ANSI bars on this terminal
    #[arg(long)]
    preview: bool,
}

/// Emit a clap-styled error and exit, for validation clap cannot express
/// in an attribute.
fn bail(message: impl std::fmt::Display) -> ! {
    Cli::command()
        .error(ErrorKind::ValueValidation, message)
        .exit()
}

pub fn parse() -> Config {
    let cli = Cli::parse();

    if !cli.fps.is_finite() || cli.fps <= 0.0 {
        bail("--fps must be a positive, finite number");
    }
    if cli.sample_height == 0 || cli.sample_height > 16384 {
        bail("--sample-height must be between 1 and 16384");
    }
    if !cli.smooth.is_finite() || cli.smooth <= 0.0 {
        bail("--smooth must be a positive number (1 disables smoothing)");
    }
    if !cli.gamma.is_finite() || cli.gamma <= 0.0 {
        bail("--gamma must be a positive number");
    }
    if !cli.saturation.is_finite() || cli.saturation < 0.0 {
        bail("--saturation must be zero or a positive number");
    }

    // clap has already restricted --start to the four valid values.
    let start = Corner::parse(&cli.start).expect("clap validated --start");
    let layout = Layout::new(
        cli.top,
        cli.right,
        cli.bottom,
        cli.left,
        start,
        !cli.counterclockwise,
    )
    .unwrap_or_else(|e| bail(e));

    Config {
        wled: cli.wled,
        port: cli.port,
        timeout_s: cli.timeout_s,
        fps: cli.fps,
        sample_height: cli.sample_height,
        band_frac: cli.band_frac,
        layout,
        saturation: cli.saturation,
        gamma: cli.gamma,
        smooth: cli.smooth,
        once: cli.once,
        dump: cli.dump,
        preview: cli.preview,
    }
}
