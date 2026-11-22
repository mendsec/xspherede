use glam::{Mat4, Vec3};

/// Duration bounds of an intergalactic flight, in seconds.
const MIN_FLIGHT_SECS: f32 = 0.8;
const MAX_FLIGHT_SECS: f32 = 1.6;
/// Flight speed used to scale the duration with the distance, in meters per second.
const FLIGHT_SPEED: f32 = 120.0;
/// Peak widening of the field of view in the middle of a flight (fraction of the base FOV).
const FLIGHT_FOV_KICK: f32 = 0.18;

/// An eased straight-line flight of the camera between two points of the universe.
#[derive(Clone, Copy, Debug)]
struct CameraFlight {
    from: Vec3,
    to: Vec3,
    elapsed: f32,
    duration: f32,
}

impl CameraFlight {
    fn progress(&self) -> f32 {
        (self.elapsed / self.duration.max(1e-3)).clamp(0.0, 1.0)
    }
}

/// Smootherstep easing: zero velocity and acceleration at both ends.
fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// 3D spatial camera for XSphereDE.
/// Allows both 360-degree spherical view rotation inside the sphere and
/// free 3D spatial navigation (flying through the volumetric Matrix code corridors).
pub struct SphereCamera {
    pub position: Vec3,
    pub target_position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub target_yaw: f32,
    pub target_pitch: f32,
    pub fov_y_radians: f32,
    /// Field of view at rest; `fov_y_radians` widens from it during a flight.
    pub base_fov_y_radians: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
    /// The camera never leaves this distance from the universe core.
    pub bounds_radius: f32,
    flight: Option<CameraFlight>,
}

