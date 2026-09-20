//! Outputs: WLED DRGB packets, PNG dumps, and the terminal preview.

use {
    crate::geometry::{EdgeColors, grade_color},
    std::{
        fs::File,
        io::{self, IsTerminal, Write},
        net::{SocketAddr, ToSocketAddrs, UdpSocket},
    },
};

// WLED UDP realtime, protocol 2 (DRGB).
// https://kno.wled.ge/interfaces/udp-realtime/
const DRGB_PROTOCOL: u8 = 2;

/// A WLED DRGB realtime packet: the protocol byte, then the timeout
/// byte (seconds WLED stays in realtime after the last packet), then
/// three bytes per LED in order. Up to 490 LEDs fit one packet.
fn drgb_packet(timeout_s: u8, colors: &[[u8; 3]]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(2 + colors.len() * 3);
    packet.push(DRGB_PROTOCOL);
    packet.push(timeout_s);
    for c in colors {
        packet.extend_from_slice(c);
    }
    packet
}

pub struct WledSender {
    sock: UdpSocket,
    addr: SocketAddr,
    timeout_s: u8,
    pub sent: u64,
}

impl WledSender {
    pub fn new(host: &str, port: u16, timeout_s: u8) -> Result<WledSender, String> {
        let addr = (host, port)
            .to_socket_addrs()
            .map_err(|e| format!("cannot resolve {host}: {e}"))?
            .next()
            .ok_or_else(|| format!("no address for {host}"))?;
        let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("udp socket: {e}"))?;
        Ok(WledSender {
            sock,
            addr,
            timeout_s,
            sent: 0,
        })
    }

    pub fn send(&mut self, colors: &[[u8; 3]]) {
        let packet = drgb_packet(self.timeout_s, colors);
        // Fire and forget: UDP errors must never stop the loop.
        let _ = self.sock.send_to(&packet, self.addr);
        self.sent += 1;
    }
}

fn crc32(seed: u32, data: &[u8]) -> u32 {
    let mut crc = seed;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Minimal PNG writer (8-bit RGB), no dependencies: the zlib stream
/// uses stored (uncompressed) deflate blocks, which every reader
/// accepts.
pub fn write_png(path: &str, width: usize, height: usize, rgb: &[u8]) -> io::Result<()> {
    let mut raw = Vec::with_capacity(height * (1 + width * 3));
    for y in 0..height {
        raw.push(0); // filter type: none
        raw.extend_from_slice(&rgb[y * width * 3..(y + 1) * width * 3]);
    }

    let mut idat = vec![0x78, 0x01];
    let chunks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, block) in chunks.iter().enumerate() {
        let last = i + 1 == chunks.len();
        idat.push(last as u8);
        idat.extend_from_slice(&(block.len() as u16).to_le_bytes());
        idat.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        idat.extend_from_slice(block);
    }
    idat.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit, RGB

    let mut f = File::create(path)?;
    f.write_all(b"\x89PNG\r\n\x1a\n")?;
    for (tag, data) in [
        (b"IHDR", ihdr.as_slice()),
        (b"IDAT", idat.as_slice()),
        (b"IEND", &[]),
    ] {
        f.write_all(&(data.len() as u32).to_be_bytes())?;
        f.write_all(tag)?;
        f.write_all(data)?;
        let crc = crc32(crc32(0xFFFF_FFFF, tag), data) ^ 0xFFFF_FFFF;
        f.write_all(&crc.to_be_bytes())?;
    }
    Ok(())
}

/// Four ANSI truecolor bars on a tty, one per edge.
pub struct Preview {
    drawn: bool,
}

impl Preview {
    const WIDTH: usize = 60;

    pub fn new() -> Preview {
        Preview { drawn: false }
    }

    pub fn draw(&mut self, e: &EdgeColors, saturation: f32, gamma: f32) {
        let mut out = io::stdout();
        if !out.is_terminal() {
            return;
        }
        let mut text = String::new();
        if self.drawn {
            text.push_str("\x1b[4F");
        }
        for (name, zones) in [
            ("top", &e.top),
            ("right", &e.right),
            ("bottom", &e.bottom),
            ("left", &e.left),
        ] {
            let cells = zones.len().min(Self::WIDTH);
            text.push_str(&format!("{name:<7}"));
            for i in 0..cells {
                let idx = if cells > 1 {
                    i * (zones.len() - 1) / (cells - 1)
                } else {
                    0
                };
                // Grade the bars like the LED output, so the preview
                // shows the effect of --saturation and --gamma.
                let z = grade_color(zones[idx], saturation, gamma);
                text.push_str(&format!("\x1b[48;2;{};{};{}m ", z[0], z[1], z[2]));
            }
            text.push_str("\x1b[0m\x1b[K\n");
        }
        self.drawn = true;
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }
}

#[cfg(test)]
mod tests {
    // Expected behavior is fixed from the specifications, not from this
    // implementation: DRGB is WLED's realtime protocol, and CRC-32 and
    // Adler-32 have standard test vectors, so a failure here means the
    // code is wrong, never that the expectation should move.
    use {
        super::*,
        std::{fs, time::SystemTime},
    };

    // A DRGB packet is the protocol byte (2), then the timeout byte,
    // then three bytes per LED in order.
    #[test]
    fn drgb_packet_layout() {
        assert_eq!(
            drgb_packet(5, &[[10, 20, 30], [40, 50, 60]]),
            vec![2, 5, 10, 20, 30, 40, 50, 60],
        );
    }

    // With no LEDs the packet is just the two-byte header.
    #[test]
    fn drgb_packet_empty() {
        assert_eq!(drgb_packet(2, &[]), vec![2, 2]);
    }

    // The packet is exactly two header bytes plus three per LED.
    #[test]
    fn drgb_packet_length() {
        let colors = vec![[0u8; 3]; 420];
        assert_eq!(drgb_packet(2, &colors).len(), 2 + 420 * 3);
    }

    // crc32 must be CRC-32/ISO-HDLC, the one PNG uses. Its documented
    // check value for the ASCII string "123456789", with the standard
    // 0xFFFFFFFF pre- and post-conditioning, is 0xCBF43926.
    #[test]
    fn crc32_standard_vector() {
        assert_eq!(crc32(0xFFFF_FFFF, b"123456789") ^ 0xFFFF_FFFF, 0xCBF4_3926);
    }

    // Adler-32 of "123456789" is the well-known 0x091E01DE.
    #[test]
    fn adler32_standard_vector() {
        assert_eq!(adler32(b"123456789"), 0x091E_01DE);
    }

    // write_png must emit a real PNG: the 8-byte signature, then an
    // IHDR carrying the requested width and height, 8-bit truecolor
    // RGB (bit depth 8, color type 2).
    #[test]
    fn write_png_header() {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("gls_png_{}_{nanos}.png", std::process::id()));
        let path = path.to_str().unwrap();
        write_png(path, 3, 2, &[0u8; 3 * 2 * 3]).unwrap();
        let bytes = fs::read(path).unwrap();
        let _ = fs::remove_file(path);

        assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(u32::from_be_bytes(bytes[8..12].try_into().unwrap()), 13); // IHDR length
        assert_eq!(&bytes[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), 3); // width
        assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), 2); // height
        assert_eq!(bytes[24], 8); // bit depth
        assert_eq!(bytes[25], 2); // color type: truecolor RGB
    }
}
