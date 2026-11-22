use crate::galaxy::{galaxy_at, Galaxy, GalaxyId, GALAXY_RADIUS};
use glam::Vec3;

/// Pointer travel (in pixels) the chord must be dragged before a warp can fire.
/// Pressing both buttons by accident without dragging never leaves the galaxy.
pub const ARM_DISTANCE_PX: f32 = 48.0;

/// Extra room around a galaxy halo that still counts as aiming at it.
const AIM_TOLERANCE: f32 = 1.15;

/// Smallest aim cone, so distant galaxies stay easy to hit.
const MIN_AIM_CONE_RADIANS: f32 = 0.14;

/// A space warp requested by the chord gesture.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct WarpJump {
    pub destination: GalaxyId,
    /// Window carried along: the chord started while that window was being dragged.
    pub cargo: Option<usize>,
}

/// State of the left + right mouse chord used for intergalactic travel.
///
/// Holding both buttons and dragging turns the view as usual. The galaxy whose halo
/// sits under the center of the view becomes the warp target, and releasing either
/// button jumps there. Releasing without a target, or pressing Escape, cancels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WarpGesture {
    active: bool,
    drag_px: f32,
    target: Option<GalaxyId>,
    cargo: Option<usize>,
}

impl WarpGesture {
    /// Starts the chord. `cargo` is the window being dragged when the chord began.
    pub fn begin(&mut self, cargo: Option<usize>) {
        *self = Self {
            active: true,
            cargo,
            ..Self::default()
        };
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Adds pointer movement to the chord.
    pub fn add_drag(&mut self, dx: f32, dy: f32) {
        if self.active {
            self.drag_px += (dx * dx + dy * dy).sqrt();
        }
    }

    /// Whether the chord has been dragged far enough to fire.
    pub fn is_armed(&self) -> bool {
        self.active && self.drag_px >= ARM_DISTANCE_PX
    }

    /// Updates the aimed galaxy. Returns `true` when the target changed.
    pub fn set_target(&mut self, target: Option<GalaxyId>) -> bool {
        let changed = self.target != target;
        self.target = target;
        changed
    }

    pub fn cargo(&self) -> Option<usize> {
        self.cargo
    }

    /// Ends the chord. Returns the jump to perform when the gesture was armed and aimed.
    pub fn release(&mut self) -> Option<WarpJump> {
        let jump = if self.is_armed() {
            self.target.map(|destination| WarpJump {
                destination,
                cargo: self.cargo,
            })
        } else {
            None
        };
        self.cancel();
        jump
    }

    /// Abandons the chord without traveling.
    pub fn cancel(&mut self) {
        *self = Self::default();
    }
}

/// The galaxy whose halo lies closest to the line of sight `(eye, forward)`.
///
/// The galaxy around the eye is skipped: a warp always leaves it. A galaxy counts as
/// aimed at when the angle to its center is within the angular size of its halo.
pub fn aim_target(galaxies: &[Galaxy], eye: Vec3, forward: Vec3) -> Option<GalaxyId> {
    let forward = forward.normalize_or_zero();
    let home = galaxy_at(galaxies, eye);
    galaxies
        .iter()
        .filter(|galaxy| Some(galaxy.id) != home)
        .filter_map(|galaxy| {
            let to_center = galaxy.center - eye;
            let distance = to_center.length();
            if distance <= f32::EPSILON {
                return None;
            }
            let angle = forward.dot(to_center / distance).clamp(-1.0, 1.0).acos();
            let cone = (GALAXY_RADIUS * AIM_TOLERANCE / distance)
                .min(1.0)
                .asin()
                .max(MIN_AIM_CONE_RADIANS);
            (angle <= cone).then_some((galaxy.id, angle))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::galaxy::{demo_universe, PRIME_GALAXY};

    #[test]
    fn test_chord_without_drag_never_warps() {
        let mut gesture = WarpGesture::default();
        gesture.begin(None);
        gesture.set_target(Some(1));
        gesture.add_drag(3.0, 4.0);
        assert!(!gesture.is_armed());
        assert_eq!(gesture.release(), None);
        assert!(!gesture.is_active());
    }

    #[test]
    fn test_armed_chord_warps_to_target_with_cargo() {
        let mut gesture = WarpGesture::default();
        gesture.begin(Some(4));
        gesture.add_drag(30.0, 40.0);
        assert!(gesture.is_armed());
        assert!(gesture.set_target(Some(2)));
        assert!(!gesture.set_target(Some(2)));
        assert_eq!(
            gesture.release(),
            Some(WarpJump {
                destination: 2,
                cargo: Some(4)
            })
        );
        assert_eq!(gesture, WarpGesture::default());
    }

    #[test]
    fn test_armed_chord_without_target_or_cancelled_does_not_warp() {
        let mut gesture = WarpGesture::default();
        gesture.begin(None);
        gesture.add_drag(100.0, 0.0);
        assert_eq!(gesture.release(), None);

        gesture.begin(None);
        gesture.add_drag(100.0, 0.0);
        gesture.set_target(Some(1));
        gesture.cancel();
        assert_eq!(gesture.release(), None);
    }

    #[test]
    fn test_drag_is_ignored_while_idle() {
        let mut gesture = WarpGesture::default();
        gesture.add_drag(500.0, 0.0);
        gesture.begin(None);
        assert!(!gesture.is_armed());
    }

    #[test]
    fn test_aim_target_picks_the_galaxy_in_front() {
        let galaxies = demo_universe();
        let eye = galaxies[PRIME_GALAXY].center;
        for galaxy in &galaxies[1..] {
            let forward = galaxy.center - eye;
            assert_eq!(aim_target(&galaxies, eye, forward), Some(galaxy.id));
        }
        // Looking straight down from the core of the universe hits nothing.
        assert_eq!(aim_target(&galaxies, eye, Vec3::NEG_Y), None);
    }

    #[test]
    fn test_aim_target_skips_the_galaxy_around_the_eye() {
        let galaxies = demo_universe();
        let eye = galaxies[1].center + Vec3::new(0.0, 0.0, 5.0);
        // Looking at its own center from inside Andromeda is not a warp target.
        assert_eq!(aim_target(&galaxies, eye, Vec3::NEG_Z), None);
        // From deep space, aiming back at Prime works.
        let deep = Vec3::new(0.0, -100.0, 0.0);
        assert_eq!(aim_target(&galaxies, deep, Vec3::Y), Some(PRIME_GALAXY));
    }
}
