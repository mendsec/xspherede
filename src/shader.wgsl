struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    // [time_secs, active_window_id, hovered_window_id, matrix_intensity]
    params: vec4<f32>,
    // xyz = direção frontal da câmera, w = distância do campo de visão limpa frontal
    camera_forward: vec4<f32>,
    // Para cada janela: xyz = centro 3D da janela no mundo, w = cosseno do cone angular de proteção
    window_shields: array<vec4<f32>, 4>,
};

@group(0) @binding(0)
var<uniform> globals: Globals;

// ============================================================================
// 1. SHADER DA GRADE ESFÉRICA (SPHEREXP WIREFRAME GLOBE)
// ============================================================================
struct GridIn {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct GridOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world_pos: vec3<f32>,
};

@vertex
fn vs_grid(in: GridIn) -> GridOut {
    var out: GridOut;
    out.clip_pos = globals.view_proj * vec4<f32>(in.position, 1.0);
    out.color = in.color;
    out.world_pos = in.position;
    return out;
}

@fragment
fn fs_grid(in: GridOut) -> @location(0) vec4<f32> {
    let time = globals.params.x;
    let pulse = 0.82 + 0.18 * sin(time * 1.8 + in.world_pos.y * 0.25);
    return vec4<f32>(in.color.rgb * pulse, in.color.a);
}

// ============================================================================
// 2. SHADER VOLUMÉTRICO 3D DO CÓDIGO MATRIX
// ============================================================================
struct MatrixIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    // [seed, speed, glyphs_y, alpha_boost]
    @location(2) params: vec4<f32>,
};

struct MatrixOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) params: vec4<f32>,
    @location(2) world_pos: vec3<f32>,
};

fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Gera glifos procedurais estilo Matrix (Katakana / Binário estilizado) em grade 5x7
fn procedural_matrix_glyph(cell_uv: vec2<f32>, glyph_id: f32) -> f32 {
    // Margem entre caracteres
    if (cell_uv.x < 0.14 || cell_uv.x > 0.86 || cell_uv.y < 0.10 || cell_uv.y > 0.90) {
        return 0.0;
    }
    let gx = floor(((cell_uv.x - 0.14) / 0.72) * 5.0);
    let gy = floor(((cell_uv.y - 0.10) / 0.80) * 7.0);

    // Simetria parcial + traços horizontais/verticais típicos de Katakana meia-largura
    let bit_hash = hash12(vec2<f32>(gx + floor(glyph_id * 17.0), gy + floor(glyph_id * 31.0)));
    let stem = select(0.0, 1.0, abs(gx - 2.0) < 0.6 && gy > 0.5 && gy < 5.5 && fract(glyph_id * 7.3) > 0.35);
    let bar = select(0.0, 1.0, (gy == 1.0 || gy == 3.0) && gx >= 1.0 && gx <= 3.0 && fract(glyph_id * 13.7) > 0.3);
    let dots = select(0.0, 1.0, bit_hash > 0.46);

    return max(max(stem, bar), dots);
}

@vertex
fn vs_matrix(in: MatrixIn) -> MatrixOut {
    var out: MatrixOut;
    out.clip_pos = globals.view_proj * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    out.params = in.params;
    out.world_pos = in.position;
    return out;
}

@fragment
fn fs_matrix(in: MatrixOut) -> @location(0) vec4<f32> {
    let time = globals.params.x * globals.params.w;
    let seed = in.params.x;
    let speed = in.params.y;
    let glyphs_y = in.params.z;
    let alpha_boost = in.params.w;

    // Coordenada vertical em células de glifo
    let row_f = in.uv.y * glyphs_y;
    let row_idx = floor(row_f);
    let cell_uv = vec2<f32>(in.uv.x, fract(row_f));

    // Alguns glifos mutam rapidamente como no filme Matrix
    let morph_step = floor(time * (2.5 + fract(seed) * 4.0));
    let mutating = step(0.82, hash12(vec2<f32>(row_idx, seed)));
    let glyph_id = hash12(vec2<f32>(row_idx + seed * 11.0, mutating * morph_step));

    let pixel_on = procedural_matrix_glyph(cell_uv, glyph_id);
    if (pixel_on < 0.5) {
        discard;
    }

    // Posição da gota de código caindo ao longo da fita 3D
    let stream_pos = fract(in.uv.y * 1.35 - time * speed * 0.22 + fract(seed * 0.73));
    // stream_pos próximo de 0.0 é a cabeça brilhante da gota; valores maiores são a cauda
    let trail = pow(1.0 - stream_pos, 3.2);
    let is_head = smoothstep(0.06, 0.0, stream_pos);

    // Cor verde fosforescente Matrix + ponta branco-esverdeada brilhante
    let tail_color = vec3<f32>(0.04, 0.92, 0.28) * trail;
    let head_color = vec3<f32>(0.82, 1.0, 0.88) * is_head;
    let final_rgb = tail_color + head_color;

    // Fade nas extremidades superior e inferior da fita 3D
    let vertical_edge_fade = smoothstep(0.0, 0.12, in.uv.y) * smoothstep(1.0, 0.88, in.uv.y);

    // Vetor da câmera até este fragmento de código Matrix
    let to_frag = in.world_pos - globals.camera_pos.xyz;
    let cam_dist = length(to_frag);
    let frag_dir = to_frag / max(cam_dist, 0.0001);

    // 1. CAMPO DE VISÃO LIMPA MÍNIMO (Esfera pessoal + Cone frontal de leitura limpo)
    let forward_align = pow(max(dot(frag_dir, globals.camera_forward.xyz), 0.0), 3.0);
    let min_clear_dist = mix(5.5, globals.camera_forward.w, forward_align);
    let near_fade = smoothstep(min_clear_dist, min_clear_dist + 2.8, cam_dist);
    let far_fade = smoothstep(42.0, 12.0, cam_dist);

    // 2. CORREDOR LIMPO DAS JANELAS: impede que códigos Matrix passem na frente das janelas
    var window_clear_mask = 1.0;
    for (var i = 0u; i < 4u; i = i + 1u) {
        let win_pos = globals.window_shields[i].xyz;
        let shield_cos = globals.window_shields[i].w;
        let to_win = win_pos - globals.camera_pos.xyz;
        let win_dist = length(to_win);
        let win_dir = to_win / max(win_dist, 0.0001);

        // Se o código Matrix estiver entre a câmera e a janela (ou colado nela) dentro do cone da janela
        if (cam_dist < win_dist + 1.8) {
            let align = dot(frag_dir, win_dir);
            let cone_fade = smoothstep(shield_cos + 0.035, shield_cos - 0.025, align);
            window_clear_mask = min(window_clear_mask, cone_fade);
        }
    }

    let alpha = clamp(
        (trail * 0.85 + is_head * 0.95) * vertical_edge_fade * near_fade * far_fade * window_clear_mask * alpha_boost,
        0.0,
        0.95
    );
    if (alpha < 0.03) {
        discard;
    }

    return vec4<f32>(final_rgb, alpha);
}

