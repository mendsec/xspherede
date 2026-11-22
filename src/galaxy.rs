use glam::Vec3;

/// Identifier of a galaxy inside the Spheradian universe.
pub type GalaxyId = usize;

/// The galaxy every session starts in, centered at the core of the universe.
pub const PRIME_GALAXY: GalaxyId = 0;

/// Radius of the universe: the outer wireframe sphere that bounds everything.
pub const UNIVERSE_RADIUS: f32 = 160.0;

/// Radius of a galaxy's halo: the local wireframe sphere around its windows.
/// Window depths (4.0 m to 24.5 m) are measured inside this radius.
pub const GALAXY_RADIUS: f32 = 26.0;

/// A cluster of windows inside the Spheradian universe.
///
/// The large sphere that surrounds the viewer is the shape of the universe: finite,
/// closed, and without an edge. Windows do not float freely in it; they are grouped
/// into galaxies. Every galaxy has its own center, and the spherical coordinates
/// `(azimuth, elevation, radius)` of its windows are measured from that center rather
/// than from the core of the universe.
pub struct Galaxy {
    pub id: GalaxyId,
    pub name: String,
    /// Center of the galaxy in universe (world) coordinates.
    pub center: Vec3,
    /// Color of the galaxy's halo grid, or `None` to keep the classic Matrix green.
    /// Halo tints are pale on purpose so they never read as trust-domain colors.
    pub halo_tint: Option<[f32; 3]>,
}

impl Galaxy {
    pub fn new(id: GalaxyId, name: impl Into<String>, center: Vec3) -> Self {
        Self {
            id,
            name: name.into(),
            center,
            halo_tint: None,
        }
    }

    pub fn with_halo_tint(mut self, tint: [f32; 3]) -> Self {
        self.halo_tint = Some(tint);
        self
    }

    /// The galaxy at the core of the universe, where every session begins.
    pub fn prime() -> Self {
        Self::new(PRIME_GALAXY, "Prime", Vec3::ZERO)
    }

    /// Whether `point` (universe coordinates) lies inside this galaxy's halo.
    pub fn contains(&self, point: Vec3) -> bool {
        self.center.distance_squared(point) <= GALAXY_RADIUS * GALAXY_RADIUS
    }
}

/// The demo universe of the prototype: the Prime galaxy at the core and three
/// more galaxies spread through the universe. Ids match their indices.
pub fn demo_universe() -> Vec<Galaxy> {
    vec![
        Galaxy::prime(),
        Galaxy::new(1, "Andromeda", Vec3::new(92.0, 14.0, -48.0)).with_halo_tint([0.55, 0.72, 1.0]),
        Galaxy::new(2, "Triangulum", Vec3::new(-86.0, -20.0, -62.0))
            .with_halo_tint([1.0, 0.86, 0.52]),
        Galaxy::new(3, "Sombrero", Vec3::new(10.0, 30.0, 100.0)).with_halo_tint([1.0, 0.62, 0.82]),
    ]
}

/// The galaxy whose halo contains `point`, choosing the closest center when halos overlap.
pub fn galaxy_at(galaxies: &[Galaxy], point: Vec3) -> Option<GalaxyId> {
    galaxies
        .iter()
        .filter(|galaxy| galaxy.contains(point))
        .min_by(|a, b| {
            a.center
                .distance_squared(point)
                .total_cmp(&b.center.distance_squared(point))
        })
        .map(|galaxy| galaxy.id)
}

/// The galaxy `step` positions away from `current`, wrapping around in both directions.
pub fn cycle_galaxy(current: GalaxyId, count: usize, step: isize) -> GalaxyId {
    if count == 0 {
        return current;
    }
    let count = count as isize;
    (current as isize + step).rem_euclid(count) as GalaxyId
}

/// Converts spherical coordinates around a galaxy center into a local Cartesian offset.
///
/// `azimuth = 0` points toward `-Z`, positive azimuth turns toward `+X`, and positive
/// elevation points toward `+Y`, matching the camera's yaw and pitch conventions.
pub fn spherical_to_local(azimuth: f32, elevation: f32, radius: f32) -> Vec3 {
    let (sin_phi, cos_phi) = elevation.sin_cos();
    let (sin_theta, cos_theta) = azimuth.sin_cos();
    Vec3::new(
        radius * cos_phi * sin_theta,
        radius * sin_phi,
        -radius * cos_phi * cos_theta,
    )
}

