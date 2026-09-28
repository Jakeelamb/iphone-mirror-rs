use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use iphone_mirror_rs::device::{ConnectionMode, DeviceOptions};
use iphone_mirror_rs::video::{DecodeMode, PresentationOptions};

pub struct Options {
    pub device: DeviceOptions,
    pub decoder: DecodeMode,
    pub duration: Option<Duration>,
    pub headless: bool,
    pub trace: Option<PathBuf>,
    pub measure_stamp: bool,
    pub game_profile: Option<PathBuf>,
    pub presentation: PresentationOptions,
}

impl Options {
    pub fn parse() -> Result<Option<Self>> {
        Self::parse_from(std::env::args().skip(1))
    }

    fn parse_from(mut args: impl Iterator<Item = String>) -> Result<Option<Self>> {
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
            presentation: PresentationOptions::default(),
        };
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h" => {
                    println!(
                        "iphone-mirror-rs\n\nNative iPhone video and input. Requires an already paired, unlocked iPhone.\n\n  --connection auto|usb|wifi  Transport (default auto)\n  --address IP:PORT           Explicit Wi-Fi endpoint\n  --pairing-file PATH         Existing CoreDevice pairing record\n  --serial ID                 Select one paired device\n  --decoder auto|cuda|vaapi|software  Decoder preference (default auto)\n  --software                 Alias for --decoder software\n  --headless                 Decode and trace without opening a window\n  --duration SECONDS         Stop after 0.1–86400 s, including setup\n  --trace PATH               Write timing/counter traces to a new file\n  --measure-stamp            Measure synthetic latency-page frame timestamps\n  --game-profile PATH        Load/create calibrated game controls (F9 setup, F8 play)\n\nControls: click/drag, wheel, keyboard; house button or F1 Home; F2 Spotlight.\nClosing the window releases input and stops the stream."
                    );
                    println!(
                        "\nPresentation experiments (baseline: auto, 1, off):\n  --present-mode auto|immediate|mailbox|fifo\n  --frame-latency 1|2        Backend queue-depth hint, not measured latency\n  --pre-present-notify on|off  Wayland frame-callback experiment\nExplicit unsupported presentation modes fail instead of silently falling back."
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
                "--present-mode" | "--frame-latency" | "--pre-present-notify" => {
                    let value = args
                        .next()
                        .with_context(|| format!("{arg} requires a value"))?;
                    options.presentation.set_option(&arg, &value)?;
                }
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
        if options.headless && options.presentation != PresentationOptions::default() {
            bail!("presentation experiments require a window");
        }
        Ok(Some(options))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iphone_mirror_rs::video::PresentPreference;

    fn parse(args: &[&str]) -> Result<Options> {
        Options::parse_from(args.iter().map(|arg| (*arg).to_owned()))?.context("expected options")
    }

    #[test]
    fn presentation_defaults_and_explicit_experiments() -> Result<()> {
        assert_eq!(parse(&[])?.presentation, PresentationOptions::default());
        let options = parse(&[
            "--present-mode",
            "mailbox",
            "--frame-latency",
            "2",
            "--pre-present-notify",
            "on",
        ])?;
        assert_eq!(options.presentation.mode, PresentPreference::Mailbox);
        assert_eq!(options.presentation.frame_latency, 2);
        assert!(options.presentation.pre_present_notify);
        Ok(())
    }

    #[test]
    fn invalid_or_unused_presentation_experiments_fail_before_startup() {
        for args in [
            vec!["--present-mode"],
            vec!["--present-mode", "fast"],
            vec!["--frame-latency", "0"],
            vec!["--frame-latency", "3"],
            vec!["--pre-present-notify", "maybe"],
            vec!["--headless", "--present-mode", "fifo"],
            vec!["--headless", "--pre-present-notify", "on"],
        ] {
            assert!(parse(&args).is_err(), "accepted {args:?}");
        }
    }
}
