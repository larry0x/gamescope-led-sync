# gamescope-led-sync

Ambient LED backlight for [gamescope](https://github.com/ValveSoftware/gamescope), Valve's Wayland compositor.

**Status:** early work in progress.

## Objective

An ambient LED backlight behind a large TV, driven by a gaming PC. The LEDs show the colors at the screen edges, so the picture appears to spill onto the wall.

The design must meet three rules:

- **No video degradation.** The TV still receives 4K resolution, 144 Hz, HDR, variable refresh rate, and audio, exactly as the PC sends them.
- **No account, cloud, or telemetry.** Everything runs on the local network.
- **Fits a 100-inch TV.** The perimeter is about 7 meters.

No commercial product meets these rules. They fall short in three ways:

- **They degrade the video.** 4K at 144 Hz with 10-bit color needs a 48 Gbit/s HDMI link, or link compression. No sync box or splitter offers either; they cap at 40 Gbit/s. So they drop the refresh rate to 120 Hz, or the color to 8-bit, and they often lose variable refresh rate.
- **They lock you into an account or a cloud.** See the table below.
- **Their LED kits are too short.** None is made for a 100-inch TV.

| Product                        | Video ceiling      | Account or cloud                      |
| ------------------------------ | ------------------ | ------------------------------------- |
| Philips Hue Play HDMI Sync Box | 4K 120 Hz          | Hue account required                  |
| Govee AI Sync Box 2            | _claims_ 4K 144 Hz | stops working without the Govee cloud |
| Lytmi Fantasy 3 / Neo 3        | 4K 120 Hz          | Tuya account required                 |

## This solution

**The simple version:** a program that reads the video frames, computes the colors, and sends it to the LED.

**The more detailed version:** a pipeline that consists of four steps:

1. gamescope draws each frame and publishes it on PipeWire.
2. This program reads the stream, shrinks it, and computes one color for each LED around the screen edges.
3. It sends the colors to a WLED controller over the local network, as DRGB realtime UDP packets.
4. WLED drives the addressable LED strip around the TV.

Desirable property it achieves:

- **One small Rust binary.** It links libpipewire and runs on Linux.
- **Near-zero cost to the game.** It asks gamescope to shrink the capture on the GPU.
- **Portable across gamescope machines.** It runs on SteamOS, Bazzite, ChimeraOS, or a custom build.
- **Fully local.** There is no account, no cloud, and no telemetry. Home Assistant can control WLED over the same network.

## License

[MIT](./LICENSE)
