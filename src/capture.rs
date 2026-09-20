//! PipeWire capture: node discovery, stream negotiation with
//! gamescope's custom fields, and the per-tick processing loop.

use {
    crate::{
        cli::Config,
        geometry::{Frame, chain, edge_means, grade_color, grade_rgb8, sampled_rgb, shape_colors},
        output::{Preview, WledSender, write_png},
    },
    pipewire::{
        self as pw,
        context::ContextRc,
        main_loop::MainLoopRc,
        node::{Node, NodeListener},
        properties::properties,
        spa::{
            param::{
                ParamType,
                format::{FormatProperties, MediaSubtype, MediaType},
                video::{VideoFormat, VideoInfoRaw},
            },
            pod::{ChoiceValue, Object, Pod, Property, Value, serialize::PodSerializer},
            utils::{Choice, ChoiceEnum, ChoiceFlags, Direction, Fraction, Id, Rectangle},
        },
        stream::{StreamFlags, StreamRc, StreamState},
        types::ObjectType,
    },
    std::{
        cell::{Cell, RefCell},
        io::Cursor,
        rc::Rc,
        sync::atomic::{AtomicBool, Ordering},
        time::{Duration, Instant},
    },
};

// gamescope's custom negotiation fields, from src/pipewire_gamescope.hpp
// (checked at tag 3.16.28). requested_size asks gamescope to composite
// the capture at a smaller size on the GPU; both are optional on
// gamescope's side of the parse.
const SPA_FORMAT_VIDEO_REQUESTED_SIZE: u32 = 0x70000;
const SPA_FORMAT_VIDEO_GAMESCOPE_FOCUS_APPID: u32 = 0x70001;

// Stable ABI value from spa/utils/type.h.
const SPA_TYPE_OBJECT_FORMAT: u32 = 0x40003;

const KEEPALIVE: Duration = Duration::from_secs(1);
const STATS_EVERY: Duration = Duration::from_secs(5);
const DISCOVERY_ONCE_LIMIT: Duration = Duration::from_secs(15);

// Set by the signal handler, polled by the loops. A global is enough:
// there is one process-wide shutdown, and an async-signal handler may
// only touch an atomic safely anyway.
static QUIT: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_sig: libc::c_int) {
    QUIT.store(true, Ordering::SeqCst);
}

/// Make Ctrl+C and SIGTERM flip a flag instead of killing us outright.
///
/// The loops poll it and quit the PipeWire main loop on the loop
/// thread, so the stream disconnects through its destructor exactly as
/// a normal `--once` exit does. This matters: gamescope renegotiates
/// its capture size for us, and tearing that down by having the client
/// vanish mid-frame crashes the compositor -- taking the running game,
/// which gamescope hosts, down with it.
pub fn install_signal_handler() {
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
    }
}

pub fn interrupted() -> bool {
    QUIT.load(Ordering::SeqCst)
}

pub enum Reason {
    OnceDone,
    OnceFailed(String),
    Stopped(String),
    Interrupted,
}

/// A setup failure that ends a capture session before it produces frames.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("pipewire {op}: {source}")]
    Pipewire {
        op: &'static str,
        #[source]
        source: pw::Error,
    },
    #[error("{op} timer: {source}")]
    Timer {
        op: &'static str,
        #[source]
        source: pw::spa::utils::result::Error,
    },
    #[error("building the format pod failed")]
    BuildPod,
}

