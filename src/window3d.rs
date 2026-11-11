use crate::security::TrustDomain;
use bytemuck::{Pod, Zeroable};
use glam::Vec3;

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct WindowVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub normal: [f32; 3],
}

/// Abstraction for real window texture providers (PipeWire or Smithay Wayland; Xwayland disabled by policy).
/// Keeps the 3D spherical engine decoupled from the underlying windowing protocol while enforcing
/// Qubes-style `TrustDomain` compartmentalization.
#[allow(dead_code)]
pub trait WindowSurfaceProvider {
    fn title(&self) -> &str;
    fn trust_domain(&self) -> TrustDomain;
    fn dimensions(&self) -> (u32, u32);
    fn render_rgba(&self, time_secs: f32) -> Vec<u8>;
}

/// Window positioned on the 3D spherical surface of XSphereDE.
pub struct SphericalWindow {
    pub id: usize,
    pub title: String,
    /// Immutable Qubes-style security domain assigned by the trusted compositor
    pub trust_domain: TrustDomain,
    /// Horizontal angle on the sphere (radians, `0` = front `-Z`)
    pub azimuth: f32,
    /// Vertical angle on the sphere (radians, `0` = equator)
    pub elevation: f32,
    /// Radial distance from the sphere center (allows pushing/pulling windows in 3D depth)
    pub radius: f32,
    /// Angular width in radians on the sphere
    pub angular_width: f32,
    /// Angular height in radians on the sphere
    pub angular_height: f32,
    pub hovered: bool,
    pub focused: bool,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    pub bind_group: wgpu::BindGroup,
}

impl SphericalWindow {
    /// Builds a curved 3D mesh that conforms to the spherical shell of radius `self.radius`.
    pub fn build_curved_mesh(
        azimuth: f32,
        elevation: f32,
        radius: f32,
        angular_width: f32,
        angular_height: f32,
    ) -> (Vec<WindowVertex>, Vec<u32>) {
        let cols = 16_u32;
        let rows = 12_u32;
        let mut vertices = Vec::with_capacity(((cols + 1) * (rows + 1)) as usize);
        let mut indices = Vec::with_capacity((cols * rows * 6) as usize);

        for r in 0..=rows {
            let v = r as f32 / rows as f32;
            // v = 0 is the top of the window, v = 1 is the bottom
            let phi = elevation + (0.5 - v) * angular_height;
            let (sin_phi, cos_phi) = phi.sin_cos();

            for c in 0..=cols {
                let u = c as f32 / cols as f32;
                let theta = azimuth + (u - 0.5) * angular_width;
                let (sin_theta, cos_theta) = theta.sin_cos();

                let pos = Vec3::new(
                    radius * cos_phi * sin_theta,
                    radius * sin_phi,
                    -radius * cos_phi * cos_theta,
                );
                // Normal pointing toward the center of the sphere (toward the user)
                let normal = (-pos).normalize_or_zero();

                vertices.push(WindowVertex {
                    position: pos.to_array(),
                    uv: [u, v],
                    normal: normal.to_array(),
                });
            }
        }

        let stride = cols + 1;
        for r in 0..rows {
            for c in 0..cols {
                let i0 = r * stride + c;
                let i1 = i0 + 1;
                let i2 = (r + 1) * stride + c;
                let i3 = i2 + 1;

                // Double-sided indices so the window remains visible if the camera flies behind it in 3D space
                indices.extend_from_slice(&[i0, i2, i1, i1, i2, i3, i0, i1, i2, i1, i3, i2]);
            }
        }

        (vertices, indices)
    }

    pub fn update_geometry(&self, queue: &wgpu::Queue) {
        let (vertices, _) = Self::build_curved_mesh(
            self.azimuth,
            self.elevation,
            self.radius,
            self.angular_width,
            self.angular_height,
        );
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
    }

    /// 3D Cartesian world position of this window's center on the sphere.
    pub fn world_center(&self) -> Vec3 {
        let (sin_phi, cos_phi) = self.elevation.sin_cos();
        let (sin_theta, cos_theta) = self.azimuth.sin_cos();
        Vec3::new(
            self.radius * cos_phi * sin_theta,
            self.radius * sin_phi,
            -self.radius * cos_phi * cos_theta,
        )
    }

