use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use iphone_mirror_rs::device::{ConnectionMode, DeviceOptions};
use iphone_mirror_rs::video::DecodeMode;

pub struct Options {
    pub device: DeviceOptions,
    pub decoder: DecodeMode,
    pub duration: Option<Duration>,
    pub headless: bool,
    pub trace: Option<PathBuf>,
    pub measure_stamp: bool,
    pub game_profile: Option<PathBuf>,
}

impl Options {
    pub fn parse() -> Result<Option<Self>> {
        let mut options = Self {
            device: DeviceOptions {
                connection: ConnectionMode::Auto,
                serial: None,
                address: None,
                pairing_file: None,
            },
            decoder: DecodeMode::Auto,
            duration: None,
            headless: false,
            trace: None,
            measure_stamp: false,
            game_profile: None,
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h" => {
                    println!(
                        "iphone-mirror-rs\n\nNative iPhone video and input. Requires an already paired, unlocked iPhone.\n\n  --connection auto|usb|wifi  Transport (default auto)\n  --address IP:PORT           Explicit Wi-Fi endpoint\n  --pairing-file PATH         Existing CoreDevice pairing record\n  --serial ID                 Select one paired device\n  --decoder auto|cuda|vaapi|software  Decoder preference (default auto)\n  --software                 Alias for --decoder software\n  --headless                 Decode and trace without opening a window\n  --duration SECONDS         Stop after 0.1–86400 s, including setup\n  --trace PATH               Write timing/counter traces to a new file\n  --measure-stamp            Measure synthetic latency-page frame timestamps\n  --game-profile PATH        Load/create calibrated game controls (F9 setup, F8 play)\n\nControls: click/drag, wheel, keyboard; house button or F1 Home; F2 Spotlight.\nClosing the window releases input and stops the stream."
                    );
                    return Ok(None);
                }
                "--connection" => {
                    options.device.connection = match args.next().as_deref() {
                        Some("auto") => ConnectionMode::Auto,
                        Some("usb") => ConnectionMode::Usb,
                        Some("wifi") => ConnectionMode::Wifi,
                        _ => bail!("--connection requires auto, usb or wifi"),
                    }
                }
                "--address" => {
                    options.device.address = Some(
                        args.next()
                            .context("--address requires IP:PORT")?
                            .parse::<SocketAddr>()
                            .context("invalid --address IP:PORT")?,
                    );
                }
                "--pairing-file" => {
                    options.device.pairing_file = Some(PathBuf::from(
                        args.next().context("--pairing-file requires a path")?,
                    ))
                }
                "--serial" => {
                    options.device.serial =
                        Some(args.next().context("--serial requires an identifier")?)
                }
                "--decoder" => {
                    options.decoder = match args.next().as_deref() {
                        Some("auto") => DecodeMode::Auto,
                        Some("cuda") => DecodeMode::Cuda,
                        Some("vaapi") => DecodeMode::Vaapi,
                        Some("software") => DecodeMode::Software,
                        _ => bail!("--decoder requires auto, cuda, vaapi or software"),
                    };
                }
                "--software" => options.decoder = DecodeMode::Software,
                "--headless" => options.headless = true,
                "--game-profile" => {
                    options.game_profile = Some(PathBuf::from(
                        args.next().context("--game-profile requires a path")?,
                    ));
                }
                "--measure-stamp" => options.measure_stamp = true,
                "--duration" => {
                    let seconds = args
                        .next()
                        .context("--duration requires seconds")?
                        .parse::<f64>()
                        .context("invalid duration")?;
                    if !seconds.is_finite() || !(0.1..=86400.0).contains(&seconds) {
                        bail!("duration must be between 0.1 and 86400 seconds");
                    }
                    options.duration = Some(Duration::from_secs_f64(seconds));
                }
                "--trace" => {
                    options.trace = Some(PathBuf::from(
                        args.next().context("--trace requires a path")?,
                    ))
                }
                _ => bail!("unknown option; use --help"),
            }
        }
        if options.headless && options.game_profile.is_some() {
            bail!("--game-profile requires a window");
        }
        Ok(Some(options))
    }
}
