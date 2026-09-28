//! Calibrated, allocation-free five-contact game controls.
use std::fmt;
use std::path::Path;

use crate::input::{HidEvent, TouchPhase, normalized_position};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Action {
    Up,
    Down,
    Left,
    Right,
    LeanLeft,
    LeanRight,
    Reload,
    Interact,
    Melee,
    Grenade,
    Primary,
    Secondary,
    Crouch,
    Fire,
    Aim,
}
const BUTTONS: [Action; 11] = [
    Action::LeanLeft,
    Action::LeanRight,
    Action::Reload,
    Action::Interact,
    Action::Melee,
    Action::Grenade,
    Action::Primary,
    Action::Secondary,
    Action::Crouch,
    Action::Fire,
    Action::Aim,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Joystick,
    Look,
    Button(Action),
}
pub const CALIBRATION_TARGETS: [Target; 13] = [
    Target::Joystick,
    Target::Look,
    Target::Button(Action::LeanLeft),
    Target::Button(Action::LeanRight),
    Target::Button(Action::Reload),
    Target::Button(Action::Interact),
    Target::Button(Action::Melee),
    Target::Button(Action::Grenade),
    Target::Button(Action::Primary),
    Target::Button(Action::Secondary),
    Target::Button(Action::Crouch),
    Target::Button(Action::Fire),
    Target::Button(Action::Aim),
];
const NAMES: [&str; 13] = [
    "joystick",
    "look",
    "lean_left",
    "lean_right",
    "reload",
    "interact",
    "melee",
    "grenade",
    "primary",
    "secondary",
    "crouch",
    "fire",
    "aim",
];
impl Target {
    fn index(self) -> Option<usize> {
        match self {
            Self::Joystick => Some(0),
            Self::Look => Some(1),
            Self::Button(action) => BUTTONS.iter().position(|&a| a == action).map(|i| i + 2),
        }
    }
    pub fn name(self) -> &'static str {
        self.index().map_or("invalid", |i| NAMES[i])
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
impl Point {
    fn valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && (0.0..=1.0).contains(&self.x)
            && (0.0..=1.0).contains(&self.y)
    }
}
#[derive(Debug)]
pub enum GameError {
    InvalidProfile(&'static str),
    Capacity,
    Io(std::io::Error),
}
impl fmt::Display for GameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProfile(reason) => write!(f, "invalid game profile: {reason}"),
            Self::Capacity => {
                f.write_str("three action contacts are already held; release one first")
            }
            Self::Io(error) => write!(f, "game profile I/O: {error}"),
        }
    }
}
impl std::error::Error for GameError {}
impl From<std::io::Error> for GameError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
#[derive(Clone, Debug)]
pub struct Profile {
    points: [Option<Point>; 13],
    /// Fraction of the displayed short edge per relative mouse pixel.
    pub sensitivity: f64,
    /// Joystick displacement as a fraction of the displayed short edge.
    pub joystick_radius: f64,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            points: [None; 13],
            sensitivity: 0.002,
            joystick_radius: 0.08,
        }
    }
}
impl Profile {
    pub fn point(&self, target: Target) -> Option<Point> {
        self.points[target.index()?]
    }
    pub fn set_point(&mut self, target: Target, point: Point) -> Result<(), GameError> {
        let index = target.index().ok_or(GameError::InvalidProfile(
            "movement uses the joystick target",
        ))?;
        if !point.valid() {
            return Err(GameError::InvalidProfile(
                "coordinates must be finite and within 0..1",
            ));
        }
        self.points[index] = Some(point);
        Ok(())
    }
    pub fn calibrated(&self) -> bool {
        self.points.iter().all(Option::is_some)
    }
    fn validate(&self) -> Result<(), GameError> {
        if !self.sensitivity.is_finite() || !(0.000001..=0.1).contains(&self.sensitivity) {
            return Err(GameError::InvalidProfile(
                "sensitivity must be within 0.000001..0.1",
            ));
        }
        if self.points.iter().flatten().any(|p| !p.valid()) {
            return Err(GameError::InvalidProfile("invalid coordinates"));
        }
        if !self.joystick_radius.is_finite() || !(0.001..=0.5).contains(&self.joystick_radius) {
            return Err(GameError::InvalidProfile(
                "joystick_radius must be within 0.001..0.5",
            ));
        }
        Ok(())
    }
    /// Versioned text; absent targets remain uncalibrated, never guessed.
    pub fn parse(text: &str) -> Result<Self, GameError> {
        let mut profile = Self::default();
        let mut version = false;
        let mut sensitivity = false;
        let mut radius = false;
        for line in text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let (name, value) = line
                .split_once('=')
                .ok_or(GameError::InvalidProfile("expected name=value"))?;
            let (name, value) = (name.trim(), value.trim());
            match name {
                "version" if !version && value == "1" => version = true,
                "sensitivity" if !sensitivity => {
                    profile.sensitivity = value
                        .parse()
                        .map_err(|_| GameError::InvalidProfile("invalid sensitivity"))?;
                    sensitivity = true;
                }
                "joystick_radius" if !radius => {
                    profile.joystick_radius = value
                        .parse()
                        .map_err(|_| GameError::InvalidProfile("invalid joystick_radius"))?;
                    radius = true;
                }
                _ => {
                    let index =
                        NAMES
                            .iter()
                            .position(|&n| n == name)
                            .ok_or(GameError::InvalidProfile(
                                "unknown, duplicate or unsupported field",
                            ))?;
                    if profile.points[index].is_some() {
                        return Err(GameError::InvalidProfile("duplicate target"));
                    }
                    let (x, y) = value
                        .split_once(',')
                        .ok_or(GameError::InvalidProfile("expected x,y"))?;
                    let point = Point {
                        x: x.trim()
                            .parse()
                            .map_err(|_| GameError::InvalidProfile("invalid x"))?,
                        y: y.trim()
                            .parse()
                            .map_err(|_| GameError::InvalidProfile("invalid y"))?,
                    };
                    profile.set_point(CALIBRATION_TARGETS[index], point)?;
                }
            }
        }
        if !version {
            return Err(GameError::InvalidProfile("missing version=1"));
        }
        profile.validate()?;
        Ok(profile)
    }
    pub fn load(path: &Path) -> Result<Self, GameError> {
        Self::parse(&std::fs::read_to_string(path)?)
    }
    pub fn save(&self, path: &Path) -> Result<(), GameError> {
        use std::io::Write;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_SAVE: AtomicU64 = AtomicU64::new(0);
        self.validate()?;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut temporary = path.as_os_str().to_os_string();
        temporary.push(format!(
            ".tmp-{}-{}",
            std::process::id(),
            NEXT_SAVE.fetch_add(1, Ordering::Relaxed)
        ));
        let temporary = std::path::PathBuf::from(temporary);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| -> std::io::Result<()> {
            writeln!(
                file,
                "version=1\nsensitivity={}\njoystick_radius={}",
                self.sensitivity, self.joystick_radius
            )?;
            for (i, point) in self.points.iter().enumerate() {
                if let Some(p) = point {
                    writeln!(file, "{}={},{}", NAMES[i], p.x, p.y)?;
                }
            }
            file.sync_all()?;
            std::fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result?;
        Ok(())
    }
}
#[derive(Debug, Default)]
pub struct EventBatch {
    pub events: [Option<HidEvent>; 8],
    pub error: Option<GameError>,
}
impl EventBatch {
    fn push(&mut self, event: HidEvent) {
        if let Some(slot) = self.events.iter_mut().find(|e| e.is_none()) {
            *slot = Some(event);
        } else {
            unreachable!("a game input operation emits at most five frames");
        }
    }
}
impl IntoIterator for EventBatch {
    type Item = Option<HidEvent>;
    type IntoIter = std::array::IntoIter<Self::Item, 8>;
    fn into_iter(self) -> Self::IntoIter {
        self.events.into_iter()
    }
}
#[derive(Clone, Copy, Debug)]
struct Contact {
    x: u16,
    y: u16,
    action: Option<Action>,
}
pub struct GameState {
    profile: Profile,
    contacts: [Option<Contact>; 5],
    held: [bool; 15],
    look: Option<Point>,
    aspect: f64,
    rotation: u16,
}
impl GameState {
    pub fn new(profile: Profile) -> Result<Self, GameError> {
        profile.validate()?;
        if !profile.calibrated() {
            return Err(GameError::InvalidProfile("calibration is incomplete"));
        }
        Ok(Self {
            profile,
            contacts: [None; 5],
            held: [false; 15],
            look: None,
            aspect: 1.0,
            rotation: 0,
        })
    }
    /// Set displayed width/height. Caller releases contacts before geometry changes.
    pub fn set_aspect(&mut self, aspect: f64) -> Result<(), GameError> {
        if !aspect.is_finite() || !(0.01..=100.0).contains(&aspect) {
            return Err(GameError::InvalidProfile("invalid displayed aspect ratio"));
        }
        self.aspect = aspect;
        Ok(())
    }
    fn scale(&self) -> (f64, f64) {
        (1.0 / self.aspect.max(1.0), self.aspect.min(1.0))
    }
    fn point(&self, target: Target) -> Point {
        // Construction requires complete calibration; no runtime mutation exists.
        match self.profile.point(target) {
            Some(p) => p,
            None => unreachable!("validated target"),
        }
    }
    fn contact(&self, p: Point, action: Option<Action>) -> Contact {
        match normalized_position(p.x, p.y, self.rotation) {
            Some((x, y)) => Contact { x, y, action },
            None => unreachable!("finite calibrated coordinates"),
        }
    }
    fn report(&self, phase: TouchPhase, released: u8, timestamp: u64) -> HidEvent {
        let mut report = [0u8; 58];
        report[0] = 9;
        report[2] = 5;
        report[40] = 2;
        report[44..50].copy_from_slice(&timestamp.to_le_bytes()[..6]);
        for (id, contact) in self.contacts.iter().enumerate() {
            if let Some(c) = contact {
                let offset = 3 + usize::from(report[1]) * 5;
                report[offset] = id as u8 | if released & (1 << id) == 0 { 0xc0 } else { 0 };
                report[offset + 1..offset + 3].copy_from_slice(&c.x.to_le_bytes());
                report[offset + 3..offset + 5].copy_from_slice(&c.y.to_le_bytes());
                report[1] += 1;
            }
        }
        HidEvent::Touch { phase, report }
    }
    fn release_slot(&mut self, id: usize, timestamp: u64, batch: &mut EventBatch) {
        if self.contacts[id].is_some() {
            batch.push(self.report(TouchPhase::End, 1 << id, timestamp));
            self.contacts[id] = None;
        }
    }
    fn orient(&mut self, rotation: u16, timestamp: u64) -> EventBatch {
        let rotation = match rotation % 360 {
            90 => 90,
            180 => 180,
            270 => 270,
            _ => 0,
        };
        let batch = if rotation != self.rotation {
            self.release_all(timestamp)
        } else {
            EventBatch::default()
        };
        self.rotation = rotation;
        batch
    }
    pub fn key(
        &mut self,
        action: Action,
        pressed: bool,
        rotation: u16,
        timestamp: u64,
    ) -> EventBatch {
        let mut batch = self.orient(rotation, timestamp);
        if self.held[action as usize] == pressed {
            return batch;
        }
        if (action as usize) < 4 {
            self.held[action as usize] = pressed;
            let center = self.point(Target::Joystick);
            if self.contacts[0].is_none() {
                self.contacts[0] = Some(self.contact(center, None));
                batch.push(self.report(TouchPhase::AnchorBegin, 0, timestamp));
            }
            // Keep the established origin while idle. Returning to center
            // stops movement immediately, without restarting the gesture on
            // the next direction key. release_all still lifts every contact.
            let dx = i32::from(self.held[Action::Right as usize])
                - i32::from(self.held[Action::Left as usize]);
            let dy = i32::from(self.held[Action::Down as usize])
                - i32::from(self.held[Action::Up as usize]);
            let norm = f64::from(dx * dx + dy * dy).sqrt().max(1.0);
            let (sx, sy) = self.scale();
            let p = Point {
                x: (center.x + f64::from(dx) / norm * self.profile.joystick_radius * sx)
                    .clamp(0.0, 1.0),
                y: (center.y + f64::from(dy) / norm * self.profile.joystick_radius * sy)
                    .clamp(0.0, 1.0),
            };
            self.contacts[0] = Some(self.contact(p, None));
            batch.push(self.report(TouchPhase::Move, 0, timestamp));
        } else if pressed {
            let Some(id) = (2..5).find(|&id| self.contacts[id].is_none()) else {
                batch.error = Some(GameError::Capacity);
                return batch;
            };
            self.held[action as usize] = true;
            self.contacts[id] =
                Some(self.contact(self.point(Target::Button(action)), Some(action)));
            batch.push(self.report(TouchPhase::Begin, 0, timestamp));
        } else {
            self.held[action as usize] = false;
            if let Some(id) =
                (2..5).find(|&id| self.contacts[id].is_some_and(|c| c.action == Some(action)))
            {
                self.release_slot(id, timestamp, &mut batch);
            }
        }
        batch
    }
    /// Recenter at a look-region edge; an oversized event is clipped to that
    /// region after recentering, so excess mouse displacement is not replayed.
    pub fn motion(&mut self, dx: f64, dy: f64, rotation: u16, timestamp: u64) -> EventBatch {
        let mut batch = self.orient(rotation, timestamp);
        if !dx.is_finite() || !dy.is_finite() || (dx == 0.0 && dy == 0.0) {
            return batch;
        }
        let center = self.point(Target::Look);
        let (sx, sy) = self.scale();
        let (dx, dy) = (
            dx * self.profile.sensitivity * sx,
            dy * self.profile.sensitivity * sy,
        );
        let mut p = self.look.unwrap_or(center);
        let bounds = (
            (center.x - 0.16 * sx).max(0.0),
            (center.x + 0.16 * sx).min(1.0),
            (center.y - 0.16 * sy).max(0.0),
            (center.y + 0.16 * sy).min(1.0),
        );
        if self.contacts[1].is_some()
            && (p.x + dx < bounds.0
                || p.x + dx > bounds.1
                || p.y + dy < bounds.2
                || p.y + dy > bounds.3)
        {
            self.release_slot(1, timestamp, &mut batch);
            p = center;
        }
        if self.contacts[1].is_none() {
            self.contacts[1] = Some(self.contact(center, None));
            batch.push(self.report(TouchPhase::Begin, 0, timestamp));
        }
        p.x = (p.x + dx).clamp(bounds.0, bounds.1);
        p.y = (p.y + dy).clamp(bounds.2, bounds.3);
        self.look = Some(p);
        self.contacts[1] = Some(self.contact(p, None));
        batch.push(self.report(TouchPhase::Move, 0, timestamp));
        batch
    }
    pub fn release_all(&mut self, timestamp: u64) -> EventBatch {
        let mut batch = EventBatch::default();
        if self.contacts.iter().any(Option::is_some) {
            batch.push(self.report(TouchPhase::End, 0x1f, timestamp));
        }
        self.contacts.fill(None);
        self.held.fill(false);
        self.look = None;
        batch
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    fn profile() -> Profile {
        let mut p = Profile::default();
        for target in CALIBRATION_TARGETS {
            p.set_point(target, Point { x: 0.5, y: 0.5 }).unwrap();
        }
        p
    }
    fn state() -> GameState {
        GameState::new(profile()).unwrap()
    }
    fn reports(batch: EventBatch) -> Vec<(TouchPhase, [u8; 58])> {
        assert!(batch.error.is_none());
        batch
            .into_iter()
            .flatten()
            .map(|event| match event {
                HidEvent::Touch { phase, report } => (phase, report),
                _ => panic!("touch only"),
            })
            .collect()
    }
    fn contact(r: &[u8; 58], id: u8) -> Option<(bool, u16, u16)> {
        (0..usize::from(r[1])).find_map(|i| {
            let offset = 3 + i * 5;
            ((r[offset] & 0x3f) == id).then(|| {
                (
                    r[offset] & 0xc0 == 0xc0,
                    u16::from_le_bytes([r[offset + 1], r[offset + 2]]),
                    u16::from_le_bytes([r[offset + 3], r[offset + 4]]),
                )
            })
        })
    }
    #[test]
    fn requires_calibration_and_roundtrips_profiles() {
        assert!(GameState::new(Profile::default()).is_err());
        let path = std::env::temp_dir().join(format!("mirror-game-{}.txt", uuid::Uuid::new_v4()));
        let p = profile();
        p.save(&path).unwrap();
        let loaded = Profile::load(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(loaded.calibrated());
        assert_eq!(loaded.sensitivity, p.sensitivity);
        assert_eq!(loaded.joystick_radius, p.joystick_radius);
        for target in CALIBRATION_TARGETS {
            assert_eq!(loaded.point(target), p.point(target));
        }
        assert!(
            !Profile::parse("version=1\nlook=0.5,0.5")
                .unwrap()
                .calibrated()
        );
        for text in [
            "",
            "version=2",
            "version=1\nversion=1",
            "version=1\nlook=NaN,0.5",
            "version=1\nlook=1.1,0.5",
            "version=1\nsensitivity=inf",
            "version=1\nsensitivity=0",
            "version=1\njoystick_radius=NaN",
            "version=1\njoystick_radius=0.6",
            "version=1\nlook=0.5,0.5\nlook=0.4,0.4",
            "version=1\nunknown=1",
        ] {
            assert!(Profile::parse(text).is_err(), "{text}");
        }
    }
    #[test]
    fn atomic_save_retains_previous_profile_on_invalid_update_and_cleans_failed_rename() {
        let directory = std::env::temp_dir().join(format!("mirror-save-{}", uuid::Uuid::new_v4()));
        let path = directory.join("profile.txt");
        let mut p = profile();
        p.save(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        p.joystick_radius = f64::NAN;
        assert!(p.save(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        p.joystick_radius = 0.04;
        p.save(&path).unwrap();
        assert_eq!(Profile::load(&path).unwrap().joystick_radius, 0.04);
        let blocked = directory.join("existing-directory");
        std::fs::create_dir(&blocked).unwrap();
        assert!(p.save(&blocked).is_err());
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 2);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn five_contacts_match_reference_full_snapshot_and_release_tombstones() {
        use idevice::core_device::hid::{TouchscreenContact, build_multitouch_report};
        let mut s = state();
        reports(s.key(Action::Up, true, 0, 1));
        reports(s.motion(2.0, 3.0, 0, 2));
        reports(s.key(Action::Fire, true, 0, 3));
        reports(s.key(Action::Aim, true, 0, 4));
        let frame = reports(s.key(Action::Reload, true, 0, 5)).pop().unwrap().1;
        assert_eq!(frame[1], 5);
        let mut contacts: Vec<_> = (0..5)
            .map(|id| {
                let (touching, x, y) = contact(&frame, id).unwrap();
                TouchscreenContact {
                    identity: id,
                    touching,
                    x,
                    y,
                }
            })
            .collect();
        assert_eq!(
            frame.as_slice(),
            build_multitouch_report(&contacts, Some(5)).unwrap()
        );
        let lifted = reports(s.key(Action::Fire, false, 0, 6));
        contacts[2].touching = false;
        assert_eq!(
            lifted[0].1.as_slice(),
            build_multitouch_report(&contacts, Some(6)).unwrap()
        );
        assert_eq!(lifted[0].0, TouchPhase::End);
        assert_eq!(lifted[0].1[1], 5);
        let old = contact(&frame, 2).unwrap();
        assert_eq!(contact(&lifted[0].1, 2), Some((false, old.1, old.2)));
        for id in [0, 1, 3, 4] {
            assert_eq!(contact(&lifted[0].1, id), contact(&frame, id));
        }
        let all = reports(s.release_all(7));
        assert_eq!(all[0].1[1], 4);
        for id in [0, 1, 3, 4] {
            assert!(!contact(&all[0].1, id).unwrap().0);
        }
        assert!(reports(s.release_all(8)).is_empty());
    }
    #[test]
    fn excess_buttons_are_rejected_without_a_stuck_or_stolen_contact() {
        let mut s = state();
        for action in [Action::Fire, Action::Aim, Action::Reload] {
            reports(s.key(action, true, 0, 0));
        }
        let denied = s.key(Action::Interact, true, 0, 1);
        assert!(matches!(denied.error, Some(GameError::Capacity)));
        assert!(denied.events.iter().all(Option::is_none));
        assert!(reports(s.key(Action::Interact, false, 0, 2)).is_empty());
        assert!(reports(s.key(Action::Fire, true, 0, 3)).is_empty()); // repeat
        reports(s.key(Action::Aim, false, 0, 4));
        let accepted = reports(s.key(Action::Interact, true, 0, 5));
        assert_eq!(accepted[0].1[1], 3);
        assert!(contact(&accepted[0].1, 3).unwrap().0);
    }
    #[test]
    fn joystick_starts_center_normalizes_diagonals_and_cancels_opposites() {
        let mut s = state();
        s.set_aspect(2.0).unwrap();
        let initial = reports(s.key(Action::Up, true, 0, 0));
        assert_eq!(initial.len(), 2);
        assert_eq!(initial[0].0, TouchPhase::AnchorBegin);
        assert_eq!(contact(&initial[0].1, 0), Some((true, 32768, 32768)));
        let diagonal = reports(s.key(Action::Right, true, 0, 1));
        let (_, x, y) = contact(&diagonal[0].1, 0).unwrap();
        let distance = ((f64::from(x) / 65535.0 - 0.5).powi(2) * 4.0
            + (f64::from(y) / 65535.0 - 0.5).powi(2))
        .sqrt();
        assert!((distance - 0.08).abs() < 0.0001);
        reports(s.key(Action::Down, true, 0, 2));
        let center = reports(s.key(Action::Left, true, 0, 3));
        assert_eq!(contact(&center[0].1, 0), Some((true, 32768, 32768)));
        for a in [Action::Up, Action::Down, Action::Left] {
            reports(s.key(a, false, 0, 4));
        }
        let release = reports(s.key(Action::Right, false, 0, 5));
        assert_eq!(release[0].0, TouchPhase::Move);
        assert_eq!(contact(&release[0].1, 0), Some((true, 32768, 32768)));
    }

    #[test]
    fn only_joystick_origin_requests_anchor_delivery_spacing() {
        let mut s = state();
        let button = reports(s.key(Action::Aim, true, 0, 0));
        assert_eq!(button[0].0, TouchPhase::Begin);
        let joystick = reports(s.key(Action::Up, true, 0, 1));
        assert_eq!(joystick.len(), 2);
        assert_eq!(joystick[0].0, TouchPhase::AnchorBegin);
        assert_eq!(joystick[1].0, TouchPhase::Move);
        let look = reports(s.motion(1.0, 0.0, 0, 2));
        assert_eq!(look.len(), 2);
        assert_eq!(look[0].0, TouchPhase::Begin);
        assert_eq!(look[1].0, TouchPhase::Move);
        assert_eq!(
            reports(s.key(Action::Right, true, 0, 3))[0].0,
            TouchPhase::Move
        );
        assert_eq!(
            reports(s.key(Action::LeanLeft, true, 0, 4))[0].0,
            TouchPhase::Begin
        );
    }

    #[test]
    fn short_direction_handoffs_stop_at_center_without_restarting_any_contact() {
        use crate::input::InputQueue;
        for gap_ms in [5u64, 10, 20, 30] {
            let mut s = state();
            s.set_aspect(2.0).unwrap();
            reports(s.key(Action::Up, true, 0, 0));
            reports(s.key(Action::LeanLeft, true, 0, 1_000_000));
            let looking = reports(s.motion(2.0, 1.0, 0, 2_000_000));
            let look = contact(&looking.last().unwrap().1, 1).unwrap();
            let lean = contact(&looking.last().unwrap().1, 2).unwrap();
            let neutral = s.key(Action::Up, false, 0, 100_000_000);
            let mut queue = InputQueue::new(128);
            for event in neutral.into_iter().flatten() {
                queue.try_push(event).unwrap();
            }
            let HidEvent::Touch { phase, report } = queue.pop().unwrap() else {
                panic!("neutral frame");
            };
            assert_eq!(phase, TouchPhase::Move);
            assert_eq!(&report[44..50], &100_000_000u64.to_le_bytes()[..6]);
            assert_eq!(contact(&report, 0), Some((true, 32768, 32768)));
            assert_eq!(contact(&report, 1), Some(look));
            assert_eq!(contact(&report, 2), Some(lean));
            assert!(queue.pop().is_none());
            let next = s.key(Action::Right, true, 0, (100 + gap_ms) * 1_000_000);
            for event in next.into_iter().flatten() {
                queue.try_push(event).unwrap();
            }
            let HidEvent::Touch { phase, report } = queue.pop().unwrap() else {
                panic!("new direction");
            };
            assert_eq!(phase, TouchPhase::Move); // no lift or new anchored touchdown
            assert_eq!(contact(&report, 0), Some((true, 35389, 32768)));
            assert_eq!(contact(&report, 1), Some(look));
            assert_eq!(contact(&report, 2), Some(lean));
            assert!(queue.pop().is_none());
            reports(s.key(Action::Right, false, 0, 150_000_000));
            let lifted = reports(s.release_all(160_000_000));
            assert_eq!(lifted.len(), 1);
            assert_eq!(lifted[0].0, TouchPhase::End);
            assert_eq!(contact(&lifted[0].1, 0), Some((false, 32768, 32768)));
            assert_eq!(contact(&lifted[0].1, 1), Some((false, look.1, look.2)));
            assert_eq!(contact(&lifted[0].1, 2), Some((false, lean.1, lean.2)));
        }
    }
    #[test]
    fn look_recenter_preserves_other_contacts_and_marks_transitions() {
        let mut s = state();
        reports(s.key(Action::Fire, true, 0, 0));
        let first = reports(s.motion(60.0, 0.0, 0, 1));
        let old_look = contact(&first[1].1, 1).unwrap();
        let old_fire = contact(&first[1].1, 2).unwrap();
        let frames = reports(s.motion(60.0, 0.0, 0, 2));
        assert_eq!(
            frames.iter().map(|f| f.0).collect::<Vec<_>>(),
            [TouchPhase::End, TouchPhase::Begin, TouchPhase::Move]
        );
        assert_eq!(
            contact(&frames[0].1, 1),
            Some((false, old_look.1, old_look.2))
        );
        assert_eq!(contact(&frames[1].1, 1), Some((true, 32768, 32768)));
        for (_, frame) in frames {
            assert_eq!(contact(&frame, 2), Some(old_fire));
        }
        assert!(reports(s.motion(f64::NAN, 0.0, 0, 3)).is_empty());
        let large = reports(s.motion(f64::MAX, -f64::MAX, 0, 4));
        let (_, x, y) = contact(&large.last().unwrap().1, 1).unwrap();
        assert!((f64::from(x) / 65535.0 - 0.66).abs() < 0.0001);
        assert!((f64::from(y) / 65535.0 - 0.34).abs() < 0.0001);
    }
    #[test]
    fn rotation_releases_old_coordinates_then_maps_new_contacts() {
        let mut p = profile();
        p.set_point(Target::Button(Action::Fire), Point { x: 0.25, y: 0.75 })
            .unwrap();
        let mut s = GameState::new(p).unwrap();
        let old = reports(s.key(Action::Fire, true, 0, 0))[0].1;
        let frames = reports(s.key(Action::Aim, true, 90, 1));
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].0, TouchPhase::End);
        let (_, x, y) = contact(&old, 2).unwrap();
        assert_eq!(contact(&frames[0].1, 2), Some((false, x, y)));
        reports(s.key(Action::Aim, false, 90, 2));
        let next = reports(s.key(Action::Fire, true, 90, 3));
        assert_eq!(contact(&next[0].1, 2), Some((true, 49151, 49151)));
    }

    #[test]
    fn mixed_sequences_preserve_remote_contact_lifetimes_and_move_identity_sets() {
        let actions = [
            Action::Up,
            Action::Down,
            Action::Left,
            Action::Right,
            Action::LeanLeft,
            Action::LeanRight,
            Action::Reload,
            Action::Interact,
            Action::Melee,
            Action::Grenade,
            Action::Primary,
            Action::Secondary,
            Action::Crouch,
            Action::Fire,
            Action::Aim,
        ];
        let mut state = state();
        let mut remote: [Option<(u16, u16)>; 5] = [None; 5];
        let mut seed = 0xcafef00d12345678u64;
        for step in 0..4096u64 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let rotation = ((step / 71) % 4 * 90) as u16;
            let batch = if step % 53 == 0 || step == 4095 {
                state.release_all(step)
            } else if step % 5 == 0 {
                state.motion(
                    (seed % 501) as f64 - 250.0,
                    ((seed >> 16) % 501) as f64 - 250.0,
                    rotation,
                    step,
                )
            } else {
                state.key(
                    actions[seed as usize % actions.len()],
                    seed & 0x100 != 0,
                    rotation,
                    step,
                )
            };
            assert!(batch.error.is_none() || matches!(batch.error, Some(GameError::Capacity)));
            for event in batch.into_iter().flatten() {
                let HidEvent::Touch { phase, report } = event else {
                    panic!("touch frame");
                };
                assert_eq!(report[0], 9);
                assert!((1..=5).contains(&report[1]));
                assert_eq!(&report[44..50], &step.to_le_bytes()[..6]);
                let mut seen = [false; 5];
                let mut next = [None; 5];
                let mut began = 0;
                let mut ended = 0;
                for slot in 0..usize::from(report[1]) {
                    let offset = 3 + slot * 5;
                    let id = usize::from(report[offset] & 0x3f);
                    assert!(id < 5 && !seen[id]);
                    seen[id] = true;
                    let xy = (
                        u16::from_le_bytes([report[offset + 1], report[offset + 2]]),
                        u16::from_le_bytes([report[offset + 3], report[offset + 4]]),
                    );
                    if report[offset] & 0xc0 == 0xc0 {
                        began += usize::from(remote[id].is_none());
                        if phase != TouchPhase::Move && remote[id].is_some() {
                            assert_eq!(
                                remote[id],
                                Some(xy),
                                "unrelated contact moved during transition"
                            );
                        }
                        next[id] = Some(xy);
                    } else {
                        assert_eq!(report[offset] & 0xc0, 0);
                        assert_eq!(
                            remote[id],
                            Some(xy),
                            "release must retain its last coordinates"
                        );
                        ended += 1;
                    }
                }
                for id in 0..5 {
                    assert!(
                        remote[id].is_none() || seen[id],
                        "live contact omitted without a tombstone"
                    );
                }
                match phase {
                    TouchPhase::Begin | TouchPhase::AnchorBegin => {
                        assert!(began > 0);
                        assert_eq!(ended, 0);
                    }
                    TouchPhase::End => {
                        assert!(ended > 0);
                        assert_eq!(began, 0);
                    }
                    TouchPhase::Move => {
                        assert_eq!(began, 0);
                        assert_eq!(ended, 0);
                    }
                }
                remote = next;
            }
            for (id, contact) in state.contacts.iter().enumerate() {
                assert_eq!(remote[id], contact.map(|c| (c.x, c.y)));
            }
            assert!(!state.held[..4].iter().any(|&h| h) || state.contacts[0].is_some());
            assert_eq!(state.contacts[1].is_some(), state.look.is_some());
            for action in BUTTONS {
                assert_eq!(
                    state.held[action as usize],
                    state.contacts[2..]
                        .iter()
                        .flatten()
                        .any(|c| c.action == Some(action))
                );
            }
        }
        assert_eq!(remote, [None; 5]);
    }
}
