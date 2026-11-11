use glam::{Mat4, Vec3};

/// Mini-Mapa 3D Interativo do Globo (fiel ao SphereXP original).
/// Mostra a própria esfera 3D vista de fora, com as janelas posicionadas em suas
/// profundidades radiais reais dentro do volume do globo, sem aros/botões externos
/// nem linhas artificiais de campo de visão.
pub struct MiniGlobeHud {
    /// Yaw da câmera externa que observa o mini-globo
    pub orb_yaw: f32,
    /// Pitch da câmera externa que observa o mini-globo
    pub orb_pitch: f32,
    pub target_orb_yaw: f32,
    pub target_orb_pitch: f32,
    /// Distância da câmera externa ao centro do mini-globo
    pub camera_distance: f32,
    pub sphere_radius: f32,
    /// Indica se o cursor do mouse está atualmente sobre o Mini-Globo
    pub is_hovered: bool,
    /// Indica se o Mini-Globo está expandido no centro da tela (modo Overview do SphereXP)
    pub expanded: bool,
    /// Progresso animado da expansão [0.0 = canto inf. direito, 1.0 = centro da tela]
    pub expansion_anim: f32,
    /// Progresso animado do hover no modo compacto [0.0 .. 1.0]
    pub hover_anim: f32,
}

impl MiniGlobeHud {
    pub fn new(sphere_radius: f32) -> Self {
        Self {
            orb_yaw: 0.0,
            orb_pitch: 0.30,
            target_orb_yaw: 0.0,
            target_orb_pitch: 0.30,
            camera_distance: sphere_radius * 2.42,
            sphere_radius,
            is_hovered: false,
            expanded: false,
            expansion_anim: 0.0,
            hover_anim: 0.0,
        }
    }

    pub fn toggle_expanded(&mut self) {
        self.expanded = !self.expanded;
    }

    /// Retorna `(x, y, tamanho)` do viewport quadrado do Mini-Globo interpolando suavemente
    /// entre o canto inferior direito e o centro da tela (quando expandido).
    pub fn viewport_rect(&self, win_width: u32, win_height: u32) -> (f32, f32, f32) {
        let w = win_width as f32;
        let h = win_height as f32;
        let min_dim = w.min(h);

        let base_compact = (min_dim * 0.25).clamp(175.0, 265.0);
        let compact_size = base_compact * (1.0 + 0.14 * self.hover_anim);
        let margin = 18.0;
        let compact_x = (w - compact_size - margin).max(0.0);
        let compact_y = (h - compact_size - margin).max(0.0);

        let expanded_size = (min_dim * 0.80).clamp(320.0, 900.0);
        let expanded_x = ((w - expanded_size) * 0.5).max(0.0);
        let expanded_y = ((h - expanded_size) * 0.5).max(0.0);

        let t = self.expansion_anim * self.expansion_anim * (3.0 - 2.0 * self.expansion_anim);
        let x = compact_x + (expanded_x - compact_x) * t;
        let y = compact_y + (expanded_y - compact_y) * t;
        let size = compact_size + (expanded_size - compact_size) * t;

        (x, y, size)
    }

    /// Verifica se o cursor `(px, py)` está dentro do círculo do Mini-Globo.
    pub fn contains_cursor(&self, px: f32, py: f32, win_width: u32, win_height: u32) -> bool {
        let (vx, vy, size) = self.viewport_rect(win_width, win_height);
        let cx = vx + size * 0.5;
        let cy = vy + size * 0.5;
        let dx = px - cx;
        let dy = py - cy;
        let radius = size * 0.48;
        (dx * dx + dy * dy) <= radius * radius
    }

    /// Gira a orientação do Mini-Globo quando o usuário o arrasta.
    pub fn rotate_globe(&mut self, delta_x: f32, delta_y: f32) {
        let sens = if self.expanded { 0.0055 } else { 0.009 };
        self.target_orb_yaw -= delta_x * sens;
        self.target_orb_pitch = (self.target_orb_pitch + delta_y * sens).clamp(-1.35, 1.35);
    }

    pub fn update(&mut self, dt: f32) {
        let smooth = (14.0 * dt).clamp(0.0, 1.0);
        self.orb_yaw += (self.target_orb_yaw - self.orb_yaw) * smooth;
        self.orb_pitch += (self.target_orb_pitch - self.orb_pitch) * smooth;

        let target_exp = if self.expanded { 1.0 } else { 0.0 };
        self.expansion_anim += (target_exp - self.expansion_anim) * (10.0 * dt).clamp(0.0, 1.0);

        let target_hov = if self.is_hovered && !self.expanded {
            1.0
        } else {
            0.0
        };
        self.hover_anim += (target_hov - self.hover_anim) * (12.0 * dt).clamp(0.0, 1.0);
    }

    /// Posição da câmera externa observando o Mini-Globo.
    pub fn eye_position(&self) -> Vec3 {
        let (sin_y, cos_y) = self.orb_yaw.sin_cos();
        let (sin_p, cos_p) = self.orb_pitch.sin_cos();
        Vec3::new(
            self.camera_distance * cos_p * sin_y,
            self.camera_distance * sin_p,
            self.camera_distance * cos_p * cos_y,
        )
    }

    pub fn view_proj_matrix(&self) -> Mat4 {
        let eye = self.eye_position();
        let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh(48.0_f32.to_radians(), 1.0, 1.0, 200.0);
        proj * view
    }

    /// Gera um raio 3D `(origem, direção)` no espaço do Mini-Globo a partir do cursor `(px, py)`.
    pub fn screen_ray(
        &self,
        px: f32,
        py: f32,
        win_width: u32,
        win_height: u32,
    ) -> Option<(Vec3, Vec3)> {
        if !self.contains_cursor(px, py, win_width, win_height) {
            return None;
        }
        let (vx, vy, size) = self.viewport_rect(win_width, win_height);
        let ndc_x = ((px - vx) / size.max(1.0)) * 2.0 - 1.0;
        let ndc_y = 1.0 - ((py - vy) / size.max(1.0)) * 2.0;

        let inv_vp = self.view_proj_matrix().inverse();
        let near_pt = inv_vp.project_point3(Vec3::new(ndc_x, ndc_y, 0.0));
        let far_pt = inv_vp.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));
        let ray_origin = self.eye_position();
        let ray_dir = (far_pt - near_pt).normalize_or_zero();
        Some((ray_origin, ray_dir))
    }

    /// Faz raycasting do cursor `(px, py)` na casca do Mini-Globo 3D e retorna `(azimuth, elevation)`
    /// do ponto clicado/apontado na superfície da esfera.
    pub fn pick_spherical_point(
        &self,
        px: f32,
        py: f32,
        win_width: u32,
        win_height: u32,
    ) -> Option<(f32, f32)> {
        let (ray_origin, ray_dir) = self.screen_ray(px, py, win_width, win_height)?;

        let r = self.sphere_radius;
        let b = ray_origin.dot(ray_dir);
        let c = ray_origin.length_squared() - r * r;
        let disc = b * b - c;

        let hit_point = if disc >= 0.0 {
            let t = (-b - disc.sqrt()).max(0.1);
            ray_origin + ray_dir * t
        } else {
            let t_closest = (-b).max(0.1);
            (ray_origin + ray_dir * t_closest).normalize_or_zero() * r
        };

        let len = hit_point.length().max(1e-4);
        let elevation = (hit_point.y / len).clamp(-1.0, 1.0).asin();
        let azimuth = hit_point.x.atan2(-hit_point.z);

        Some((azimuth, elevation))
    }
}
