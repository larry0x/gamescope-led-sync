//! Ambient LED sync for gamescope sessions.
//!
//! Reads the gamescope compositor's PipeWire video stream on the same
//! machine, computes one color per LED around the screen edges, and
//! streams them to a WLED controller as DRGB realtime packets over
//! UDP. The HDMI signal is never touched.
//!
//! Functionally identical to the Python implementation in ../python,
//! with one difference: this client speaks gamescope's custom
//! `requested_size` negotiation, so gamescope downscales the capture
//! on the GPU and ships small frames.

mod capture;
mod cli;
mod geometry;
mod log;
mod output;

use std::{process::ExitCode, thread::sleep, time::Duration};

const RECONNECT_DELAY: Duration = Duration::from_secs(2);

fn main() -> ExitCode {
    let cfg = cli::parse();
    pipewire::init();
    capture::install_signal_handler();

    let l = &cfg.layout;
    log::info(format!(
        "layout: top {}, right {}, bottom {}, left {} = {} LEDs, start {}, {}",
        l.top(),
        l.right(),
        l.bottom(),
        l.left(),
        l.count(),
        l.start().name(),
        if l.clockwise() {
            "clockwise"
        } else {
            "counterclockwise"
        },
    ));
    if cfg.wled.is_none() {
        log::info("no --wled given: dry run, nothing is sent");
    }

    loop {
        match capture::run(&cfg) {
            Ok(capture::Reason::OnceDone) => return ExitCode::SUCCESS,
            Ok(capture::Reason::Interrupted) => {
                log::info("interrupted; disconnected cleanly");
                return ExitCode::SUCCESS;
            },
            Ok(capture::Reason::OnceFailed(msg)) => {
                log::warn(msg);
                return ExitCode::FAILURE;
            },
            Ok(capture::Reason::Stopped(msg)) => {
                if cfg.once {
                    log::warn(msg);
                    return ExitCode::FAILURE;
                }
                log::warn(format!("session ended ({msg}); reconnecting in 2 s"));
            },
            Err(msg) => {
                if cfg.once {
                    log::warn(msg);
                    return ExitCode::FAILURE;
                }
                log::warn(format!("{msg}; retrying in 2 s"));
            },
        }
        // A signal during the session or the wait must end the process,
        // not trigger another reconnect.
        if capture::interrupted() {
            return ExitCode::SUCCESS;
        }
        sleep(RECONNECT_DELAY);
    }
}