// ============================================================================
// 3. SHADER DAS JANELAS 3D CURVAS NA ESFERA
// ============================================================================
struct WindowUniform {
    // [window_id, is_hovered, is_focused, opacity]
    state: vec4<f32>,
};

@group(1) @binding(0)
var win_tex: texture_2d<f32>;
@group(1) @binding(1)
var win_sampler: sampler;
@group(1) @binding(2)
var<uniform> win_uniform: WindowUniform;

struct WinIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
};

struct WinOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) world_pos: vec3<f32>,
};

@vertex
fn vs_window(in: WinIn) -> WinOut {
    var out: WinOut;
    out.clip_pos = globals.view_proj * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    out.normal = in.normal;
    out.world_pos = in.position;
    return out;
}

@fragment
fn fs_window(in: WinOut) -> @location(0) vec4<f32> {
    let tex_color = textureSample(win_tex, win_sampler, in.uv);
    let is_hovered = win_uniform.state.y;
    let is_focused = win_uniform.state.z;

    // Brilho de borda quando a janela 3D está sob o cursor ou focada na esfera
    let edge_x = min(in.uv.x, 1.0 - in.uv.x);
    let edge_y = min(in.uv.y, 1.0 - in.uv.y);
    let edge_dist = min(edge_x, edge_y);
    let border_glow = smoothstep(0.035, 0.0, edge_dist) * (0.25 + 0.55 * is_hovered + 0.45 * is_focused);

    let glow_rgb = vec3<f32>(0.25, 1.0, 0.55) * border_glow;
    let rgb = tex_color.rgb + glow_rgb;
    // Janela em uso (focada ou sob o cursor) fica totalmente nítida e opaca para leitura limpa
    let use_boost = max(is_hovered, is_focused) * 0.12;
    let alpha = clamp(tex_color.a * (win_uniform.state.w + use_boost) + border_glow * 0.6, 0.0, 0.99);

    return vec4<f32>(rgb, alpha);
}

// ============================================================================
// 4. SHADER DO FUNDO ORB HOLOGRÁFICO DO MINI-GLOBO (HUD)
// ============================================================================
struct MinimapBgOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_minimap_bg(@builtin(vertex_index) vid: u32) -> MinimapBgOut {
    // Quad cobrindo o viewport do Mini-Globo em 2 triângulos (6 vértices)
    var pos = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0,  1.0)
    );
    let p = pos[vid];
    var out: MinimapBgOut;
    // z = 0.999 limpa o depth buffer do viewport do Mini-Globo para renderizar a esfera 3D por cima
    out.clip_pos = vec4<f32>(p.x, p.y, 0.999, 1.0);
    out.ndc = p;
    return out;
}

@fragment
fn fs_minimap_bg(in: MinimapBgOut) -> @location(0) vec4<f32> {
    let r = length(in.ndc);
    if (r > 0.96) {
        discard;
    }

    // Sombra esférica suave sem anéis artificiais nem pontos na borda;
    // limpa o depth buffer local para renderizar o globo 3D e as janelas em suas profundidades reais.
    let edge_fade = smoothstep(0.96, 0.78, r);
    let base_rgb = vec3<f32>(0.004, 0.022, 0.010);
    return vec4<f32>(base_rgb, 0.62 * edge_fade);
}

