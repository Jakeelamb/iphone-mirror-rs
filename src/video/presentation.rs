//! Explicit presentation experiments. Defaults preserve the qualified policy.
use anyhow::{Result, bail, ensure};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PresentPreference {
    #[default]
    Auto,
    Immediate,
    Mailbox,
    Fifo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresentationOptions {
    pub mode: PresentPreference,
    /// A backend-clamped hint, not a measured number of queued pictures.
    pub frame_latency: u32,
    pub pre_present_notify: bool,
}

impl Default for PresentationOptions {
    fn default() -> Self {
        Self {
            mode: PresentPreference::Auto,
            frame_latency: 1,
            pre_present_notify: false,
        }
    }
}

impl PresentationOptions {
    /// Shared by the real viewer and the device-free surface fixture.
    pub fn set_option(&mut self, flag: &str, value: &str) -> Result<()> {
        match flag {
            "--present-mode" => {
                self.mode = match value {
                    "auto" => PresentPreference::Auto,
                    "immediate" => PresentPreference::Immediate,
                    "mailbox" => PresentPreference::Mailbox,
                    "fifo" => PresentPreference::Fifo,
                    _ => bail!("--present-mode requires auto, immediate, mailbox or fifo"),
                };
            }
            "--frame-latency" => {
                self.frame_latency = match value {
                    "1" => 1,
                    "2" => 2,
                    _ => bail!("--frame-latency requires 1 or 2"),
                };
            }
            "--pre-present-notify" => {
                self.pre_present_notify = match value {
                    "on" => true,
                    "off" => false,
                    _ => bail!("--pre-present-notify requires on or off"),
                };
            }
            _ => bail!("unknown presentation option"),
        }
        Ok(())
    }

    /// An explicit unsupported mode fails instead of silently changing an A/B run.
    pub fn select_mode(&self, supported: &[wgpu::PresentMode]) -> Result<wgpu::PresentMode> {
        ensure!(
            matches!(self.frame_latency, 1 | 2),
            "frame latency must be 1 or 2"
        );
        use wgpu::PresentMode;
        let requested = match self.mode {
            PresentPreference::Auto => {
                return [
                    PresentMode::Immediate,
                    PresentMode::Mailbox,
                    PresentMode::Fifo,
                ]
                .into_iter()
                .find(|mode| supported.contains(mode))
                .ok_or_else(|| anyhow::anyhow!("GPU surface has no supported present mode"));
            }
            PresentPreference::Immediate => PresentMode::Immediate,
            PresentPreference::Mailbox => PresentMode::Mailbox,
            PresentPreference::Fifo => PresentMode::Fifo,
        };
        ensure!(
            supported.contains(&requested),
            "requested present mode {requested:?} unavailable; supported modes: {supported:?}"
        );
        Ok(requested)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_preserves_priority_independent_of_capability_order() -> Result<()> {
        use wgpu::PresentMode::*;
        let options = PresentationOptions::default();
        assert_eq!(options.select_mode(&[Fifo, Mailbox, Immediate])?, Immediate);
        assert_eq!(options.select_mode(&[Fifo, Mailbox])?, Mailbox);
        assert_eq!(options.select_mode(&[Fifo])?, Fifo);
        assert!(options.select_mode(&[]).is_err());
        Ok(())
    }

    #[test]
    fn explicit_mode_never_silently_falls_back() -> Result<()> {
        let mut options = PresentationOptions::default();
        options.set_option("--present-mode", "mailbox")?;
        assert!(options.select_mode(&[wgpu::PresentMode::Fifo]).is_err());
        assert_eq!(
            options.select_mode(&[wgpu::PresentMode::Fifo, wgpu::PresentMode::Mailbox])?,
            wgpu::PresentMode::Mailbox
        );
        Ok(())
    }

    #[test]
    fn experiment_values_are_bounded_and_reject_typos() -> Result<()> {
        let baseline = PresentationOptions::default();
        assert_eq!(baseline.frame_latency, 1);
        assert!(!baseline.pre_present_notify);
        for (flag, bad) in [
            ("--present-mode", "fast"),
            ("--frame-latency", "0"),
            ("--frame-latency", "3"),
            ("--frame-latency", "NaN"),
            ("--pre-present-notify", "true"),
            ("--unknown", "1"),
        ] {
            let mut options = baseline;
            assert!(options.set_option(flag, bad).is_err());
            assert_eq!(options, baseline);
        }
        let mut options = baseline;
        options.set_option("--frame-latency", "2")?;
        options.set_option("--pre-present-notify", "on")?;
        assert_eq!(options.frame_latency, 2);
        assert!(options.pre_present_notify);
        options.frame_latency = 0;
        assert!(options.select_mode(&[wgpu::PresentMode::Fifo]).is_err());
        Ok(())
    }
}