/// The client's format offer.
///
/// No VideoModifier on purpose: gamescope advertises a DMA-BUF format
/// variant (with modifier) and a shared-memory variant (without), and
/// omitting the modifier selects shared memory, which maps cleanly
/// everywhere, NVIDIA included. Size is a wide range because gamescope
/// advertises its size as fixed; the custom requested_size is what
/// makes it renegotiate to a small capture.
fn format_pod(cfg: &Config) -> Vec<u8> {
    let h = cfg.sample_height;
    let w = h.saturating_mul(16) / 9; // rough aspect; gamescope aspect-fits and rounds
    let obj = Object {
        type_: SPA_TYPE_OBJECT_FORMAT,
        id: ParamType::EnumFormat.as_raw(),
        properties: vec![
            Property::new(
                FormatProperties::MediaType.as_raw(),
                Value::Id(Id(MediaType::Video.as_raw())),
            ),
            Property::new(
                FormatProperties::MediaSubtype.as_raw(),
                Value::Id(Id(MediaSubtype::Raw.as_raw())),
            ),
            Property::new(
                FormatProperties::VideoFormat.as_raw(),
                Value::Id(Id(VideoFormat::BGRx.as_raw())),
            ),
            Property::new(
                FormatProperties::VideoSize.as_raw(),
                Value::Choice(ChoiceValue::Rectangle(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: Rectangle {
                            width: w,
                            height: h,
                        },
                        min: Rectangle {
                            width: 1,
                            height: 1,
                        },
                        max: Rectangle {
                            width: 16384,
                            height: 16384,
                        },
                    },
                ))),
            ),
            Property::new(
                FormatProperties::VideoFramerate.as_raw(),
                Value::Fraction(Fraction { num: 0, denom: 1 }),
            ),
            Property::new(
                SPA_FORMAT_VIDEO_REQUESTED_SIZE,
                Value::Rectangle(Rectangle {
                    width: w,
                    height: h,
                }),
            ),
            Property::new(SPA_FORMAT_VIDEO_GAMESCOPE_FOCUS_APPID, Value::Long(0)),
        ],
    };
    PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(obj))
        .expect("pod serialization cannot fail")
        .0
        .into_inner()
}

/// Wait until gamescope's video node exists; return its global id.
///
/// Matches media.name, not node.name: NixOS wraps the binary, which
/// turns node.name into ".gamescope-wrapped". media.name stays
/// "gamescope" on every distribution.
///
/// The registry's global event only carries a node's terse properties
/// (node.name, media.class, ...); media.name is in the full node info.
/// So every candidate video node is bound and its info inspected.
fn discover(
    mainloop: &MainLoopRc,
    core: &pw::core::CoreRc,
    once: bool,
) -> Result<Option<u32>, CaptureError> {
    let registry = Rc::new(
        core.get_registry_rc()
            .map_err(|source| CaptureError::Pipewire {
                op: "registry",
                source,
            })?,
    );
    let found: Rc<Cell<Option<u32>>> = Rc::new(Cell::new(None));
    // Bound candidates with their info listeners, alive until
    // discovery ends.
    let candidates: Rc<RefCell<Vec<(Node, NodeListener)>>> = Rc::new(RefCell::new(Vec::new()));

    let _listener = registry
        .add_listener_local()
        .global({
            let registry = registry.clone();
            let found = found.clone();
            let candidates = candidates.clone();
            let ml = mainloop.clone();
            move |global| {
                if global.type_ != ObjectType::Node || found.get().is_some() {
                    return;
                }
                if let Some(props) = global.props {
                    // media.class is part of the terse properties; use
                    // it to skip audio nodes. Absence means "inspect".
                    if !props
                        .get("media.class")
                        .unwrap_or("Video")
                        .contains("Video")
                    {
                        return;
                    }
                }
                let node: Node = match registry.bind(global) {
                    Ok(node) => node,
                    Err(_) => return,
                };
                let id = global.id;
                let info_listener = node
                    .add_listener_local()
                    .info({
                        let found = found.clone();
                        let ml = ml.clone();
                        move |info| {
                            let hit = info
                                .props()
                                .is_some_and(|p| p.get("media.name") == Some("gamescope"));
                            if hit && found.get().is_none() {
                                found.set(Some(id));
                                ml.quit();
                            }
                        }
                    })
                    .register();
                candidates.borrow_mut().push((node, info_listener));
            }
        })
        .register();

    let started = Instant::now();
    let announced = Rc::new(Cell::new(false));
    let timer = mainloop.loop_().add_timer({
        let ml = mainloop.clone();
        let found = found.clone();
        let announced = announced.clone();
        move |_expirations| {
            if QUIT.load(Ordering::SeqCst) || found.get().is_some() {
                ml.quit();
                return;
            }
            if !announced.get() {
                tracing::info!("waiting for the gamescope node...");
                announced.set(true);
            }
            if once && started.elapsed() >= DISCOVERY_ONCE_LIMIT {
                ml.quit();
            }
        }
    });
    // A short tick so Ctrl+C during discovery is noticed promptly.
    timer
        .update_timer(
            Some(Duration::from_millis(500)),
            Some(Duration::from_millis(500)),
        )
        .into_result()
        .map_err(|source| CaptureError::Timer {
            op: "discovery",
            source,
        })?;

    mainloop.run();
    Ok(found.get())
}