impl SphereCamera {
    pub fn new(width: u32, height: u32) -> Self {
        let aspect = (width.max(1) as f32) / (height.max(1) as f32);
        Self {
            position: Vec3::ZERO,
            target_position: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            target_yaw: 0.0,
            target_pitch: 0.0,
            fov_y_radians: 70.0_f32.to_radians(),
            base_fov_y_radians: 70.0_f32.to_radians(),
            aspect,
            near: 0.1,
            far: 120.0,
            bounds_radius: 22.0,
            flight: None,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.aspect = (width.max(1) as f32) / (height.max(1) as f32);
    }

    /// Forward direction vector where the camera is looking in 3D space.
    pub fn forward(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        Vec3::new(sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch).normalize()
    }

    /// Camera right vector on the horizontal plane.
    pub fn right(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        Vec3::new(cos_yaw, 0.0, sin_yaw).normalize()
    }

    /// Up vector relative to the camera orientation.
    pub fn up(&self) -> Vec3 {
        self.right().cross(self.forward()).normalize()
    }

    /// Smoothly rotates the camera based on mouse delta.
    pub fn rotate(&mut self, delta_x: f32, delta_y: f32, sensitivity: f32) {
        self.target_yaw += delta_x * sensitivity;
        self.target_pitch = (self.target_pitch - delta_y * sensitivity)
            .clamp(-85.0_f32.to_radians(), 85.0_f32.to_radians());
    }

    /// Moves the camera through the universe (flying through Matrix columns).
    /// Manual movement interrupts a flight in progress, leaving the camera where it is.
    pub fn translate_local(&mut self, forward_delta: f32, right_delta: f32, up_delta: f32) {
        self.cancel_flight();
        let movement =
            self.forward() * forward_delta + self.right() * right_delta + self.up() * up_delta;
        self.target_position = self.clamp_to_bounds(self.target_position + movement);
    }

    fn clamp_to_bounds(&self, point: Vec3) -> Vec3 {
        if point.length() > self.bounds_radius {
            point.normalize_or_zero() * self.bounds_radius
        } else {
            point
        }
    }

    /// Smoothly returns to `home` (the center of the current galaxy) and levels the view.
    pub fn reset_to(&mut self, home: Vec3) {
        self.cancel_flight();
        self.target_position = self.clamp_to_bounds(home);
        self.target_pitch = 0.0;
    }

    /// Starts an eased flight to `destination` and turns toward it.
    pub fn fly_to(&mut self, destination: Vec3) {
        let to = self.clamp_to_bounds(destination);
        let distance = self.position.distance(to);
        self.look_at_point(to);
        self.flight = Some(CameraFlight {
            from: self.position,
            to,
            elapsed: 0.0,
            duration: (distance / FLIGHT_SPEED).clamp(MIN_FLIGHT_SECS, MAX_FLIGHT_SECS),
        });
    }

    pub fn is_flying(&self) -> bool {
        self.flight.is_some()
    }

    /// Stops a flight in progress where the camera currently is.
    pub fn cancel_flight(&mut self) {
        if self.flight.take().is_some() {
            self.target_position = self.position;
            self.fov_y_radians = self.base_fov_y_radians;
        }
    }

    /// Smoothly turns the camera toward a point of the universe, from wherever it is.
    pub fn look_at_point(&mut self, point: Vec3) {
        let dir = (point - self.target_position).normalize_or_zero();
        if dir == Vec3::ZERO {
            return;
        }
        let azimuth = dir.x.atan2(-dir.z);
        let elevation = dir.y.clamp(-1.0, 1.0).asin();
        self.look_at_spherical(azimuth, elevation);
    }

    /// Smoothly directs the main camera toward a spherical coordinate `(azimuth, elevation)`,
    /// always choosing the shortest angular path (avoiding unnecessary 360-degree spins).
    pub fn look_at_spherical(&mut self, target_azimuth: f32, target_elevation: f32) {
        let mut diff_yaw = target_azimuth - self.yaw;
        while diff_yaw > std::f32::consts::PI {
            diff_yaw -= std::f32::consts::TAU;
        }
        while diff_yaw < -std::f32::consts::PI {
            diff_yaw += std::f32::consts::TAU;
        }
        self.target_yaw = self.yaw + diff_yaw;
        self.target_pitch = target_elevation.clamp(-85.0_f32.to_radians(), 85.0_f32.to_radians());
    }

    /// Updates smooth damping interpolation and any flight in progress each frame.
    pub fn update(&mut self, dt: f32) {
        let smoothing = (14.0 * dt).clamp(0.0, 1.0);
        self.yaw += (self.target_yaw - self.yaw) * smoothing;
        self.pitch += (self.target_pitch - self.pitch) * smoothing;

        if let Some(mut flight) = self.flight {
            flight.elapsed += dt;
            let t = flight.progress();
            let eased = ease_in_out(t);
            self.position = flight.from.lerp(flight.to, eased);
            self.target_position = self.position;
            self.fov_y_radians = self.base_fov_y_radians
                * (1.0 + FLIGHT_FOV_KICK * (std::f32::consts::PI * eased).sin());
            if t >= 1.0 {
                self.position = flight.to;
                self.target_position = flight.to;
                self.fov_y_radians = self.base_fov_y_radians;
                self.flight = None;
            } else {
                self.flight = Some(flight);
            }
        } else {
            self.position = self.position.lerp(self.target_position, smoothing);
        }
    }

    pub fn view_matrix(&self) -> Mat4 {
        glam::camera::rh::view::look_to_mat4(self.position, self.forward(), Vec3::Y)
    }

    pub fn proj_matrix(&self) -> Mat4 {
        glam::camera::rh::proj::directx::perspective(
            self.fov_y_radians,
            self.aspect,
            self.near,
            self.far,
        )
    }

    pub fn view_proj_matrix(&self) -> Mat4 {
        self.proj_matrix() * self.view_matrix()
    }

    /// Generates a 3D ray `(origin, unit_direction)` from normalized device coordinates `[-1, 1]`.
    pub fn screen_ray(&self, ndc_x: f32, ndc_y: f32) -> (Vec3, Vec3) {
        let inv_vp = self.view_proj_matrix().inverse();
        let near_point = inv_vp.project_point3(Vec3::new(ndc_x, ndc_y, 0.0));
        let far_point = inv_vp.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));
        let dir = (far_point - near_point).normalize_or_zero();
        (self.position, dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Advances the camera in fixed steps for `secs` seconds.
    fn run(camera: &mut SphereCamera, secs: f32) {
        let steps = (secs / 0.01).round() as usize;
        for _ in 0..steps {
            camera.update(0.01);
        }
    }

    #[test]
    fn test_flight_reaches_destination_and_restores_fov() {
        let mut camera = SphereCamera::new(1600, 900);
        camera.bounds_radius = 156.0;
        let destination = Vec3::new(92.0, 14.0, -48.0);
        camera.fly_to(destination);
        assert!(camera.is_flying());

        // Midway through the shortest possible flight.
        run(&mut camera, MIN_FLIGHT_SECS * 0.5);
        assert!(camera.is_flying());
        assert!(camera.fov_y_radians > camera.base_fov_y_radians);
        let halfway = camera.position;
        assert!(halfway.length() > 1.0 && halfway.distance(destination) > 1.0);

        run(&mut camera, MAX_FLIGHT_SECS);
        assert!(!camera.is_flying());
        assert!(camera.position.distance(destination) < 1e-3);
        assert_eq!(camera.fov_y_radians, camera.base_fov_y_radians);

        // The camera turned to face the direction of travel.
        let dir = destination.normalize();
        assert!(
            camera.forward().dot(dir) > 0.99,
            "forward = {:?}",
            camera.forward()
        );
    }

    #[test]
    fn test_manual_movement_cancels_a_flight_in_place() {
        let mut camera = SphereCamera::new(1600, 900);
        camera.bounds_radius = 156.0;
        camera.fly_to(Vec3::new(0.0, 0.0, -100.0));
        run(&mut camera, 0.4);
        let stopped_at = camera.position;
        camera.translate_local(0.0, 0.0, 0.0);
        assert!(!camera.is_flying());
        run(&mut camera, 1.0);
        assert!(camera.position.distance(stopped_at) < 1e-3);
        assert_eq!(camera.fov_y_radians, camera.base_fov_y_radians);
    }

    #[test]
    fn test_camera_stays_inside_the_universe() {
        let mut camera = SphereCamera::new(1600, 900);
        camera.bounds_radius = 50.0;
        camera.fly_to(Vec3::new(0.0, 0.0, -500.0));
        run(&mut camera, MAX_FLIGHT_SECS + 0.1);
        assert!((camera.position.length() - 50.0).abs() < 1e-3);
        camera.translate_local(500.0, 0.0, 0.0);
        run(&mut camera, 2.0);
        assert!(camera.position.length() <= 50.0 + 1e-3);
    }

    /// The renderer relies on a right-handed, Y-up view space projected to
    /// clip space with Y up and depth in `[0, 1]` (the wgpu convention).
    #[test]
    fn test_view_proj_clip_space_convention() {
        let camera = SphereCamera::new(1600, 900);
        let view_proj = camera.view_proj_matrix();

        // The default camera looks down -Z: near plane maps to depth 0, far plane to depth 1.
        let near_point = view_proj.project_point3(Vec3::new(0.0, 0.0, -camera.near));
        let far_point = view_proj.project_point3(Vec3::new(0.0, 0.0, -camera.far));
        assert!(near_point.z.abs() < 1e-4, "near depth = {}", near_point.z);
        assert!(
            (far_point.z - 1.0).abs() < 1e-4,
            "far depth = {}",
            far_point.z
        );

        // World up and world right stay up and right on screen.
        let up_right = view_proj.project_point3(Vec3::new(1.0, 1.0, -10.0));
        assert!(up_right.x > 0.0 && up_right.y > 0.0);
        assert!(up_right.z > 0.0 && up_right.z < 1.0);
    }
}
