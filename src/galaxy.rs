use glam::Vec3;

/// Identifier of a galaxy inside the XSphereDE universe.
pub type GalaxyId = usize;

/// The galaxy every session starts in, centered at the core of the universe.
pub const PRIME_GALAXY: GalaxyId = 0;

/// Radius of the universe: the outer wireframe sphere that bounds everything.
pub const UNIVERSE_RADIUS: f32 = 160.0;

/// Radius of a galaxy's halo: the local wireframe sphere around its windows.
/// Window depths (4.0 m to 24.5 m) are measured inside this radius.
pub const GALAXY_RADIUS: f32 = 26.0;

/// A cluster of windows inside the XSphereDE universe.
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

#[cfg(test)]
mod tests {
    use super::*;

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
