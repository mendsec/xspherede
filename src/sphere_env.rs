use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::f32::consts::{PI, TAU};

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GridVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MatrixVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    /// [seed, speed, glyph_count_y, alpha_boost]
    pub params: [f32; 4],
}

/// Generates the spherical grid lines inspired by classic SphereXP.
pub fn generate_sphere_grid(radius: f32, meridians: u32, parallels: u32) -> Vec<GridVertex> {
    let mut vertices = Vec::new();
    let segments = 96_u32;

    // 1. Parallels (horizontal latitude lines)
    for p in 1..parallels {
        let frac = (p as f32) / (parallels as f32);
        let phi = (frac - 0.5) * PI; // -PI/2 to +PI/2
        let y = radius * phi.sin();
        let ring_r = radius * phi.cos();

        let is_equator = p * 2 == parallels;
        let is_polar_ring = p == 1 || p == parallels - 1;

        let color = if is_equator {
            [0.15, 0.95, 0.45, 0.55] // Bright Matrix equator
        } else if is_polar_ring {
            [0.25, 1.0, 0.65, 0.70] // Classic SphereXP polar ring
        } else {
            [0.05, 0.55, 0.22, 0.24]
        };

        for s in 0..segments {
            let t0 = (s as f32 / segments as f32) * TAU;
            let t1 = ((s + 1) as f32 / segments as f32) * TAU;

            let p0 = [ring_r * t0.sin(), y, -ring_r * t0.cos()];
            let p1 = [ring_r * t1.sin(), y, -ring_r * t1.cos()];

            vertices.push(GridVertex {
                position: p0,
                color,
            });
            vertices.push(GridVertex {
                position: p1,
                color,
            });
        }
    }

    // 2. Meridians (vertical longitude lines connecting the poles)
    for m in 0..meridians {
        let theta = (m as f32 / meridians as f32) * TAU;
        let is_cardinal = m % (meridians / 4).max(1) == 0;
        let color = if is_cardinal {
            [0.15, 0.85, 0.40, 0.42]
        } else {
            [0.04, 0.48, 0.18, 0.20]
        };

        // Trim near the polar caps to preserve the classic SphereXP look
        let min_phi = -0.44 * PI;
        let max_phi = 0.44 * PI;

        for s in 0..segments {
            let f0 = s as f32 / segments as f32;
            let f1 = (s + 1) as f32 / segments as f32;
            let phi0 = min_phi + f0 * (max_phi - min_phi);
            let phi1 = min_phi + f1 * (max_phi - min_phi);

            let p0 = [
                radius * phi0.cos() * theta.sin(),
                radius * phi0.sin(),
                -radius * phi0.cos() * theta.cos(),
            ];
            let p1 = [
                radius * phi1.cos() * theta.sin(),
                radius * phi1.sin(),
                -radius * phi1.cos() * theta.cos(),
            ];

            vertices.push(GridVertex {
                position: p0,
                color,
            });
            vertices.push(GridVertex {
                position: p1,
                color,
            });
        }
    }

    vertices
}

/// Generates a galaxy halo: a spherical grid of `radius` around `center`.
///
/// With `tint = None` the classic Matrix green grid is kept. Otherwise every line is
/// recolored to `tint`, preserving its relative brightness and opacity. `opacity`
/// scales the alpha of every line (used to keep the universe boundary subdued).
pub fn generate_halo_grid(
    center: Vec3,
    radius: f32,
    meridians: u32,
    parallels: u32,
    tint: Option<[f32; 3]>,
    opacity: f32,
) -> Vec<GridVertex> {
    let mut vertices = generate_sphere_grid(radius, meridians, parallels);
    for v in &mut vertices {
        v.position = (Vec3::from_array(v.position) + center).to_array();
        if let Some([tr, tg, tb]) = tint {
            let brightness = v.color[0].max(v.color[1]).max(v.color[2]);
            v.color[0] = tr * brightness;
            v.color[1] = tg * brightness;
            v.color[2] = tb * brightness;
        }
        v.color[3] *= opacity;
    }
    vertices
}

/// Lightweight deterministic pseudo-random number generator (`xorshift32`).
fn next_rand(state: &mut u32) -> f32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    (x as f32) / (u32::MAX as f32)
}