struct State {
    width: usize,
    height: usize,
    stride: usize,
    frame: Vec<u8>,
    fresh: bool,
    smoothed: Option<Vec<[f32; 3]>>,
    last_send: Instant,
    stats_at: Instant,
    frames: u64,
    proc: Duration,
    end: Option<Reason>,
}

pub fn run(
    cfg: &Config,
    sender: Option<&Rc<RefCell<WledSender>>>,
    last_colors: &Rc<RefCell<Option<Vec<[u8; 3]>>>>,
) -> Result<Reason, CaptureError> {
    let mainloop = MainLoopRc::new(None).map_err(|source| CaptureError::Pipewire {
        op: "main loop",
        source,
    })?;
    let context = ContextRc::new(&mainloop, None).map_err(|source| CaptureError::Pipewire {
        op: "context",
        source,
    })?;
    let core = context
        .connect_rc(None)
        .map_err(|source| CaptureError::Pipewire {
            op: "connect",
            source,
        })?;

    let Some(node_id) = discover(&mainloop, &core, cfg.once)? else {
        if interrupted() {
            return Ok(Reason::Interrupted);
        }
        return Ok(Reason::OnceFailed(
            "no gamescope node after 15 s; is gamescope running?".into(),
        ));
    };
    tracing::info!("found gamescope node, id {node_id}");

    let stream = Rc::new(
        StreamRc::new(
            core.clone(),
            "gamescope-led-sync",
            properties! {
                *pw::keys::MEDIA_TYPE => "Video",
                *pw::keys::MEDIA_CATEGORY => "Capture",
                *pw::keys::MEDIA_ROLE => "Screen",
            },
        )
        .map_err(|source| CaptureError::Pipewire {
            op: "stream",
            source,
        })?,
    );

    let now = Instant::now();
    let state = Rc::new(RefCell::new(State {
        width: 0,
        height: 0,
        stride: 0,
        frame: Vec::new(),
        fresh: false,
        smoothed: None,
        last_send: now,
        stats_at: now,
        frames: 0,
        proc: Duration::ZERO,
        end: None,
    }));

    let preview = cfg.preview.then(|| Rc::new(RefCell::new(Preview::new())));

    let _listener = stream
        .add_local_listener_with_user_data(())
        .state_changed({
            let state = state.clone();
            let ml = mainloop.clone();
            move |_stream, _ud, _old, new| match new {
                StreamState::Error(e) => {
                    state.borrow_mut().end = Some(Reason::Stopped(format!("stream error: {e}")));
                    ml.quit();
                }
                StreamState::Unconnected => {
                    state.borrow_mut().end =
                        Some(Reason::Stopped("stream disconnected (gamescope gone?)".into()));
                    ml.quit();
                }
                _ => {}
            }
        })
        .param_changed({
            let state = state.clone();
            let sample_height = cfg.sample_height as usize;
            move |_stream, _ud, id, param| {
                if id != ParamType::Format.as_raw() {
                    return;
                }
                let Some(param) = param else { return };
                let mut info = VideoInfoRaw::new();
                if info.parse(param).is_err() {
                    return;
                }
                let (w, h) = (info.size().width as usize, info.size().height as usize);
                let mut st = state.borrow_mut();
                if (w, h) != (st.width, st.height) {
                    st.width = w;
                    st.height = h;
                    let step = (h / sample_height.max(1)).max(1);
                    tracing::info!(
                        "capture {}x{} {:?}, subsampling step {} -> {}x{}",
                        w,
                        h,
                        info.format(),
                        step,
                        w.div_ceil(step),
                        h.div_ceil(step),
                    );
                }
            }
        })
        // Buffers are drained by the timer below; the process callback
        // only has to exist so the loop wakes on new frames.
        .process(|_stream, _ud| {})
        .register()
        .map_err(|source| CaptureError::Pipewire { op: "stream listener", source })?;

    let pod_bytes = format_pod(cfg);
    let pod = Pod::from_bytes(&pod_bytes).ok_or(CaptureError::BuildPod)?;
    stream
        .connect(
            Direction::Input,
            Some(node_id),
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
            &mut [pod],
        )
        .map_err(|source| CaptureError::Pipewire {
            op: "stream connect",
            source,
        })?;

    let tick = Duration::from_secs_f64(1.0 / cfg.fps);
    let timer = mainloop.loop_().add_timer({
        let stream = stream.clone();
        let state = state.clone();
        let sender = sender.cloned();
        let last_colors = last_colors.clone();
        let preview = preview.clone();
        let ml = mainloop.clone();
        let cfg = cfg.clone();
        move |_expirations| {
            if QUIT.load(Ordering::SeqCst) {
                state.borrow_mut().end = Some(Reason::Interrupted);
                ml.quit();
                return;
            }

            let mut st = state.borrow_mut();
            let st = &mut *st;

            // Drain everything queued and keep only the newest frame.
            // Dropping a buffer requeues it, so gamescope keeps
            // producing at damage rate; only processing and sending run
            // at --fps. Frames are small thanks to requested_size.
            while let Some(mut buf) = stream.dequeue_buffer() {
                let datas = buf.datas_mut();
                let Some(d) = datas.get_mut(0) else {
                    continue;
                };
                let size = d.chunk().size() as usize;
                let offset = d.chunk().offset() as usize;
                let stride = d.chunk().stride();
                if let Some(bytes) = d.data() {
                    // Valid data is [offset, offset + size), per the SPA
                    // chunk contract; clamp to the mapped region.
                    let start = offset.min(bytes.len());
                    let end = offset.saturating_add(size).min(bytes.len());
                    st.frame.clear();
                    st.frame.extend_from_slice(&bytes[start..end.max(start)]);
                    st.stride = if stride > 0 {
                        stride as usize
                    } else {
                        st.width * 4
                    };
                    st.fresh = true;
                }
            }

            let now = Instant::now();
            let usable = st.fresh
                && st.width > 0
                && st.stride >= st.width * 4
                && st.frame.len() >= st.stride * st.height;

            if usable {
                st.fresh = false;
                let t0 = Instant::now();
                let step = (st.height / (cfg.sample_height as usize).max(1)).max(1);
                let (edges, colors_f, dump) = {
                    let frame = Frame {
                        data: &st.frame,
                        width: st.width,
                        height: st.height,
                        stride: st.stride,
                        step,
                    };
                    let edges = edge_means(&frame, cfg.band_frac, &cfg.layout);
                    let colors_f = chain(&edges, &cfg.layout);
                    let dump = (cfg.once && cfg.dump.is_some()).then(|| sampled_rgb(&frame));
                    (edges, colors_f, dump)
                };

                let replace = cfg.smooth >= 1.0
                    || st
                        .smoothed
                        .as_ref()
                        .is_none_or(|v| v.len() != colors_f.len());
                if replace {
                    st.smoothed = Some(colors_f);
                } else if let Some(prev) = &mut st.smoothed {
                    for (p, n) in prev.iter_mut().zip(colors_f.iter()) {
                        for k in 0..3 {
                            p[k] += cfg.smooth * (n[k] - p[k]);
                        }
                    }
                }
                let colors = shape_colors(st.smoothed.as_ref().unwrap(), cfg.saturation, cfg.gamma);
                st.proc += t0.elapsed();

                if let Some(sender) = &sender {
                    sender.borrow_mut().send(&colors);
                    st.last_send = now;
                }
                *last_colors.borrow_mut() = Some(colors);
                st.frames += 1;

                if let Some(preview) = &preview {
                    preview.borrow_mut().draw(&edges, cfg.saturation, cfg.gamma);
                }

                if cfg.once {
                    // The dump and the logged colors are graded with the
                    // same knobs as the LED output, so a saved frame
                    // previews what --saturation/--gamma do. At the
                    // defaults the grading is a no-op, i.e. the raw
                    // capture.
                    if let (Some(path), Some((rgb, sw, sh))) = (&cfg.dump, &dump) {
                        let graded = grade_rgb8(rgb, cfg.saturation, cfg.gamma);
                        match write_png(path, *sw, *sh, &graded) {
                            Ok(()) => tracing::info!("wrote {path} ({sw}x{sh})"),
                            Err(e) => tracing::warn!("writing {path} failed: {e}"),
                        }
                    }
                    for (name, zones) in [
                        ("top", &edges.top),
                        ("right", &edges.right),
                        ("bottom", &edges.bottom),
                        ("left", &edges.left),
                    ] {
                        // An edge may have zero LEDs (a three-sided
                        // strip), so it has no zones to report.
                        let (Some(first), Some(last)) = (zones.first(), zones.last()) else {
                            continue;
                        };
                        let f = grade_color(*first, cfg.saturation, cfg.gamma);
                        let l = grade_color(*last, cfg.saturation, cfg.gamma);
                        tracing::info!(
                            "{}: first rgb({},{},{}) last rgb({},{},{}), {} zones",
                            name,
                            f[0],
                            f[1],
                            f[2],
                            l[0],
                            l[1],
                            l[2],
                            zones.len(),
                        );
                    }
                    st.end = Some(Reason::OnceDone);
                    ml.quit();
                    return;
                }
            } else if let Some(sender) = &sender {
                // Quiet stream: a static screen is normal (gamescope is
                // damage-driven). Keep WLED in realtime mode so it does
                // not fall back to its preset mid-game. Actual node
                // death arrives via state_changed instead.
                if now.duration_since(st.last_send) >= KEEPALIVE
                    && let Some(colors) = last_colors.borrow().as_ref()
                {
                    sender.borrow_mut().send(colors);
                    st.last_send = now;
                }
            }

            if st.frames > 0 && now.duration_since(st.stats_at) >= STATS_EVERY {
                let elapsed = now.duration_since(st.stats_at).as_secs_f64();
                let sent = sender.as_ref().map_or(0, |s| s.borrow().sent);
                tracing::info!(
                    "{:.1} frames/s, {:.1} ms processing, {} packets sent",
                    st.frames as f64 / elapsed,
                    st.proc.as_secs_f64() * 1000.0 / st.frames as f64,
                    sent,
                );
                st.stats_at = now;
                st.frames = 0;
                st.proc = Duration::ZERO;
            }
        }
    });
    timer
        .update_timer(Some(tick), Some(tick))
        .into_result()
        .map_err(|source| CaptureError::Timer {
            op: "session",
            source,
        })?;

    mainloop.run();

    let end = state.borrow_mut().end.take();
    Ok(end.unwrap_or(Reason::Stopped("main loop ended".into())))
}
