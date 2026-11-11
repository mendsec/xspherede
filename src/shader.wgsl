struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    // [time_secs, active_window_id, hovered_window_id, matrix_intensity]
    params: vec4<f32>,
    // xyz = camera forward vector, w = frontal clear field-of-view distance
    camera_forward: vec4<f32>,
    // For each window: xyz = 3D world center of the window, w = cosine of the angular protection cone
    window_shields: array<vec4<f32>, 4>,
};

@group(0) @binding(0)
var<uniform> globals: Globals;

// ============================================================================
// 1. SPHERICAL GRID SHADER (SPHEREXP WIREFRAME GLOBE)
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
// 2. 3D VOLUMETRIC MATRIX CODE SHADER
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

// Generates procedural Matrix-style glyphs (stylized half-width Katakana / binary) on a 5x7 grid
fn procedural_matrix_glyph(cell_uv: vec2<f32>, glyph_id: f32) -> f32 {
    // Padding between characters
    if (cell_uv.x < 0.14 || cell_uv.x > 0.86 || cell_uv.y < 0.10 || cell_uv.y > 0.90) {
        return 0.0;
    }
    let gx = floor(((cell_uv.x - 0.14) / 0.72) * 5.0);
    let gy = floor(((cell_uv.y - 0.10) / 0.80) * 7.0);

    // Partial symmetry + horizontal/vertical strokes typical of half-width Katakana
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

    // Vertical coordinate in glyph cells
    let row_f = in.uv.y * glyphs_y;
    let row_idx = floor(row_f);
    let cell_uv = vec2<f32>(in.uv.x, fract(row_f));

    // Some glyphs mutate rapidly just like in The Matrix film
    let morph_step = floor(time * (2.5 + fract(seed) * 4.0));
    let mutating = step(0.82, hash12(vec2<f32>(row_idx, seed)));
    let glyph_id = hash12(vec2<f32>(row_idx + seed * 11.0, mutating * morph_step));

    let pixel_on = procedural_matrix_glyph(cell_uv, glyph_id);
    if (pixel_on < 0.5) {
        discard;
    }

    // Position of the falling code drop along the 3D ribbon
    let stream_pos = fract(in.uv.y * 1.35 - time * speed * 0.22 + fract(seed * 0.73));
    // stream_pos near 0.0 is the bright leading head of the drop; larger values form the trail
    let trail = pow(1.0 - stream_pos, 3.2);
    let is_head = smoothstep(0.06, 0.0, stream_pos);

    // Phosphorescent Matrix green + bright greenish-white leading tip
    let tail_color = vec3<f32>(0.04, 0.92, 0.28) * trail;
    let head_color = vec3<f32>(0.82, 1.0, 0.88) * is_head;
    let final_rgb = tail_color + head_color;

    // Fade at the top and bottom edges of the 3D ribbon
    let vertical_edge_fade = smoothstep(0.0, 0.12, in.uv.y) * smoothstep(1.0, 0.88, in.uv.y);

    // Vector from the camera to this Matrix code fragment
    let to_frag = in.world_pos - globals.camera_pos.xyz;
    let cam_dist = length(to_frag);
    let frag_dir = to_frag / max(cam_dist, 0.0001);

    // 1. MINIMUM CLEAR FIELD OF VIEW (Personal sphere + clean frontal reading cone)
    let forward_align = pow(max(dot(frag_dir, globals.camera_forward.xyz), 0.0), 3.0);
    let min_clear_dist = mix(5.5, globals.camera_forward.w, forward_align);
    let near_fade = smoothstep(min_clear_dist, min_clear_dist + 2.8, cam_dist);
    let far_fade = smoothstep(42.0, 12.0, cam_dist);

    // 2. CLEAN WINDOW CORRIDOR: prevents Matrix code from passing in front of windows
    var window_clear_mask = 1.0;
    for (var i = 0u; i < 4u; i = i + 1u) {
        let win_pos = globals.window_shields[i].xyz;
        let shield_cos = globals.window_shields[i].w;
        let to_win = win_pos - globals.camera_pos.xyz;
        let win_dist = length(to_win);
        let win_dir = to_win / max(win_dist, 0.0001);

        // If the Matrix code is between the camera and the window (or right next to it) within the window cone
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
// 3. CURVED 3D SPHERICAL WINDOW SHADER (QUBES-OS UNFORGEABLE TRUST BORDERS)
// ============================================================================
struct WindowUniform {
    // [window_id, is_hovered, is_focused, opacity]
    state: vec4<f32>,
    // [trust_r, trust_g, trust_b, lockdown_active]
    // Controlled exclusively by the trusted XSphereDE compositor; untrusted client textures cannot alter it.
    trust_color: vec4<f32>,
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
    let trust_rgb = win_uniform.trust_color.rgb;
    let lockdown_active = win_uniform.trust_color.w;

    // Distance to the closest window edge in UV space
    let edge_x = min(in.uv.x, 1.0 - in.uv.x);
    let edge_y = min(in.uv.y, 1.0 - in.uv.y);
    let edge_dist = min(edge_x, edge_y);

    // 1. UNFORGEABLE COMPOSITOR SECURITY FRAME (Qubes OS Trusted Window Manager Architecture):
    // The outer frame and top trust-seal strip are forcefully rendered by the compositor shader,
    // making it mathematically impossible for a compromised client surface to spoof its TrustDomain color.
    let is_hard_frame = edge_dist < 0.014 || in.uv.y < 0.024;
    if (is_hard_frame) {
        let focus_boost = 0.78 + 0.22 * max(is_hovered, is_focused);
        let seal_stripe = select(1.0, 0.72, in.uv.y < 0.024 && fract(in.uv.x * 36.0) > 0.65);
        return vec4<f32>(trust_rgb * focus_boost * seal_stripe, 0.98);
    }

    // 2. INSTANT LOCKDOWN SHIELD (Key 'L'):
    // Blanks client window contents behind an encrypted-vault grid pattern while preserving domain color.
    var client_rgb = tex_color.rgb;
    if (lockdown_active > 0.5) {
        let grid_x = step(0.92, fract(in.uv.x * 28.0));
        let grid_y = step(0.92, fract(in.uv.y * 18.0));
        let lock_pattern = max(grid_x, grid_y) * 0.28;
        client_rgb = vec3<f32>(0.01, 0.03, 0.02) + trust_rgb * (0.08 + lock_pattern);
    }

    // 3. Domain-colored halo glow when the 3D window is hovered or focused on the sphere
    let border_glow = smoothstep(0.042, 0.012, edge_dist) * (0.30 + 0.55 * is_hovered + 0.45 * is_focused);
    let glow_rgb = trust_rgb * border_glow;
    let rgb = client_rgb + glow_rgb;

    // Active window (focused or hovered) becomes crisp and opaque for clean reading
    let use_boost = max(is_hovered, is_focused) * 0.12;
    let alpha = clamp(tex_color.a * (win_uniform.state.w + use_boost) + border_glow * 0.6, 0.0, 0.99);

    return vec4<f32>(rgb, alpha);
}

// ============================================================================
// 4. HOLOGRAPHIC ORB BACKGROUND SHADER FOR THE MINI-GLOBE (HUD)
// ============================================================================
struct MinimapBgOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_minimap_bg(@builtin(vertex_index) vid: u32) -> MinimapBgOut {
    // Full-viewport quad for the Mini-Globe in 2 triangles (6 vertices)
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
    // z = 0.999 clears the Mini-Globe viewport depth buffer so the 3D sphere renders cleanly on top
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

    // Smooth spherical backdrop without artificial rings or border dots;
    // clears the local depth buffer to render the 3D globe and windows at their true radial depths.
    let edge_fade = smoothstep(0.96, 0.78, r);
    let base_rgb = vec3<f32>(0.004, 0.022, 0.010);
    return vec4<f32>(base_rgb, 0.62 * edge_fade);
}

