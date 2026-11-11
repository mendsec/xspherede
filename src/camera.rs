use glam::{Mat4, Vec3};

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
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
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
            aspect,
            near: 0.1,
            far: 120.0,
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

    /// Moves the camera through the internal 3D space of the sphere (flying through Matrix columns).
    pub fn translate_local(&mut self, forward_delta: f32, right_delta: f32, up_delta: f32) {
        let movement =
            self.forward() * forward_delta + self.right() * right_delta + self.up() * up_delta;
        self.target_position += movement;

        // Keep the user within the main sphere boundary (radius ~26.0)
        let max_radius = 22.0;
        if self.target_position.length() > max_radius {
            self.target_position = self.target_position.normalize() * max_radius;
        }
    }

    /// Smoothly returns to the center of the sphere.
    pub fn reset_to_center(&mut self) {
        self.target_position = Vec3::ZERO;
        self.target_pitch = 0.0;
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

    /// Updates smooth damping interpolation each frame.
    pub fn update(&mut self, dt: f32) {
        let smoothing = (14.0 * dt).clamp(0.0, 1.0);
        self.yaw += (self.target_yaw - self.yaw) * smoothing;
        self.pitch += (self.target_pitch - self.pitch) * smoothing;
        self.position = self.position.lerp(self.target_position, smoothing);
    }

    pub fn view_matrix(&self) -> Mat4 {
        Mat4::look_to_rh(self.position, self.forward(), Vec3::Y)
    }

    pub fn proj_matrix(&self) -> Mat4 {
        Mat4::perspective_rh(self.fov_y_radians, self.aspect, self.near, self.far)
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