    /// Cosine of the angular protection cone for this window (with extra padding when focused/hovered)
    /// to prevent Matrix code streams from passing in front of the window.
    pub fn shield_cos(&self) -> f32 {
        let half_diag = 0.5 * (self.angular_width.hypot(self.angular_height));
        let margin = if self.focused || self.hovered { 0.14 } else { 0.06 };
        (half_diag + margin).cos()
    }

    /// Performs exact 3D raycasting against the spherical patch of the window.
    /// Returns the distance `t` along the ray if the cursor hits this 3D window.
    pub fn intersect_ray(&self, ray_origin: Vec3, ray_dir: Vec3) -> Option<f32> {
        // Ray-sphere intersection for radius `self.radius` centered at the origin (0,0,0):
        // |O + t*D|^2 = R^2  =>  t^2 + 2*(O.D)*t + (|O|^2 - R^2) = 0
        let b = ray_origin.dot(ray_dir);
        let c = ray_origin.length_squared() - self.radius * self.radius;
        let disc = b * b - c;
        if disc < 0.0 {
            return None;
        }
        let sqrt_disc = disc.sqrt();

        // Test both roots (positive distances in front of the camera)
        for t in [-b - sqrt_disc, -b + sqrt_disc] {
            if t <= 0.05 {
                continue;
            }
            let hit = ray_origin + ray_dir * t;
            // Convert hit point into spherical coordinates (azimuth, elevation)
            let r = hit.length();
            if r < 1e-4 {
                continue;
            }
            let phi = (hit.y / r).clamp(-1.0, 1.0).asin();
            let theta = hit.x.atan2(-hit.z);

            let d_theta = wrap_angle_diff(theta, self.azimuth).abs();
            let d_phi = (phi - self.elevation).abs();

            if d_theta <= self.angular_width * 0.5 && d_phi <= self.angular_height * 0.5 {
                return Some(t);
            }
        }
        None
    }
}

/// Angular difference normalized to the interval `[-PI, PI]`.
pub fn wrap_angle_diff(a: f32, b: f32) -> f32 {
    let mut diff = a - b;
    while diff > std::f32::consts::PI {
        diff -= std::f32::consts::TAU;
    }
    while diff < -std::f32::consts::PI {
        diff += std::f32::consts::TAU;
    }
    diff
}

