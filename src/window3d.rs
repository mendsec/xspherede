use bytemuck::{Pod, Zeroable};
use glam::Vec3;

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct WindowVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub normal: [f32; 3],
}

/// Abstração para provedores de texturas de janelas reais (XComposite, PipeWire ou Smithay Wayland).
/// Permite que o motor 3D esférico permaneça desacoplado do protocolo de sistema de janelas.
#[allow(dead_code)]
pub trait WindowSurfaceProvider {
    fn title(&self) -> &str;
    fn dimensions(&self) -> (u32, u32);
    fn render_rgba(&self, time_secs: f32) -> Vec<u8>;
}

/// Janela posicionada na superfície esférica 3D do XSphereDE.
pub struct SphericalWindow {
    pub id: usize,
    pub title: String,
    /// Ângulo horizontal na esfera (radianos, 0 = frente -Z)
    pub azimuth: f32,
    /// Ângulo vertical na esfera (radianos, 0 = equador)
    pub elevation: f32,
    /// Distância radial do centro da esfera (permite aproximar/afastar janelas em profundidade!)
    pub radius: f32,
    /// Largura angular em radianos na esfera
    pub angular_width: f32,
    /// Altura angular em radianos na esfera
    pub angular_height: f32,
    pub hovered: bool,
    pub focused: bool,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    pub bind_group: wgpu::BindGroup,
}

impl SphericalWindow {
    /// Gera a malha 3D curva acompanhando perfeitamente a casca esférica de raio `self.radius`.
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
            // v = 0 é o topo da janela, v = 1 é a base
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
                // Normal apontando para o centro da esfera (para o usuário)
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

                // Dupla face para continuar visível caso a câmera passe por trás da janela no espaço 3D
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

    /// Posição 3D cartesiana do centro desta janela na esfera.
    pub fn world_center(&self) -> Vec3 {
        let (sin_phi, cos_phi) = self.elevation.sin_cos();
        let (sin_theta, cos_theta) = self.azimuth.sin_cos();
        Vec3::new(
            self.radius * cos_phi * sin_theta,
            self.radius * sin_phi,
            -self.radius * cos_phi * cos_theta,
        )
    }

    /// Cosseno do raio angular de proteção da janela (com folga extra quando focada/em uso)
    /// para bloquear códigos Matrix de passarem sobre a janela.
    pub fn shield_cos(&self) -> f32 {
        let half_diag = 0.5 * (self.angular_width.hypot(self.angular_height));
        let margin = if self.focused || self.hovered { 0.14 } else { 0.06 };
        (half_diag + margin).cos()
    }