/// How a galaxy's halo grid is drawn.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HaloStyle {
    Hidden,
    /// A faint, sparse outline that only shows where the galaxy is.
    Beacon,
    /// The full halo grid.
    Full,
}

/// How much of a galaxy is drawn in the main view.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct GalaxyVisibility {
    pub halo: HaloStyle,
    pub windows: bool,
    pub rain: bool,
}

impl GalaxyVisibility {
    /// Not drawn at all.
    pub const HIDDEN: Self = Self::new(HaloStyle::Hidden, false, false);
    /// Only a faint outline.
    pub const BEACON: Self = Self::new(HaloStyle::Beacon, false, false);
    /// The full halo, without windows or code rain.
    pub const HALO: Self = Self::new(HaloStyle::Full, false, false);
    /// Halo, windows, and code rain: the galaxy you are in.
    pub const POPULATED: Self = Self::new(HaloStyle::Full, true, true);
    /// Halo and windows without code rain: a galaxy seen from the overview.
    pub const MAPPED: Self = Self::new(HaloStyle::Full, true, false);
    /// A faint outline with its windows: an overview galaxy out of the spotlight.
    pub const DIMMED: Self = Self::new(HaloStyle::Beacon, true, false);

    const fn new(halo: HaloStyle, windows: bool, rain: bool) -> Self {
        Self {
            halo,
            windows,
            rain,
        }
    }
}

/// What the viewer needs to see of the universe right now.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewContext {
    /// The galaxy the viewer works in.
    pub current: GalaxyId,
    /// The galaxy whose halo contains the camera, if any.
    pub home: Option<GalaxyId>,
    /// Whether the space warp chord is held.
    pub aiming: bool,
    /// The galaxy the warp chord is locked on.
    pub aim: Option<GalaxyId>,
    /// The galaxy being flown to.
    pub destination: Option<GalaxyId>,
    /// Whether the Cosmic Zoom-Out overview is open.
    pub overview: bool,
    /// The galaxy under the pointer in the overview; the others are dimmed.
    pub spotlight: Option<GalaxyId>,
}

impl ViewContext {
    /// Whether the rest of the universe should be revealed. Inside a galaxy it is not:
    /// other galaxies only appear in the overview, while aiming a warp, while flying,
    /// or in deep space.
    pub fn reveals_universe(&self) -> bool {
        self.overview
            || self.aiming
            || self.destination.is_some()
            || self.home != Some(self.current)
    }

