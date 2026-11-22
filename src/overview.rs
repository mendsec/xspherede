use crate::galaxy::{Galaxy, GalaxyId, GALAXY_RADIUS};
use glam::Vec3;
use std::collections::BTreeMap;

/// Pitch limit of the overview orbit, so the view never flips over a pole.
const MAX_ORBIT_PITCH: f32 = 1.25;
/// Orbit turn per pixel of drag, in radians.
const ORBIT_SENSITIVITY: f32 = 0.005;
/// Zoom range, as fractions of the distance that frames the whole universe.
const MIN_ZOOM: f32 = 0.55;
const MAX_ZOOM: f32 = 1.6;
/// Extra room around the galaxies when framing them.
const FRAME_MARGIN: f32 = 1.12;

/// The Cosmic Zoom-Out: the camera pulls out of the current galaxy until every galaxy
/// is in view, then orbits the universe. Galaxies are drawn with their live windows,
/// and windows of the same trust domain are linked across the universe, in the spirit
/// of a knowledge graph.
#[derive(Clone, Debug, PartialEq)]
pub struct CosmicOverview {
    active: bool,
    /// Point the orbit turns around: the center of the galaxies.
    pub focus: Vec3,
    /// Orbit direction, using the camera's yaw and pitch conventions.
    pub yaw: f32,
    pub pitch: f32,
    /// Distance that frames every galaxy for the current field of view.
    pub frame_distance: f32,
    /// Zoom factor applied to `frame_distance`.
    pub zoom: f32,
}

impl Default for CosmicOverview {
    fn default() -> Self {
        Self {
            active: false,
            focus: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            frame_distance: 1.0,
            zoom: 1.0,
        }
    }
}

impl CosmicOverview {
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Opens the overview looking along `(yaw, pitch)`, the viewer's current direction,
    /// so the zoom-out feels like pulling straight back from where you were looking.
    pub fn open(&mut self, galaxies: &[Galaxy], yaw: f32, pitch: f32, fov_y: f32, aspect: f32) {
        let (focus, radius) = galaxy_bounds(galaxies);
        *self = Self {
            active: true,
            focus,
            yaw,
            pitch: pitch.clamp(-MAX_ORBIT_PITCH, MAX_ORBIT_PITCH),
            frame_distance: framing_distance(radius, fov_y, aspect),
            zoom: 1.0,
        };
    }

    pub fn close(&mut self) {
        self.active = false;
    }

    /// Turns the orbit by a pointer drag.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw += dx * ORBIT_SENSITIVITY;
        self.pitch = (self.pitch - dy * ORBIT_SENSITIVITY).clamp(-MAX_ORBIT_PITCH, MAX_ORBIT_PITCH);
    }

    /// Zooms in (positive `steps`) or out (negative).
    pub fn zoom_by(&mut self, steps: f32) {
        self.zoom = (self.zoom * 0.9_f32.powf(steps)).clamp(MIN_ZOOM, MAX_ZOOM);
    }

    /// Direction the overview camera looks in.
    pub fn forward(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        Vec3::new(sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch)
    }

    /// Where the overview camera sits.
    pub fn eye(&self) -> Vec3 {
        self.focus - self.forward() * self.frame_distance * self.zoom
    }

    /// Farthest the overview camera can be from the universe core.
    pub fn max_reach(&self) -> f32 {
        self.focus.length() + self.frame_distance * MAX_ZOOM
    }
}

/// Center and radius of a sphere that holds every galaxy's halo.
pub fn galaxy_bounds(galaxies: &[Galaxy]) -> (Vec3, f32) {
    if galaxies.is_empty() {
        return (Vec3::ZERO, GALAXY_RADIUS);
    }
    let center = galaxies.iter().map(|g| g.center).sum::<Vec3>() / galaxies.len() as f32;
    let radius = galaxies
        .iter()
        .map(|g| g.center.distance(center) + GALAXY_RADIUS)
        .fold(GALAXY_RADIUS, f32::max);
    (center, radius)
}

/// Distance at which a sphere of `radius` fits in a view with this field of view.
pub fn framing_distance(radius: f32, fov_y: f32, aspect: f32) -> f32 {
    let half_y = fov_y * 0.5;
    let half_x = (half_y.tan() * aspect.max(1e-3)).atan();
    let half = half_y.min(half_x).max(0.05);
    radius * FRAME_MARGIN / half.sin()
}