/// Generates 3D volumetric Matrix code ribbons inside a galaxy of radius `sphere_radius`
/// centered at `center`, along with its outer spherical code curtain, allowing the user to
/// fly through the code corridors when zooming or moving through 3D space.
///
/// Indices start at `base_vertex`, so the volumes of several galaxies can be appended to
/// one buffer. `variant` changes the random layout so galaxies do not look identical.
pub fn generate_matrix_code_volume(
    sphere_radius: f32,
    center: Vec3,
    variant: u32,
    base_vertex: u32,
) -> (Vec<MatrixVertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    // Release date of The Matrix film :) Mixed with the variant for distinct galaxies.
    let mut rng = 0x1999_0331_u32 ^ variant.wrapping_mul(0x9E37_79B9);
    if rng == 0 {
        rng = 0x1999_0331;
    }

    let mut push_ribbon = |center_x: f32,
                           center_y: f32,
                           center_z: f32,
                           half_width: f32,
                           half_height: f32,
                           yaw_angle: f32,
                           seed: f32,
                           speed: f32,
                           glyphs_y: f32,
                           alpha_boost: f32| {
        let base_idx = base_vertex + vertices.len() as u32;
        let center_x = center.x + center_x;
        let center_y = center.y + center_y;
        let center_z = center.z + center_z;
        let (s, c) = yaw_angle.sin_cos();
        let dx = c * half_width;
        let dz = s * half_width;

        let p_tl = [center_x - dx, center_y + half_height, center_z - dz];
        let p_tr = [center_x + dx, center_y + half_height, center_z + dz];
        let p_bl = [center_x - dx, center_y - half_height, center_z - dz];
        let p_br = [center_x + dx, center_y - half_height, center_z + dz];

        let params = [seed, speed, glyphs_y, alpha_boost];

        vertices.push(MatrixVertex {
            position: p_tl,
            uv: [0.0, 0.0],
            params,
        });
        vertices.push(MatrixVertex {
            position: p_tr,
            uv: [1.0, 0.0],
            params,
        });
        vertices.push(MatrixVertex {
            position: p_br,
            uv: [1.0, 1.0],
            params,
        });
        vertices.push(MatrixVertex {
            position: p_bl,
            uv: [0.0, 1.0],
            params,
        });

        // Double-sided geometry so ribbons remain visible from any angle when flying through
        indices.extend_from_slice(&[
            base_idx,
            base_idx + 1,
            base_idx + 2,
            base_idx,
            base_idx + 2,
            base_idx + 3,
            base_idx,
            base_idx + 2,
            base_idx + 1,
            base_idx,
            base_idx + 3,
            base_idx + 2,
        ]);
    };

    // 1. Volumetric columns distributed beyond the Window Clear Field of View radius (r >= 12.5).
    // Guarantees a clean central workspace for windows while preserving 3D depth around and behind them.
    let total_volumetric_columns = 320;
    let min_clear_radius = 12.5;
    for i in 0..total_volumetric_columns {
        let r_frac = next_rand(&mut rng);
        // Distribute between min_clear_radius (12.5) and (sphere_radius - 1.2)
        let dist = min_clear_radius + r_frac.sqrt() * (sphere_radius - min_clear_radius - 1.2);
        let angle =
            (i as f32 / total_volumetric_columns as f32) * TAU + (next_rand(&mut rng) - 0.5) * 0.25;

        let cx = dist * angle.sin();
        let cz = -dist * angle.cos();
        let cy = (next_rand(&mut rng) - 0.5) * 14.0;

        let half_h = 4.5 + next_rand(&mut rng) * 7.5;
        let half_w = 0.14 + next_rand(&mut rng) * 0.12;
        let seed = next_rand(&mut rng) * 100.0;
        let speed = 0.65 + next_rand(&mut rng) * 1.45;
        let glyphs_y = (half_h * 5.0).round();
        let alpha_boost = 0.75 + next_rand(&mut rng) * 0.45;

        // Orient the ribbon perpendicular to the radial vector
        push_ribbon(
            cx,
            cy,
            cz,
            half_w,
            half_h,
            angle,
            seed,
            speed,
            glyphs_y,
            alpha_boost,
        );
    }

    // 2. Dense outer spherical curtain (the Matrix "wallpaper" wrapping the entire sphere)
    let outer_columns = 180;
    let outer_r = sphere_radius * 0.985;
    for i in 0..outer_columns {
        let angle = (i as f32 / outer_columns as f32) * TAU;
        let cx = outer_r * angle.sin();
        let cz = -outer_r * angle.cos();
        let half_h = sphere_radius * 0.85;
        let half_w = (TAU * outer_r / outer_columns as f32) * 0.42;
        let seed = next_rand(&mut rng) * 200.0;
        let speed = 0.5 + next_rand(&mut rng) * 1.1;
        let glyphs_y = 48.0;

        push_ribbon(
            cx, 0.0, cz, half_w, half_h, angle, seed, speed, glyphs_y, 1.1,
        );
    }

    (vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_halo_grid_is_centered_and_tinted() {
        let center = Vec3::new(30.0, -5.0, 12.0);
        let plain = generate_sphere_grid(10.0, 8, 6);
        let halo = generate_halo_grid(center, 10.0, 8, 6, Some([1.0, 0.5, 0.0]), 0.5);
        assert_eq!(plain.len(), halo.len());
        for (p, h) in plain.iter().zip(&halo) {
            let local = Vec3::from_array(h.position) - center;
            assert!((local - Vec3::from_array(p.position)).length() < 1e-4);
            let brightness = p.color[0].max(p.color[1]).max(p.color[2]);
            assert!((h.color[0] - brightness).abs() < 1e-6);
            assert!((h.color[1] - 0.5 * brightness).abs() < 1e-6);
            assert_eq!(h.color[2], 0.0);
            assert!((h.color[3] - 0.5 * p.color[3]).abs() < 1e-6);
        }
    }

    #[test]
    fn test_matrix_volumes_can_share_one_buffer() {
        let (first_v, first_i) = generate_matrix_code_volume(26.0, Vec3::ZERO, 0, 0);
        let base = first_v.len() as u32;
        let center = Vec3::new(90.0, 0.0, 0.0);
        let (second_v, second_i) = generate_matrix_code_volume(26.0, center, 1, base);
        assert_eq!(first_v.len(), second_v.len());
        assert!(first_i.iter().all(|&i| i < base));
        assert!(second_i
            .iter()
            .all(|&i| i >= base && i < base + second_v.len() as u32));
        // The second volume surrounds its own galaxy center.
        for v in &second_v {
            assert!(Vec3::from_array(v.position).distance(center) < 26.0 * 1.5);
        }
        // Different variants produce different layouts.
        assert_ne!(first_v[0].params, second_v[0].params);
    }
}
