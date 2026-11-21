use glam::Vec3;

/// Identifier of a galaxy inside the XSphereDE universe.
pub type GalaxyId = usize;

/// The galaxy every session starts in, centered at the core of the universe.
pub const PRIME_GALAXY: GalaxyId = 0;

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
}

impl Galaxy {
    pub fn new(id: GalaxyId, name: impl Into<String>, center: Vec3) -> Self {
        Self {
            id,
            name: name.into(),
            center,
        }
    }

    /// The galaxy at the core of the universe, where every session begins.
    pub fn prime() -> Self {
        Self::new(PRIME_GALAXY, "Prime", Vec3::ZERO)
    }
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
