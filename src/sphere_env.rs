use bytemuck::{Pod, Zeroable};
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

/// Gera as linhas da grade esférica inspirada no SphereXP clássico.
pub fn generate_sphere_grid(radius: f32, meridians: u32, parallels: u32) -> Vec<GridVertex> {
    let mut vertices = Vec::new();
    let segments = 96_u32;

    // 1. Paralelos (linhas horizontais de latitude)
    for p in 1..parallels {
        let frac = (p as f32) / (parallels as f32);
        let phi = (frac - 0.5) * PI; // -PI/2 a +PI/2
        let y = radius * phi.sin();
        let ring_r = radius * phi.cos();

        let is_equator = p * 2 == parallels;
        let is_polar_ring = p == 1 || p == parallels - 1;

        let color = if is_equator {
            [0.15, 0.95, 0.45, 0.55] // Equador Matrix brilhante
        } else if is_polar_ring {
            [0.25, 1.0, 0.65, 0.70] // Anel polar clássico do SphereXP
        } else {
            [0.05, 0.55, 0.22, 0.24]
        };

        for s in 0..segments {
            let t0 = (s as f32 / segments as f32) * TAU;
            let t1 = ((s + 1) as f32 / segments as f32) * TAU;

            let p0 = [ring_r * t0.sin(), y, -ring_r * t0.cos()];
            let p1 = [ring_r * t1.sin(), y, -ring_r * t1.cos()];

            vertices.push(GridVertex { position: p0, color });
            vertices.push(GridVertex { position: p1, color });
        }
    }

    // 2. Meridianos (linhas verticais de longitude ligando os polos)
    for m in 0..meridians {
        let theta = (m as f32 / meridians as f32) * TAU;
        let is_cardinal = m % (meridians / 4).max(1) == 0;
        let color = if is_cardinal {
            [0.15, 0.85, 0.40, 0.42]
        } else {
            [0.04, 0.48, 0.18, 0.20]
        };

        // Corta próximo às tampas polares para manter o visual clássico do SphereXP
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

            vertices.push(GridVertex { position: p0, color });
            vertices.push(GridVertex { position: p1, color });
        }
    }

    vertices
}

/// Gera um gerador pseudo-aleatório determinístico leve (xorshift32).
fn next_rand(state: &mut u32) -> f32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    (x as f32) / (u32::MAX as f32)
}

/// Gera as fileiras tridimensionais (volumétricas) de código Matrix dentro da esfera,
/// além da parede esférica externa de códigos, permitindo que o usuário passe por entre
/// os corredores de código ao dar zoom ou caminhar pelo espaço 3D.
pub fn generate_matrix_code_volume(sphere_radius: f32) -> (Vec<MatrixVertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut rng = 0x1999_0331_u32; // Ano de lançamento do filme Matrix :)

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
        let base_idx = vertices.len() as u32;
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

        // Dupla face para ser visível de qualquer ângulo quando a câmera atravessar as fileiras
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

    // 1. Fileiras volumétricas distribuídas além do Raio de Visão Limpa das janelas (r >= 12.5)
    // Garante uma zona central limpa para uso das janelas, mantendo profundidade 3D ao redor e ao fundo.
    let total_volumetric_columns = 320;
    let min_clear_radius = 12.5;
    for i in 0..total_volumetric_columns {
        let r_frac = next_rand(&mut rng);
        // Distribui entre min_clear_radius (12.5) e (sphere_radius - 1.2)
        let dist = min_clear_radius + r_frac.sqrt() * (sphere_radius - min_clear_radius - 1.2);
        let angle = (i as f32 / total_volumetric_columns as f32) * TAU
            + (next_rand(&mut rng) - 0.5) * 0.25;

        let cx = dist * angle.sin();
        let cz = -dist * angle.cos();
        let cy = (next_rand(&mut rng) - 0.5) * 14.0;

        let half_h = 4.5 + next_rand(&mut rng) * 7.5;
        let half_w = 0.14 + next_rand(&mut rng) * 0.12;
        let seed = next_rand(&mut rng) * 100.0;
        let speed = 0.65 + next_rand(&mut rng) * 1.45;
        let glyphs_y = (half_h * 5.0).round();
        let alpha_boost = 0.75 + next_rand(&mut rng) * 0.45;

        // Orienta a fita perpendicularmente ao raio (ou em grade Matrix)
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

    // 2. Cortina esférica externa densa (o "papel de parede" Matrix envolvendo toda a esfera)
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
            cx,
            0.0,
            cz,
            half_w,
            half_h,
            angle,
            seed,
            speed,
            glyphs_y,
            1.1,
        );
    }

    (vertices, indices)
}
