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
    Sprint,
    Vault,
    Rappel,
    SecondaryGadget,
    Mount,
}
const BUTTONS: [Action; 15] = [
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
    Action::Vault,
    Action::Rappel,
    Action::SecondaryGadget,
    Action::Mount,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Joystick,
    Look,
    Sprint,
    Button(Action),
}
pub const REQUIRED_TARGET_COUNT: usize = 13;
pub const CALIBRATION_TARGETS: [Target; 18] = [
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
    Target::Button(Action::Vault),
    Target::Button(Action::Rappel),
    Target::Button(Action::SecondaryGadget),
    Target::Button(Action::Mount),
    Target::Sprint,
];
const NAMES: [&str; 18] = [
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
    "vault",
    "rappel",
    "secondary_gadget",
    "mount",
    "sprint",
];
impl Target {
    fn index(self) -> Option<usize> {
        match self {
            Self::Joystick => Some(0),
            Self::Look => Some(1),
            Self::Sprint => Some(17),
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
    Uncalibrated(Target),
    Capacity,
    Io(std::io::Error),
}
impl fmt::Display for GameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProfile(reason) => write!(f, "invalid game profile: {reason}"),
            Self::Uncalibrated(target) => write!(
                f,
                "{} is not calibrated; press F10, its key, then click its HUD target",
                target.name()
            ),
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
    points: [Option<Point>; CALIBRATION_TARGETS.len()],
    /// Fraction of the displayed short edge per raw relative mouse unit.
    pub sensitivity: f64,
    /// Joystick displacement as a fraction of the displayed short edge.
    pub joystick_radius: f64,
    /// Forward joystick radius multiplier while sprint is held, capped at 0.5.
    pub sprint_multiplier: f64,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            points: [None; CALIBRATION_TARGETS.len()],
            sensitivity: 0.002,
            joystick_radius: 0.08,
            sprint_multiplier: 2.0,
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
        self.points[..REQUIRED_TARGET_COUNT]
            .iter()
            .all(Option::is_some)
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
        if let (Some(center), Some(sprint)) =
            (self.point(Target::Joystick), self.point(Target::Sprint))
            && sprint.y >= center.y
        {
            return Err(GameError::InvalidProfile(
                "sprint endpoint must be above the joystick center",
            ));
        }
        if !self.joystick_radius.is_finite() || !(0.001..=0.5).contains(&self.joystick_radius) {
            return Err(GameError::InvalidProfile(
                "joystick_radius must be within 0.001..0.5",
            ));
        }
        if !self.sprint_multiplier.is_finite() || !(1.0..=4.0).contains(&self.sprint_multiplier) {
            return Err(GameError::InvalidProfile(
                "sprint_multiplier must be within 1..4",
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
        let mut sprint = false;
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
                "sprint_multiplier" if !sprint => {
                    profile.sprint_multiplier = value
                        .parse()
                        .map_err(|_| GameError::InvalidProfile("invalid sprint_multiplier"))?;
                    sprint = true;
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
                "version=1\nsensitivity={}\njoystick_radius={}\nsprint_multiplier={}",
                self.sensitivity, self.joystick_radius, self.sprint_multiplier
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
    pub events: [Option<HidEvent>; 32],
    pub error: Option<GameError>,
    /// Look contacts lifted and reanchored by this operation (not initial down).
    pub look_resets: u32,
    /// Motion remained after the per-callback work bound, or was invalid/blocked.
    pub clipped_motion: bool,
}
impl EventBatch {
    fn push(&mut self, event: HidEvent) {
        if let Some(slot) = self.events.iter_mut().find(|e| e.is_none()) {
            *slot = Some(event);
        } else {
            unreachable!("eight look segments and their transitions fit in 32 frames");
        }
    }
}
impl IntoIterator for EventBatch {
    type Item = Option<HidEvent>;
    type IntoIter = std::array::IntoIter<Self::Item, 32>;
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
    held: [bool; Action::Mount as usize + 1],
    look: Option<Point>,
    aspect: f64,
    rotation: u16,
    movement_idle_since: Option<u64>,
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
            held: [false; Action::Mount as usize + 1],
            look: None,
            aspect: 1.0,
            rotation: 0,
            movement_idle_since: None,
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
            self.expire_idle_movement(timestamp)
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
        if (action as usize) < 4 || action == Action::Sprint {
            self.held[action as usize] = pressed;
            let center = self.point(Target::Joystick);
            // Shift alone never begins a gesture or starts moving.
            if action == Action::Sprint && self.contacts[0].is_none() {
                return batch;
            }
            if self.contacts[0].is_none() {
                self.contacts[0] = Some(self.contact(center, None));
                batch.push(self.report(TouchPhase::AnchorBegin, 0, timestamp));
            }
            // Preserve the origin during quick handoffs, but let the event-loop
            // timer lift idle movement so it cannot span screen changes forever.
            if self.held[..4].iter().any(|&held| held) {
                self.movement_idle_since = None;
            } else {
                self.movement_idle_since.get_or_insert(timestamp);
            }
            let dx = i32::from(self.held[Action::Right as usize])
                - i32::from(self.held[Action::Left as usize]);
            let dy = i32::from(self.held[Action::Down as usize])
                - i32::from(self.held[Action::Up as usize]);
            let norm = f64::from(dx * dx + dy * dy).sqrt().max(1.0);
            let (sx, sy) = self.scale();
            let radius = if self.held[Action::Sprint as usize] && dy < 0 {
                self.profile.point(Target::Sprint).map_or_else(
                    || (self.profile.joystick_radius * self.profile.sprint_multiplier).min(0.5),
                    |endpoint| (center.y - endpoint.y) / sy,
                )
            } else {
                self.profile.joystick_radius
            };
            let p = Point {
                x: (center.x + f64::from(dx) / norm * radius * sx).clamp(0.0, 1.0),
                y: (center.y + f64::from(dy) / norm * radius * sy).clamp(0.0, 1.0),
            };
            self.contacts[0] = Some(self.contact(p, None));
            batch.push(self.report(TouchPhase::Move, 0, timestamp));
        } else if pressed {
            let target = Target::Button(action);
            let Some(point) = self.profile.point(target) else {
                batch.error = Some(GameError::Uncalibrated(target));
                return batch;
            };
            let Some(id) = (2..5).find(|&id| self.contacts[id].is_none()) else {
                batch.error = Some(GameError::Capacity);
                return batch;
            };
            self.held[action as usize] = true;
            self.contacts[id] = Some(self.contact(point, Some(action)));
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
    /// Split a mouse vector at look-region edges, lifting/reanchoring between
    /// segments. Process at most eight segments in this callback; discard any
    /// remaining displacement with an explicit flag, never a deferred tail.
    pub fn motion(&mut self, dx: f64, dy: f64, rotation: u16, timestamp: u64) -> EventBatch {
        const MAX_SEGMENTS: usize = 8;
        let mut batch = self.orient(rotation, timestamp);
        if !dx.is_finite() || !dy.is_finite() {
            batch.clipped_motion = true;
            return batch;
        }
        if dx == 0.0 && dy == 0.0 {
            return batch;
        }
        let center = self.point(Target::Look);
        let (sx, sy) = self.scale();
        let (mut dx, mut dy) = (
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
        // One common fraction preserves the vector direction at edges/corners.
        let fraction = |p: Point, dx: f64, dy: f64| {
            let axis = |position: f64, delta: f64, low: f64, high: f64| {
                if delta > 0.0 {
                    (high - position) / delta
                } else if delta < 0.0 {
                    (low - position) / delta
                } else {
                    1.0
                }
            };
            axis(p.x, dx, bounds.0, bounds.1)
                .min(axis(p.y, dy, bounds.2, bounds.3))
                .clamp(0.0, 1.0)
        };
        for segment in 0..MAX_SEGMENTS {
            let part = fraction(p, dx, dy);
            if part > 0.0 {
                if self.contacts[1].is_none() {
                    self.contacts[1] = Some(self.contact(p, None));
                    batch.push(self.report(TouchPhase::Begin, 0, timestamp));
                }
                p.x = (p.x + dx * part).clamp(bounds.0, bounds.1);
                p.y = (p.y + dy * part).clamp(bounds.2, bounds.3);
                self.look = Some(p);
                self.contacts[1] = Some(self.contact(p, None));
                batch.push(self.report(TouchPhase::Move, 0, timestamp));
                if part == 1.0 {
                    return batch;
                }
                dx *= 1.0 - part;
                dy *= 1.0 - part;
            }
            // A target exactly on the screen edge cannot accept outward motion.
            // Avoid an endless lift/down loop or shifting the calibrated anchor.
            if segment + 1 == MAX_SEGMENTS || fraction(center, dx, dy) == 0.0 {
                batch.clipped_motion = true;
                break;
            }
            self.release_slot(1, timestamp, &mut batch);
            p = center;
            self.contacts[1] = Some(self.contact(center, None));
            self.look = Some(center);
            batch.push(self.report(TouchPhase::Begin, 0, timestamp));
            batch.look_resets += 1;
        }
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
        self.movement_idle_since = None;
        batch
    }
    /// Deadline in the caller's nanosecond timestamp domain.
    pub fn movement_deadline(&self) -> Option<u64> {
        self.movement_idle_since
            .map(|at| at.saturating_add(150_000_000))
    }
    /// Lift only idle movement; aiming and held action contacts remain intact.
    pub fn expire_idle_movement(&mut self, timestamp: u64) -> EventBatch {
        let mut batch = EventBatch::default();
        if self.movement_deadline().is_some_and(|at| timestamp >= at) {
            self.release_slot(0, timestamp, &mut batch);
            self.movement_idle_since = None;
        }
        batch
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    fn profile() -> Profile {
        let mut p = Profile::default();
        for target in CALIBRATION_TARGETS
            .into_iter()
            .filter(|&target| target != Target::Sprint)
        {
            p.set_point(target, Point { x: 0.5, y: 0.5 }).unwrap();
        }
        p
    }
    fn state() -> GameState {
        GameState::new(profile()).unwrap()
    }
    fn emitted_look_displacement(batches: impl IntoIterator<Item = EventBatch>) -> (i64, i64) {
        let mut previous = None;
        let mut total = (0, 0);
        for batch in batches {
            for (_, report) in reports(batch) {
                match contact(&report, 1) {
                    Some((true, x, y)) => {
                        if let Some((px, py)) = previous {
                            total.0 += i64::from(x) - i64::from(px);
                            total.1 += i64::from(y) - i64::from(py);
                        }
                        previous = Some((x, y));
                    }
                    _ => previous = None,
                }
            }
        }
        total
    }

    #[test]
    fn look_displacement_is_independent_of_mouse_event_grouping() {
        let mut one = state();
        let mut split = state();
        let whole = emitted_look_displacement([one.motion(100.0, 0.0, 0, 1)]);
        let pieces = emitted_look_displacement([
            split.motion(50.0, 0.0, 0, 1),
            split.motion(50.0, 0.0, 0, 2),
        ]);
        assert!(
            (whole.0 - pieces.0).abs() <= 2,
            "100 units: {whole:?}; 2x50: {pieces:?}"
        );
        assert!((whole.0 as f64 - 0.2 * 65535.0).abs() <= 2.0);
        assert_eq!((whole.1, pieces.1), (0, 0));
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
        assert_eq!(loaded.sprint_multiplier, p.sprint_multiplier);
        assert_eq!(Profile::parse("version=1").unwrap().sprint_multiplier, 2.0);
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
            "version=1\nsprint_multiplier=NaN",
            "version=1\nsprint_multiplier=0.9",
            "version=1\nsprint_multiplier=4.1",
            "version=1\nsprint_multiplier=2\nsprint_multiplier=3",
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
    fn sprint_changes_existing_forward_contact_and_preserves_other_fingers() {
        let mut s = state();
        assert!(reports(s.key(Action::Sprint, true, 0, 0)).is_empty());
        let forward = reports(s.key(Action::Up, true, 0, 1));
        assert_eq!(forward.len(), 2);
        assert_eq!(contact(&forward[1].1, 0), Some((true, 32768, 22282)));
        reports(s.motion(1.0, 1.0, 0, 2));
        reports(s.key(Action::LeanLeft, true, 0, 3));
        let held = reports(s.key(Action::Fire, true, 0, 4))[0].1;
        let walking = reports(s.key(Action::Sprint, false, 0, 5));
        assert_eq!(walking.len(), 1);
        assert_eq!(walking[0].0, TouchPhase::Move);
        assert_eq!(contact(&walking[0].1, 0), Some((true, 32768, 27525)));
        for id in 1..4 {
            assert_eq!(contact(&walking[0].1, id), contact(&held, id));
        }
        reports(s.key(Action::Sprint, true, 0, 6));
        let neutral = reports(s.key(Action::Up, false, 0, 7));
        assert_eq!(contact(&neutral[0].1, 0), Some((true, 32768, 32768)));
        let backward = reports(s.key(Action::Down, true, 0, 8));
        assert_eq!(contact(&backward[0].1, 0), Some((true, 32768, 38010)));
        reports(s.release_all(9));
        let fresh = reports(s.key(Action::Up, true, 0, 10));
        assert_eq!(contact(&fresh[1].1, 0), Some((true, 32768, 27525)));
    }

    #[test]
    fn idle_movement_expires_before_reusing_a_gesture_after_respawn_pause() {
        let mut s = state();
        reports(s.key(Action::Up, true, 0, 0));
        let looking = reports(s.motion(1.0, 1.0, 0, 1));
        let look = contact(&looking.last().unwrap().1, 1);
        reports(s.key(Action::Up, false, 0, 10_000_000));
        assert!(reports(s.expire_idle_movement(159_999_999)).is_empty());
        let idle = reports(s.expire_idle_movement(160_000_000));
        assert_eq!(
            idle.len(),
            1,
            "a centered movement touch must not survive an idle pause indefinitely"
        );
        assert_eq!(idle[0].0, TouchPhase::End);
        assert!(!contact(&idle[0].1, 0).unwrap().0);
        assert_eq!(contact(&idle[0].1, 1), look);
        let next = reports(s.key(Action::Right, true, 0, 200_000_000));
        assert_eq!(next[0].0, TouchPhase::AnchorBegin);
        assert_eq!(next[1].0, TouchPhase::Move);
        assert_eq!(contact(&next[1].1, 1), look);
    }

    #[test]
    fn movement_before_idle_deadline_cancels_expiry_and_late_input_reanchors() {
        let mut s = state();
        reports(s.key(Action::Up, true, 0, 0));
        reports(s.key(Action::Up, false, 0, 10_000_000));
        assert_eq!(s.movement_deadline(), Some(160_000_000));
        let quick = reports(s.key(Action::Right, true, 0, 159_000_000));
        assert_eq!(quick.len(), 1);
        assert_eq!(quick[0].0, TouchPhase::Move);
        assert_eq!(s.movement_deadline(), None);
        assert!(reports(s.expire_idle_movement(200_000_000)).is_empty());
        reports(s.key(Action::Right, false, 0, 210_000_000));
        let late = reports(s.key(Action::Left, true, 0, 400_000_000));
        assert_eq!(
            late.iter().map(|(phase, _)| *phase).collect::<Vec<_>>(),
            [TouchPhase::End, TouchPhase::AnchorBegin, TouchPhase::Move]
        );
        reports(s.release_all(410_000_000));
        assert_eq!(s.movement_deadline(), None);
    }

    #[test]
    fn sprint_diagonals_normalize_and_rotate_without_lifting_movement() {
        for rotation in [0, 90, 180, 270] {
            for aspect in [0.5, 1.0, 2.0] {
                let mut s = state();
                s.set_aspect(aspect).unwrap();
                reports(s.key(Action::Up, true, rotation, 0));
                reports(s.key(Action::Right, true, rotation, 1));
                let sprint = reports(s.key(Action::Sprint, true, rotation, 2));
                assert_eq!(sprint.len(), 1);
                assert_eq!(sprint[0].0, TouchPhase::Move);
                let (sx, sy) = s.scale();
                let d = 0.16 / 2_f64.sqrt();
                let (x, y) = normalized_position(0.5 + d * sx, 0.5 - d * sy, rotation).unwrap();
                assert_eq!(contact(&sprint[0].1, 0), Some((true, x, y)));
                assert!(reports(s.key(Action::Sprint, true, rotation, 3)).is_empty());
            }
        }
    }

    #[test]
    fn calibrated_sprint_endpoint_overrides_multiplier_in_each_aspect_and_rotation() {
        for aspect in [0.5, 1.0, 2.0] {
            for rotation in [0, 90, 180, 270] {
                let mut p = profile();
                p.set_point(Target::Sprint, Point { x: 0.5, y: 0.15 })
                    .unwrap();
                let mut s = GameState::new(p).unwrap();
                s.set_aspect(aspect).unwrap();
                reports(s.key(Action::Up, true, rotation, 0));
                let sprint = reports(s.key(Action::Sprint, true, rotation, 1));
                let (x, y) = normalized_position(0.5, 0.15, rotation).unwrap();
                assert_eq!(contact(&sprint[0].1, 0), Some((true, x, y)));
                let walking = reports(s.key(Action::Sprint, false, rotation, 2));
                assert_ne!(contact(&walking[0].1, 0), Some((true, x, y)));
            }
        }
        let mut p = profile();
        p.set_point(Target::Sprint, Point { x: 0.5, y: 0.6 })
            .unwrap();
        assert!(GameState::new(p).is_err());
    }

    #[test]
    fn old_profiles_work_and_unmapped_extra_actions_never_guess_a_touch() {
        let mut p = Profile::default();
        for target in &CALIBRATION_TARGETS[..REQUIRED_TARGET_COUNT] {
            p.set_point(*target, Point { x: 0.5, y: 0.5 }).unwrap();
        }
        assert!(p.calibrated());
        let mut s = GameState::new(p).unwrap();
        for action in [
            Action::Vault,
            Action::Rappel,
            Action::SecondaryGadget,
            Action::Mount,
        ] {
            let batch = s.key(action, true, 0, 0);
            assert!(matches!(batch.error, Some(GameError::Uncalibrated(_))));
            assert!(batch.events.iter().all(Option::is_none));
            assert!(reports(s.key(action, false, 0, 1)).is_empty());
        }
    }

    #[test]
    fn extra_buttons_hold_distinct_targets_and_release_without_disturbing_movement() {
        let mut p = profile();
        let actions = [Action::Vault, Action::Rappel, Action::SecondaryGadget];
        for (i, action) in actions.iter().enumerate() {
            p.set_point(
                Target::Button(*action),
                Point {
                    x: 0.2 + i as f64 * 0.2,
                    y: 0.7,
                },
            )
            .unwrap();
        }
        let mut s = GameState::new(p).unwrap();
        reports(s.key(Action::Up, true, 0, 0));
        reports(s.motion(1.0, 0.0, 0, 1));
        for (i, action) in actions.iter().enumerate() {
            let down = reports(s.key(*action, true, 0, 2));
            let (x, y) = normalized_position(0.2 + i as f64 * 0.2, 0.7, 0).unwrap();
            assert_eq!(contact(&down[0].1, i as u8 + 2), Some((true, x, y)));
        }
        let released = reports(s.key(Action::SecondaryGadget, false, 0, 3));
        assert!(contact(&released[0].1, 0).unwrap().0);
        assert!(contact(&released[0].1, 1).unwrap().0);
        assert!(!contact(&released[0].1, 4).unwrap().0);
    }

    #[test]
    fn mount_holds_and_releases_its_target_while_movement_and_look_continue() {
        let mut p = profile();
        p.set_point(Target::Button(Action::Mount), Point { x: 0.7, y: 0.4 })
            .unwrap();
        let mut s = GameState::new(p).unwrap();
        reports(s.key(Action::Up, true, 0, 0));
        let looking = reports(s.motion(1.0, 0.0, 0, 1));
        let movement = contact(&looking.last().unwrap().1, 0);
        let look = contact(&looking.last().unwrap().1, 1);
        let down = reports(s.key(Action::Mount, true, 0, 2));
        let (x, y) = normalized_position(0.7, 0.4, 0).unwrap();
        assert_eq!(contact(&down[0].1, 2), Some((true, x, y)));
        assert_eq!(contact(&down[0].1, 0), movement);
        assert_eq!(contact(&down[0].1, 1), look);
        assert!(reports(s.key(Action::Mount, true, 0, 3)).is_empty());
        let up = reports(s.key(Action::Mount, false, 0, 4));
        assert_eq!(contact(&up[0].1, 2), Some((false, x, y)));
        assert_eq!(contact(&up[0].1, 0), movement);
        assert_eq!(contact(&up[0].1, 1), look);
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
            [
                TouchPhase::Move,
                TouchPhase::End,
                TouchPhase::Begin,
                TouchPhase::Move
            ]
        );
        let edge = contact(&frames[0].1, 1).unwrap();
        assert!(edge.1 > old_look.1);
        assert_eq!(contact(&frames[1].1, 1), Some((false, edge.1, edge.2)));
        assert_eq!(contact(&frames[2].1, 1), Some((true, 32768, 32768)));
        for (_, frame) in frames {
            assert_eq!(contact(&frame, 2), Some(old_fire));
        }
        assert!(reports(s.motion(f64::NAN, 0.0, 0, 3)).is_empty());
        let large = reports(s.motion(f64::MAX, -f64::MAX, 0, 4));
        let (_, x, y) = contact(&large.last().unwrap().1, 1).unwrap();
        assert!((f64::from(x) / 65535.0 - 0.66).abs() < 0.0001);
        assert!((f64::from(y) / 65535.0 - 0.34).abs() < 0.0001);
    }

    fn rotated_vector(x: f64, y: f64, rotation: u16) -> (f64, f64) {
        match rotation {
            90 => (y, -x),
            180 => (-x, -y),
            270 => (-y, x),
            _ => (x, y),
        }
    }

    #[test]
    fn look_vectors_conserve_displacement_across_groupings_aspects_and_rotations() {
        for aspect in [0.01, 0.5, 1.0, 2.0, 100.0] {
            for rotation in [0, 90, 180, 270] {
                for (dx, dy) in [
                    (80.0, 0.0),
                    (81.0, 0.0),
                    (-220.0, 0.0),
                    (0.0, 220.0),
                    (220.0, 73.0),
                    (-73.0, 220.0),
                    (160.0, 160.0),
                    (-160.0, -160.0),
                ] {
                    for groups in [1, 2, 5, 20] {
                        let mut s = state();
                        s.set_aspect(aspect).unwrap();
                        let (sx, sy) = s.scale();
                        let expected = rotated_vector(
                            dx * 0.002 * sx * 65535.0,
                            dy * 0.002 * sy * 65535.0,
                            rotation,
                        );
                        let mut segments = 1;
                        let batches: Vec<_> = (0..groups)
                            .map(|timestamp| {
                                let batch = s.motion(
                                    dx / f64::from(groups),
                                    dy / f64::from(groups),
                                    rotation,
                                    timestamp as u64,
                                );
                                assert!(!batch.clipped_motion);
                                segments += batch.look_resets;
                                batch
                            })
                            .collect();
                        let actual = emitted_look_displacement(batches);
                        // Moves within one contact telescope. Each independent
                        // segment has two rounded endpoints, hence <1 code of
                        // error per axis, regardless of raw event count.
                        let tolerance = f64::from(segments) + 1e-8;
                        assert!(
                            (actual.0 as f64 - expected.0).abs() <= tolerance,
                            "x {actual:?} vs {expected:?}, {segments} segments, {aspect}, {rotation}, {groups}"
                        );
                        assert!(
                            (actual.1 as f64 - expected.1).abs() <= tolerance,
                            "y {actual:?} vs {expected:?}, {segments} segments, {aspect}, {rotation}, {groups}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn look_edge_segments_keep_full_vector_direction() {
        for (dx, dy) in [(220.0, 73.0), (-73.0, 220.0), (160.0, 160.0)] {
            let mut s = state();
            let mut previous = None;
            let batch = s.motion(dx, dy, 0, 0);
            assert!(batch.look_resets > 0);
            for (phase, report) in reports(batch) {
                assert_ne!(phase, TouchPhase::AnchorBegin);
                match contact(&report, 1) {
                    Some((true, x, y)) => {
                        if let Some((px, py)) = previous {
                            let vx = f64::from(x) - f64::from(px);
                            let vy = f64::from(y) - f64::from(py);
                            assert!(vx * dx >= 0.0 && vy * dy >= 0.0);
                            // Each axis displacement has <1 code rounding error.
                            assert!((vx * dy - vy * dx).abs() <= dx.abs() + dy.abs());
                        }
                        previous = Some((x, y));
                    }
                    _ => previous = None,
                }
            }
        }
    }

    #[test]
    fn look_reverses_from_boundary_without_unnecessary_reset() {
        let mut s = state();
        let edge = s.motion(80.0, 0.0, 0, 0);
        assert_eq!(edge.look_resets, 0);
        let reverse = s.motion(-40.0, 0.0, 0, 1);
        assert_eq!(reverse.look_resets, 0);
        assert_eq!(reports(reverse).len(), 1);
        let onward = s.motion(80.0, 0.0, 0, 2);
        assert_eq!(onward.look_resets, 1);
        assert!(!onward.clipped_motion);

        let mut s = state();
        let mut segments = 1;
        let batches: Vec<_> = [(120.0, 40.0), (-240.0, -80.0), (120.0, 40.0)]
            .into_iter()
            .map(|(dx, dy)| {
                let batch = s.motion(dx, dy, 0, 0);
                segments += batch.look_resets;
                assert!(!batch.clipped_motion);
                batch
            })
            .collect();
        let total = emitted_look_displacement(batches);
        assert!(total.0.abs() <= i64::from(segments));
        assert!(total.1.abs() <= i64::from(segments));
    }

    #[test]
    fn look_repeated_resets_have_only_per_contact_quantization_error() {
        for rotation in [0, 90, 180, 270] {
            let mut s = state();
            let mut segments = 1;
            let batches: Vec<_> = (0..1000)
                .map(|timestamp| {
                    let batch = s.motion(100.0, 37.0, rotation, timestamp);
                    assert!(!batch.clipped_motion);
                    segments += batch.look_resets;
                    batch
                })
                .collect();
            let actual = emitted_look_displacement(batches);
            let expected = rotated_vector(
                100_000.0 * 0.002 * 65535.0,
                37_000.0 * 0.002 * 65535.0,
                rotation,
            );
            assert!((actual.0 as f64 - expected.0).abs() <= f64::from(segments));
            assert!((actual.1 as f64 - expected.1).abs() <= f64::from(segments));
        }
    }

    #[test]
    fn look_keeps_held_contacts_and_neutral_joystick_then_releases_without_tail() {
        let mut s = state();
        reports(s.key(Action::Up, true, 0, 0));
        reports(s.key(Action::LeanLeft, true, 0, 1));
        let held = reports(s.key(Action::Fire, true, 0, 2))[0].1;
        let motion = s.motion(300.0, -170.0, 0, 3);
        assert!(!motion.clipped_motion);
        assert!(motion.look_resets > 0);
        for (_, report) in reports(motion) {
            for id in [0, 2, 3] {
                assert_eq!(contact(&report, id), contact(&held, id));
            }
        }
        let neutral = reports(s.key(Action::Up, false, 0, 4))[0].1;
        assert_eq!(contact(&neutral, 0), Some((true, 32768, 32768)));
        for (_, report) in reports(s.motion(400.0, 0.0, 0, 5)) {
            for id in [0, 2, 3] {
                assert_eq!(contact(&report, id), contact(&neutral, id));
            }
        }
        let released = reports(s.release_all(6));
        assert_eq!(released.len(), 1);
        for id in [0, 1, 2, 3] {
            assert!(!contact(&released[0].1, id).unwrap().0);
        }
        assert!(reports(s.motion(0.0, 0.0, 0, 7)).is_empty());
        assert!(reports(s.key(Action::Up, false, 0, 8)).is_empty());
        let next = reports(s.motion(1.0, 0.0, 0, 9));
        assert_eq!(next.len(), 2);
        assert_eq!(next[0].1[1], 1);
        assert_eq!(contact(&next[0].1, 1), Some((true, 32768, 32768)));
    }

    #[test]
    fn extreme_and_edge_look_motion_is_bounded_and_reports_clipping() {
        for center in [
            Point { x: 0.5, y: 0.5 },
            Point { x: 0.0, y: 0.0 },
            Point { x: 1.0, y: 1.0 },
            Point {
                x: 0.00001,
                y: 0.99999,
            },
        ] {
            for aspect in [0.01, 1.0, 100.0] {
                for rotation in [0, 90, 180, 270] {
                    for (dx, dy) in [
                        (f64::MAX, f64::MAX),
                        (-f64::MAX, f64::MAX),
                        (f64::MAX, -f64::MAX),
                        (-f64::MAX, -f64::MAX),
                    ] {
                        let mut p = profile();
                        p.set_point(Target::Look, center).unwrap();
                        let mut s = GameState::new(p).unwrap();
                        s.set_aspect(aspect).unwrap();
                        reports(s.key(Action::Fire, true, 0, 0));
                        let batch = s.motion(dx, dy, rotation, 1);
                        assert!(batch.clipped_motion);
                        assert!(batch.look_resets <= 7);
                        assert!(reports(batch).len() <= 24);
                        // No clipped displacement is deferred to a later event.
                        assert!(reports(s.motion(0.0, 0.0, rotation, 2)).is_empty());
                        reports(s.release_all(3));
                        assert!(reports(s.release_all(4)).is_empty());
                    }
                }
            }
        }
        let mut p = profile();
        p.set_point(Target::Look, Point { x: 0.0, y: 0.0 }).unwrap();
        let mut s = GameState::new(p).unwrap();
        let blocked = s.motion(-1.0, 1.0, 0, 0);
        assert!(blocked.clipped_motion);
        assert_eq!(blocked.look_resets, 0);
        assert!(reports(blocked).is_empty());
        let inward = s.motion(100.0, 50.0, 0, 1);
        assert!(!inward.clipped_motion);
        let actual = emitted_look_displacement([inward]);
        assert!((actual.0 as f64 - 0.2 * 65535.0).abs() <= 2.0);
        assert!((actual.1 as f64 - 0.1 * 65535.0).abs() <= 2.0);
    }

    #[test]
    fn invalid_look_motion_preserves_contacts_and_flags_invalid_input() {
        let mut s = state();
        let first = reports(s.motion(10.0, 0.0, 0, 0));
        let old = contact(&first[1].1, 1).unwrap();
        for (dx, dy) in [
            (f64::NAN, 0.0),
            (0.0, f64::INFINITY),
            (f64::NEG_INFINITY, 1.0),
        ] {
            let batch = s.motion(dx, dy, 0, 1);
            assert!(batch.clipped_motion);
            assert_eq!(batch.look_resets, 0);
            assert!(reports(batch).is_empty());
        }
        let zero = s.motion(0.0, 0.0, 0, 2);
        assert!(!zero.clipped_motion);
        assert!(reports(zero).is_empty());
        let release = reports(s.release_all(3));
        assert_eq!(contact(&release[0].1, 1), Some((false, old.1, old.2)));
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
            Action::Sprint,
            Action::Vault,
            Action::Rappel,
            Action::SecondaryGadget,
            Action::Mount,
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