/// The galaxy whose halo the ray hits first.
pub fn pick_galaxy(galaxies: &[Galaxy], origin: Vec3, dir: Vec3) -> Option<GalaxyId> {
    let dir = dir.normalize_or_zero();
    galaxies
        .iter()
        .filter_map(|galaxy| {
            let to_origin = origin - galaxy.center;
            let b = to_origin.dot(dir);
            let c = to_origin.length_squared() - GALAXY_RADIUS * GALAXY_RADIUS;
            let disc = b * b - c;
            if disc < 0.0 {
                return None;
            }
            let t = -b - disc.sqrt();
            let t = if t >= 0.0 { t } else { -b + disc.sqrt() };
            (t >= 0.0).then_some((galaxy.id, t))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// Links between windows that share a trust domain, like the edges of a graph view.
///
/// `nodes` holds each window's `(domain, galaxy)`. The windows of one domain are chained
/// in galaxy order, so a domain spread over several galaxies draws a visible path
/// between them. Returns pairs of indices into `nodes`.
pub fn trust_links<K: Ord + Copy>(nodes: &[(K, GalaxyId)]) -> Vec<(usize, usize)> {
    let mut by_domain: BTreeMap<K, Vec<usize>> = BTreeMap::new();
    for (index, (domain, _)) in nodes.iter().enumerate() {
        by_domain.entry(*domain).or_default().push(index);
    }
    let mut links = Vec::new();
    for members in by_domain.values_mut() {
        members.sort_by_key(|&index| (nodes[index].1, index));
        links.extend(members.windows(2).map(|pair| (pair[0], pair[1])));
    }
    links
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::galaxy::demo_universe;

    #[test]
    fn test_overview_frames_every_galaxy() {
        let galaxies = demo_universe();
        let mut overview = CosmicOverview::default();
        overview.open(&galaxies, 0.4, 0.2, 70_f32.to_radians(), 16.0 / 9.0);
        assert!(overview.is_active());
        let eye = overview.eye();
        let forward = overview.forward();
        let half_fov = 35_f32.to_radians();
        for galaxy in &galaxies {
            let to_galaxy = galaxy.center - eye;
            let angle = forward.angle_between(to_galaxy);
            let halo = (GALAXY_RADIUS / to_galaxy.length()).asin();
            assert!(
                angle + halo < half_fov,
                "{} is outside the overview",
                galaxy.name
            );
        }
        assert!(eye.length() <= overview.max_reach());
    }

    #[test]
    fn test_overview_pulls_straight_back_along_the_view() {
        let galaxies = demo_universe();
        let mut overview = CosmicOverview::default();
        overview.open(&galaxies, 1.0, 0.3, 1.2, 1.5);
        let back = (overview.focus - overview.eye()).normalize();
        assert!(back.distance(overview.forward()) < 1e-4);
    }

    #[test]
    fn test_orbit_and_zoom_stay_in_range() {
        let mut overview = CosmicOverview::default();
        overview.orbit(0.0, -10_000.0);
        assert!(overview.pitch <= MAX_ORBIT_PITCH);
        overview.zoom_by(100.0);
        assert_eq!(overview.zoom, MIN_ZOOM);
        overview.zoom_by(-100.0);
        assert_eq!(overview.zoom, MAX_ZOOM);
    }

    #[test]
    fn test_pick_galaxy_hits_the_nearest_halo() {
        let galaxies = demo_universe();
        let target = galaxies[1].center;
        let origin = target + Vec3::new(0.0, 0.0, 200.0);
        assert_eq!(pick_galaxy(&galaxies, origin, Vec3::NEG_Z), Some(1));
        assert_eq!(pick_galaxy(&galaxies, origin, Vec3::Z), None);
    }

    #[test]
    fn test_trust_links_chain_each_domain_across_galaxies() {
        // Domains: 'a' in galaxies 2, 0, 1; 'b' alone; 'c' twice in galaxy 3.
        let nodes = [('a', 2), ('b', 0), ('a', 0), ('c', 3), ('a', 1), ('c', 3)];
        let links = trust_links(&nodes);
        assert_eq!(links, vec![(2, 4), (4, 0), (3, 5)]);
    }
}