    /// Visibility of every galaxy, indexed by id.
    pub fn galaxy_visibility(&self, count: usize) -> Vec<GalaxyVisibility> {
        let reveal = self.reveals_universe();
        (0..count)
            .map(|id| {
                if self.overview {
                    match self.spotlight {
                        Some(spot) if spot != id => GalaxyVisibility::DIMMED,
                        _ => GalaxyVisibility::MAPPED,
                    }
                } else if id == self.current || Some(id) == self.destination {
                    GalaxyVisibility::POPULATED
                } else if self.aiming && Some(id) == self.aim {
                    GalaxyVisibility::HALO
                } else if reveal {
                    GalaxyVisibility::BEACON
                } else {
                    GalaxyVisibility::HIDDEN
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inside_a_galaxy_only_that_galaxy_is_visible() {
        let view = ViewContext {
            current: 1,
            home: Some(1),
            ..ViewContext::default()
        };
        assert!(!view.reveals_universe());
        assert_eq!(
            view.galaxy_visibility(3),
            vec![
                GalaxyVisibility::HIDDEN,
                GalaxyVisibility::POPULATED,
                GalaxyVisibility::HIDDEN
            ]
        );
    }

    #[test]
    fn test_aiming_reveals_beacons_and_the_locked_halo() {
        let view = ViewContext {
            current: 0,
            home: Some(0),
            aiming: true,
            aim: Some(2),
            ..ViewContext::default()
        };
        assert!(view.reveals_universe());
        assert_eq!(
            view.galaxy_visibility(3),
            vec![
                GalaxyVisibility::POPULATED,
                GalaxyVisibility::BEACON,
                GalaxyVisibility::HALO
            ]
        );
    }

    #[test]
    fn test_overview_maps_every_galaxy_and_spotlights_the_hovered_one() {
        let mut view = ViewContext {
            current: 0,
            home: None,
            overview: true,
            ..ViewContext::default()
        };
        assert_eq!(view.galaxy_visibility(2), vec![GalaxyVisibility::MAPPED; 2]);
        view.spotlight = Some(1);
        assert_eq!(
            view.galaxy_visibility(2),
            vec![GalaxyVisibility::DIMMED, GalaxyVisibility::MAPPED]
        );
        // No galaxy shows code rain in the overview.
        assert!(view.galaxy_visibility(2).iter().all(|v| !v.rain));
    }

    #[test]
    fn test_flight_and_deep_space_reveal_the_universe() {
        let flight = ViewContext {
            current: 0,
            home: None,
            destination: Some(2),
            ..ViewContext::default()
        };
        assert_eq!(
            flight.galaxy_visibility(3),
            vec![
                GalaxyVisibility::POPULATED,
                GalaxyVisibility::BEACON,
                GalaxyVisibility::POPULATED
            ]
        );
        let deep_space = ViewContext {
            current: 0,
            home: None,
            ..ViewContext::default()
        };
        assert!(deep_space.reveals_universe());
        assert_eq!(
            deep_space.galaxy_visibility(2),
            vec![GalaxyVisibility::POPULATED, GalaxyVisibility::BEACON]
        );
    }

    #[test]
    fn test_prime_galaxy_sits_at_the_core_of_the_universe() {
        let prime = Galaxy::prime();
        assert_eq!(prime.id, PRIME_GALAXY);
        assert_eq!(prime.center, Vec3::ZERO);
        assert_eq!(prime.halo_tint, None);
    }

    #[test]
    fn test_demo_universe_galaxies_fit_and_do_not_overlap() {
        let galaxies = demo_universe();
        assert!(galaxies.len() >= 2);
        for (index, galaxy) in galaxies.iter().enumerate() {
            assert_eq!(galaxy.id, index, "galaxy ids must match their indices");
            assert!(
                galaxy.center.length() + GALAXY_RADIUS < UNIVERSE_RADIUS,
                "{} leaks out of the universe",
                galaxy.name
            );
            for other in &galaxies[index + 1..] {
                assert!(
                    galaxy.center.distance(other.center) > 2.0 * GALAXY_RADIUS,
                    "{} overlaps {}",
                    galaxy.name,
                    other.name
                );
            }
        }
        assert_eq!(galaxies[PRIME_GALAXY].center, Vec3::ZERO);
    }

    #[test]
    fn test_galaxy_at_finds_the_galaxy_around_a_point() {
        let galaxies = demo_universe();
        assert_eq!(
            galaxy_at(&galaxies, Vec3::new(3.0, -2.0, 5.0)),
            Some(PRIME_GALAXY)
        );
        let andromeda = galaxies[1].center;
        assert_eq!(
            galaxy_at(&galaxies, andromeda + Vec3::new(0.0, 10.0, 0.0)),
            Some(1)
        );
        // Deep intergalactic space belongs to no galaxy.
        assert_eq!(galaxy_at(&galaxies, Vec3::new(0.0, -120.0, 0.0)), None);
    }

    #[test]
    fn test_cycle_galaxy_wraps_both_ways() {
        assert_eq!(cycle_galaxy(0, 4, 1), 1);
        assert_eq!(cycle_galaxy(3, 4, 1), 0);
        assert_eq!(cycle_galaxy(0, 4, -1), 3);
        assert_eq!(cycle_galaxy(2, 0, 1), 2);
    }

    #[test]
    fn test_spherical_to_local_axes() {
        let front = spherical_to_local(0.0, 0.0, 10.0);
        assert!((front - Vec3::new(0.0, 0.0, -10.0)).length() < 1e-5);

        let right = spherical_to_local(std::f32::consts::FRAC_PI_2, 0.0, 10.0);
        assert!((right - Vec3::new(10.0, 0.0, 0.0)).length() < 1e-5);

        let up = spherical_to_local(0.0, std::f32::consts::FRAC_PI_2, 10.0);
        assert!((up - Vec3::new(0.0, 10.0, 0.0)).length() < 1e-5);
    }
}