/// Generates a stylized RGBA texture representing XSphereDE desktop windows in the prototype,
/// respecting `secureblue` (no external thumbnailers) and `Qubes OS` (`TrustDomain` header styling).
pub fn generate_demo_window_rgba(
    width: u32,
    height: u32,
    theme_variant: usize,
    trust_domain: TrustDomain,
) -> Vec<u8> {
    let mut data = vec![0u8; (width * height * 4) as usize];
    let title_bar_h = (height as f32 * 0.11) as u32;
    let [tr, tg, tb] = trust_domain.border_color_rgb();
    let dom_r = (tr * 255.0) as u8;
    let dom_g = (tg * 255.0) as u8;
    let dom_b = (tb * 255.0) as u8;

    for y in 0..height {
        for x in 0..width {
            let idx = ((y * width + x) * 4) as usize;
            let border = x < 3 || x >= width - 3 || y < 3 || y >= height - 3;

            if border {
                data[idx] = dom_r;
                data[idx + 1] = dom_g;
                data[idx + 2] = dom_b;
                data[idx + 3] = 248;
                continue;
            }

            if y < title_bar_h {
                // Qubes-inspired domain-colored title bar
                let grad = 1.0 - (x as f32 / width as f32) * 0.45;
                data[idx] = ((dom_r as f32) * 0.46 * grad) as u8;
                data[idx + 1] = ((dom_g as f32) * 0.46 * grad) as u8;
                data[idx + 2] = ((dom_b as f32) * 0.46 * grad) as u8;
                data[idx + 3] = 238;

                // Security domain badge indicator on the left side of the title bar
                let badge_y = y >= 9 && y + 7 < title_bar_h;
                if badge_y && x >= 12 && x <= 74 && (x % 6 < 4) {
                    data[idx] = dom_r;
                    data[idx + 1] = dom_g;
                    data[idx + 2] = dom_b;
                }

                // Window buttons in the top-right corner (close, maximize, sphere-pin)
                let btn_y = y >= 8 && y + 6 < title_bar_h;
                if btn_y && x > width - 26 && x < width - 10 {
                    data[idx] = 235;
                    data[idx + 1] = 75;
                    data[idx + 2] = 75;
                } else if btn_y && x > width - 48 && x < width - 32 {
                    data[idx] = dom_r;
                    data[idx + 1] = dom_g;
                    data[idx + 2] = dom_b;
                }
                continue;
            }

            // Translucent dark window body with distinct visual elements per app
            let mut r = 8_u8;
            let mut g = 22_u8;
            let mut b = 14_u8;
            let alpha = 214_u8;

            // Subtle scanline
            if y % 4 == 0 {
                g = g.saturating_add(5);
            }

            match theme_variant {
                0 => {
                    // [DOM0: VAULT] Matrix Terminal // run0 (Zero-SUID Policy)
                    if x > 18 && x < width - 18 && y > title_bar_h + 14 {
                        let line = (y - title_bar_h - 14) / 14;
                        let row_py = (y - title_bar_h - 14) % 14;
                        let max_x = ((line * 73 + 91) % (width - 60)).max(40);
                        if row_py < 8 && x < max_x && (x % 9 < 6) {
                            if line == 0 {
                                // First line highlighted with Vault Violet
                                r = dom_r;
                                g = dom_g;
                                b = dom_b;
                            } else {
                                r = 55;
                                g = if line % 4 == 0 { 250 } else { 190 };
                                b = 105;
                            }
                        }
                    }
                }
                1 => {
                    // [TRUSTED] 3D Security & Kernel Auditor (waveform + policy bars)
                    let gx = x as f32 / width as f32;
                    let gy = (y - title_bar_h) as f32 / (height - title_bar_h) as f32;
                    let wave = 0.5 + 0.28 * (gx * 16.0).sin() * (gx * 5.0).cos();
                    if (gy - wave).abs() < 0.022 {
                        r = dom_r;
                        g = dom_g;
                        b = dom_b;
                    } else if gy > wave {
                        g = g.saturating_add(((1.0 - gy) * 45.0) as u8);
                    }
                }
                2 => {
                    // [ISOLATED] Spatial File Navigator (secureblue No-Thumbnails Policy: vector icons only)
                    let content_y = y - title_bar_h;
                    let cell_x = x / 56;
                    let cell_y = content_y / 56;
                    let local_x = x % 56;
                    let local_y = content_y % 56;
                    let in_icon_border = (local_x == 12 || local_x == 43 || local_y == 10 || local_y == 37)
                        && (12..=43).contains(&local_x)
                        && (10..=37).contains(&local_y);
                    let in_safe_glyph = (18..=37).contains(&local_x)
                        && (16..=31).contains(&local_y)
                        && (local_y % 5 < 3);
                    if cell_x > 0 && cell_x < 6 && cell_y > 0 && cell_y < 4 && (in_icon_border || in_safe_glyph) {
                        r = dom_r;
                        g = dom_g;
                        b = dom_b;
                    }
                }
                _ => {
                    // [DISP-RED] Disposable Quarantine Sandbox Radar
                    let cx = width as f32 * 0.5;
                    let cy = (height + title_bar_h) as f32 * 0.5;
                    let dx = x as f32 - cx;
                    let dy = y as f32 - cy;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if (dist - 35.0).abs() < 1.8 || (dist - 70.0).abs() < 1.8 || (dist - 105.0).abs() < 2.0 {
                        r = dom_r;
                        g = dom_g;
                        b = dom_b;
                    }
                }
            }

            data[idx] = r;
            data[idx + 1] = g;
            data[idx + 2] = b;
            data[idx + 3] = alpha;
        }
    }

    data
}
