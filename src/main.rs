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
mod output;

use std::{cell::RefCell, process::ExitCode, rc::Rc, thread::sleep, time::Duration};

const RECONNECT_DELAY: Duration = Duration::from_secs(2);

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let cfg = cli::parse();
    pipewire::init();
    capture::install_signal_handler();

    let l = &cfg.layout;
    tracing::info!(
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
    );
    // The WLED sender and the last colors sent live here, across
    // sessions, so a reconnect keeps the LEDs lit instead of dropping
    // them for the reconnect gap (see wait_and_keepalive).
    let sender = match &cfg.wled {
        Some(host) => match output::WledSender::new(host, cfg.port, cfg.timeout_s) {
            Ok(s) => Some(Rc::new(RefCell::new(s))),
            Err(e) => {
                tracing::error!("{e}");
                return ExitCode::FAILURE;
            },
        },
        None => {
            tracing::info!("no --wled given: dry run, nothing is sent");
            None
        },
    };
    let last_colors: Rc<RefCell<Option<Vec<[u8; 3]>>>> = Rc::new(RefCell::new(None));

    loop {
        match capture::run(&cfg, sender.as_ref(), &last_colors) {
            Ok(capture::Reason::OnceDone) => return ExitCode::SUCCESS,
            Ok(capture::Reason::Interrupted) => {
                tracing::info!("interrupted; disconnected cleanly");
                return ExitCode::SUCCESS;
            },
            Ok(capture::Reason::OnceFailed(msg)) => {
                tracing::warn!("{msg}");
                return ExitCode::FAILURE;
            },
            Ok(capture::Reason::Stopped(msg)) => {
                if cfg.once {
                    tracing::warn!("{msg}");
                    return ExitCode::FAILURE;
                }
                tracing::warn!("session ended ({msg}); reconnecting in 2 s");
            },
            Err(msg) => {
                if cfg.once {
                    tracing::warn!("{msg}");
                    return ExitCode::FAILURE;
                }
                tracing::warn!("{msg}; retrying in 2 s");
            },
        }
        // A signal during the session or the wait must end the process,
        // not trigger another reconnect.
        if capture::interrupted() {
            return ExitCode::SUCCESS;
        }
        wait_and_keepalive(sender.as_ref(), &last_colors);
        if capture::interrupted() {
            return ExitCode::SUCCESS;
        }
    }
}

/// Wait before reconnecting, resending the last colors about once a
/// second so WLED stays in realtime mode and does not fall back to its
/// preset during the gap.
fn wait_and_keepalive(
    sender: Option<&Rc<RefCell<output::WledSender>>>,
    last_colors: &Rc<RefCell<Option<Vec<[u8; 3]>>>>,
) {
    let step = Duration::from_secs(1);
    let mut waited = Duration::ZERO;
    while waited < RECONNECT_DELAY {
        if capture::interrupted() {
            return;
        }
        if let Some(sender) = sender
            && let Some(colors) = last_colors.borrow().as_ref()
        {
            sender.borrow_mut().send(colors);
        }
        sleep(step);
        waited += step;
    }
}