    /// Faz raycasting 3D exato contra o patch esférico da janela.
    /// Retorna a distância `t` ao longo do raio se o cursor atingir esta janela 3D.
    pub fn intersect_ray(&self, ray_origin: Vec3, ray_dir: Vec3) -> Option<f32> {
        // Interseção raio-esfera de raio `self.radius` centrada na origem (0,0,0):
        // |O + t*D|^2 = R^2  =>  t^2 + 2*(O.D)*t + (|O|^2 - R^2) = 0
        let b = ray_origin.dot(ray_dir);
        let c = ray_origin.length_squared() - self.radius * self.radius;
        let disc = b * b - c;
        if disc < 0.0 {
            return None;
        }
        let sqrt_disc = disc.sqrt();

        // Testa as duas raízes (positivas à frente da câmera)
        for t in [-b - sqrt_disc, -b + sqrt_disc] {
            if t <= 0.05 {
                continue;
            }
            let hit = ray_origin + ray_dir * t;
            // Converte o ponto de impacto para coordenadas esféricas (azimuth, elevation)
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

/// Diferença angular normalizada para o intervalo [-PI, PI].
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

/// Gera uma textura RGBA estilizada para representar janelas do ambiente XSphereDE no protótipo.
pub fn generate_demo_window_rgba(
    width: u32,
    height: u32,
    theme_variant: usize,
) -> Vec<u8> {
    let mut data = vec![0u8; (width * height * 4) as usize];
    let title_bar_h = (height as f32 * 0.11) as u32;

    for y in 0..height {
        for x in 0..width {
            let idx = ((y * width + x) * 4) as usize;
            let border = x < 3 || x >= width - 3 || y < 3 || y >= height - 3;

            if border {
                data[idx] = 40;
                data[idx + 1] = 255;
                data[idx + 2] = 120;
                data[idx + 3] = 245;
                continue;
            }

            if y < title_bar_h {
                // Barra de título estilo Cyber-SphereXP
                let grad = 1.0 - (x as f32 / width as f32) * 0.5;
                data[idx] = (18.0 * grad) as u8;
                data[idx + 1] = (115.0 * grad) as u8;
                data[idx + 2] = (55.0 * grad) as u8;
                data[idx + 3] = 235;

                // Botões de janela no canto superior direito (fechar, maximizar, esfera-pin)
                let btn_y = y >= 6 && y + 6 < title_bar_h;
                if btn_y && x > width - 26 && x < width - 10 {
                    data[idx] = 230;
                    data[idx + 1] = 75;
                    data[idx + 2] = 75;
                } else if btn_y && x > width - 48 && x < width - 32 {
                    data[idx] = 60;
                    data[idx + 1] = 240;
                    data[idx + 2] = 130;
                }
                continue;
            }

            // Corpo da janela translúcido escuro com elementos visuais distintos por app
            let mut r = 8_u8;
            let mut g = 24_u8;
            let mut b = 14_u8;
            let alpha = 212_u8;

            // Scanline sutil
            if y % 4 == 0 {
                g = g.saturating_add(6);
            }

            match theme_variant {
                0 => {
                    // Terminal Matrix / XSphere Shell
                    if x > 18 && x < width - 18 && y > title_bar_h + 14 {
                        let line = (y - title_bar_h - 14) / 14;
                        let row_py = (y - title_bar_h - 14) % 14;
                        let max_x = ((line * 73 + 91) % (width - 60)).max(40);
                        if row_py < 8 && x < max_x && (x % 9 < 6) {
                            r = 45;
                            g = if line % 4 == 0 { 250 } else { 185 };
                            b = 95;
                        }
                    }
                }
                1 => {
                    // Monitor de Desempenho 3D (Gráfico de onda + barras)
                    let gx = x as f32 / width as f32;
                    let gy = (y - title_bar_h) as f32 / (height - title_bar_h) as f32;
                    let wave = 0.5 + 0.28 * (gx * 16.0).sin() * (gx * 5.0).cos();
                    if (gy - wave).abs() < 0.022 {
                        r = 80;
                        g = 255;
                        b = 150;
                    } else if gy > wave {
                        g = g.saturating_add(((1.0 - gy) * 45.0) as u8);
                    }
                }
                2 => {
                    // Navegador Espacial de Arquivos (Ícones em grade)
                    let content_y = y - title_bar_h;
                    let cell_x = x / 56;
                    let cell_y = content_y / 56;
                    let in_icon_x = (x % 56) > 12 && (x % 56) < 44;
                    let in_icon_y = (content_y % 56) > 10 && (content_y % 56) < 38;
                    if cell_x > 0 && cell_x < 6 && cell_y > 0 && cell_y < 4 && in_icon_x && in_icon_y {
                        r = 35;
                        g = 200;
                        b = 110;
                    }
                }
                _ => {
                    // Radar / Mapa Esférico do Desktop
                    let cx = width as f32 * 0.5;
                    let cy = (height + title_bar_h) as f32 * 0.5;
                    let dx = x as f32 - cx;
                    let dy = y as f32 - cy;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if (dist - 35.0).abs() < 1.8 || (dist - 70.0).abs() < 1.8 || (dist - 105.0).abs() < 2.0 {
                        r = 50;
                        g = 235;
                        b = 120;
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
