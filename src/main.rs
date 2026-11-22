#![forbid(unsafe_code)]

mod camera;
mod galaxy;
mod minimap;
mod overview;
mod security;
mod sphere_env;
mod warp;
mod window3d;

use bytemuck::{Pod, Zeroable};
use camera::SphereCamera;
use galaxy::{
    cycle_galaxy, demo_universe, galaxy_at, spherical_to_local, Galaxy, GalaxyId, GalaxyVisibility,
    HaloStyle, ViewContext, GALAXY_RADIUS, PRIME_GALAXY, UNIVERSE_RADIUS,
};
use minimap::MiniGlobeHud;
use overview::{pick_galaxy, trust_links, CosmicOverview};
use security::{
    validate_surface_buffer, verify_shader_source_integrity, SecureClipboardVault,
    SecurityAuditReport, SecurityPolicy, TrustDomain,
};
use sphere_env::{generate_halo_grid, generate_matrix_code_volume, GridVertex, MatrixVertex};
use std::collections::HashSet;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use warp::{aim_target, WarpGesture, WarpJump};
use wgpu::util::DeviceExt;
use window3d::{generate_demo_window_rgba, SphericalWindow, WindowVertex};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct GlobalsUniform {
    view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    params: [f32; 4],
    camera_forward: [f32; 4],
    window_shields: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct WindowStateUniform {
    state: [f32; 4],
    trust_color: [f32; 4],
}

struct WindowRenderEntry {
    win: SphericalWindow,
    state_buffer: wgpu::Buffer,
}

/// Non-GPU pointer, keyboard, and window-interaction state carried across renderer rebuilds.
struct InteractionState {
    cursor_pos: (f32, f32),
    last_cursor_pos: Option<(f32, f32)>,
    right_mouse_down: bool,
    left_mouse_down: bool,
    middle_mouse_down: bool,
    free_look_mode: bool,
    shift_held: bool,
    ctrl_held: bool,
    keys_pressed: HashSet<KeyCode>,
    hovered_window: Option<usize>,
    focused_window: Option<usize>,
    dragging_window: Option<usize>,
    minimap_dragging: bool,
    minimap_drag_distance: f32,
    /// Left + right mouse chord for intergalactic travel.
    warp: WarpGesture,
    /// Button press in the Cosmic Zoom-Out overview, until it is released.
    overview_press: Option<OverviewPress>,
}

/// A pointer press inside the Cosmic Zoom-Out overview.
#[derive(Copy, Clone, Debug, PartialEq)]
struct OverviewPress {
    /// Window under the pointer when the left button was pressed: dragging moves it.
    window: Option<usize>,
    /// Pointer travel since the press, to tell a click from a drag.
    travel_px: f32,
}

/// Pointer travel below which a press and release count as a click.
const CLICK_SLOP_PX: f32 = 6.0;

impl InteractionState {
    fn new(width: u32, height: u32) -> Self {
        Self {
            cursor_pos: (width as f32 * 0.5, height as f32 * 0.5),
            last_cursor_pos: None,
            right_mouse_down: false,
            left_mouse_down: false,
            middle_mouse_down: false,
            free_look_mode: false,
            shift_held: false,
            ctrl_held: false,
            keys_pressed: HashSet::new(),
            hovered_window: None,
            focused_window: Some(0),
            dragging_window: None,
            minimap_dragging: false,
            minimap_drag_distance: 0.0,
            warp: WarpGesture::default(),
            overview_press: None,
        }
    }

    /// Releases held keys, modifiers, mouse buttons, and active drags so they are not
    /// carried over as stuck if release events are missed while rendering is suspended.
    fn release_held(&mut self) {
        self.keys_pressed.clear();
        self.left_mouse_down = false;
        self.right_mouse_down = false;
        self.middle_mouse_down = false;
        self.shift_held = false;
        self.ctrl_held = false;
        self.dragging_window = None;
        self.minimap_dragging = false;
        self.warp.cancel();
        self.overview_press = None;
    }
}

/// Returns `true` when enough time has elapsed since `last_attempt` to retry recovery.
fn should_attempt_recovery(
    last_attempt: Option<Instant>,
    now: Instant,
    interval: Duration,
) -> bool {
    last_attempt.is_none_or(|last| now.duration_since(last) >= interval)
}

struct SpheradianRenderer {
    window: Arc<Window>,
    instance: wgpu::Instance,
    /// `None` only between losing the surface and successfully recreating it.
    surface: Option<wgpu::Surface<'static>>,
    device: wgpu::Device,
    /// Set by wgpu when the device is lost. Every GPU resource in this renderer is
    /// then invalid and the renderer must be rebuilt (see `SpheradianApp::recover_from_device_loss`).
    device_lost: Arc<AtomicBool>,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth_view: wgpu::TextureView,

    camera: SphereCamera,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,

    // 3D Mini-Globe HUD (in the bottom-right corner)
    minimap: MiniGlobeHud,
    minimap_globals_buffer: wgpu::Buffer,
    minimap_globals_bind_group: wgpu::BindGroup,
    minimap_bg_pipeline: wgpu::RenderPipeline,

    grid_pipeline: wgpu::RenderPipeline,
    grid_vertex_buffer: wgpu::Buffer,
    /// Vertex range of the universe boundary in `grid_vertex_buffer`.
    universe_grid_range: Range<u32>,
    /// Vertex range of each galaxy's halo in `grid_vertex_buffer`, indexed by galaxy id.
    galaxy_grid_ranges: Vec<Range<u32>>,
    /// Vertex range of each galaxy's sparse, faint beacon outline, indexed by galaxy id.
    galaxy_beacon_ranges: Vec<Range<u32>>,

    matrix_pipeline: wgpu::RenderPipeline,
    matrix_vertex_buffer: wgpu::Buffer,
    matrix_index_buffer: wgpu::Buffer,
    /// Index range of each galaxy's code rain in `matrix_index_buffer`, indexed by galaxy id.
    galaxy_matrix_ranges: Vec<Range<u32>>,

    window_pipeline: wgpu::RenderPipeline,
    windows: Vec<WindowRenderEntry>,

    /// Clusters of windows inside the universe (the outer sphere is the shape of the universe).
    galaxies: Vec<Galaxy>,
    /// Galaxy the viewer is currently in.
    current_galaxy: GalaxyId,
    /// Destination of the intergalactic flight in progress, if any.
    travel_destination: Option<GalaxyId>,
    /// Window carried along by the current warp; it moves to the destination on arrival.
    travel_cargo: Option<usize>,
    /// How much of each galaxy the main view draws, refreshed every frame.
    galaxy_visibility: Vec<GalaxyVisibility>,
    /// Whether the main view shows the universe boundary and other galaxies.
    universe_revealed: bool,
    /// Window to focus when the current flight arrives (picked in the overview).
    travel_focus: Option<usize>,
    /// The Cosmic Zoom-Out overview of the whole universe.
    overview: CosmicOverview,
    /// Galaxy under the pointer in the overview; the others are dimmed.
    overview_spotlight: Option<GalaxyId>,
    /// Trust links between windows, drawn as lines in the overview.
    constellation_buffer: wgpu::Buffer,
    constellation_vertex_count: u32,
    /// Armed state last shown in the space warp hint, to avoid retitling every frame.
    warp_hint_shown: Option<bool>,

    // Hardened Security Subsystem (secureblue + Qubes OS policies)
    security_policy: SecurityPolicy,
    clipboard_vault: SecureClipboardVault,
    lockdown_active: bool,

    start_time: Instant,
    last_frame: Instant,
    matrix_speed_mult: f32,

    // 3D spatial and Mini-Globe interaction state
    interaction: InteractionState,
}

impl SpheradianRenderer {
    const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

    fn create_depth_view(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Spheradian Depth Buffer"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// Builds the renderer and every GPU resource it owns for `window`.
    ///
    /// Returns an error when the GPU cannot be initialized (no surface, adapter, or
    /// device). Security invariant failures still abort.
    async fn new(window: Arc<Window>) -> Result<Self, String> {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..wgpu::InstanceDescriptor::new_with_display_handle(Box::new(window.clone()))
        });

        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("Failed to create wgpu surface: {e}"))?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|e| format!("No compatible GPU adapter found: {e}"))?;

        let adapter_info = adapter.get_info();
        log::info!(
            "Selected GPU for Spheradian: {:?} (backend: {:?})",
            adapter_info.name,
            adapter_info.backend
        );

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Spheradian Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| format!("Failed to initialize graphics device: {e}"))?;

        let device_lost = Arc::new(AtomicBool::new(false));
        {
            let device_lost = device_lost.clone();
            device.set_device_lost_callback(move |reason, message| {
                log::error!("GPU device lost ({reason:?}): {message}");
                device_lost.store(true, Ordering::Release);
            });
        }

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let present_mode = if surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::Mailbox)
        {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            color_space: wgpu::SurfaceColorSpace::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let depth_view = Self::create_depth_view(&device, width, height);
        let mut camera = SphereCamera::new(width, height);
        // The camera must see across the whole universe and never leave it.
        camera.far = UNIVERSE_RADIUS * 2.5;
        camera.bounds_radius = UNIVERSE_RADIUS - 4.0;
        let minimap = MiniGlobeHud::new(GALAXY_RADIUS);

        // Enforce maximum hardened security policy (secureblue + Qubes OS)
        let security_policy = SecurityPolicy::maximum_hardened();
        assert!(
            security_policy.is_fully_enforced(),
            "Spheradian requires all mandatory security policies to be enabled"
        );

        let shader_source = include_str!("shader.wgsl");
        if security_policy.verify_shader_integrity {
            let digest = verify_shader_source_integrity(shader_source).expect(
                "CRITICAL SECURITY ALERT: WGSL shader cryptographic integrity check failed",
            );
            log::info!(
                "Verified WGSL shader SHA-256 cryptographic integrity: {}",
                digest
            );
        }

        let audit_report = SecurityAuditReport::run_audit(&security_policy, shader_source);
        log::info!("[SECURITY AUDIT] {}", audit_report.summary_line());
        if !audit_report.dangerous_env_detected.is_empty() {
            log::warn!(
                "[SECURITY AUDIT] Untrusted/injectable environment variables detected: {:?}",
                audit_report.dangerous_env_detected
            );
        }

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Spheradian Unified Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        // Global uniform (main camera + elapsed time)
        let globals_uniform = GlobalsUniform {
            view_proj: camera.view_proj_matrix().to_cols_array_2d(),
            camera_pos: [0.0, 0.0, 0.0, 1.0],
            params: [0.0, -1.0, -1.0, 1.0],
            camera_forward: [0.0, 0.0, -1.0, 11.8],
            window_shields: [[0.0, 0.0, -10.0, 0.95]; 4],
        };

        let globals_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Globals Uniform Buffer"),
            contents: bytemuck::cast_slice(&[globals_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let minimap_eye = minimap.eye_position();
        let minimap_globals_uniform = GlobalsUniform {
            view_proj: minimap.view_proj_matrix().to_cols_array_2d(),
            camera_pos: [minimap_eye.x, minimap_eye.y, minimap_eye.z, 1.0],
            params: [0.0, 0.0, 0.0, 1.0],
            camera_forward: [0.0, 0.0, -1.0, 11.8],
            window_shields: [[0.0, 0.0, -10.0, 0.95]; 4],
        };

        let minimap_globals_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Minimap Globals Uniform Buffer"),
            contents: bytemuck::cast_slice(&[minimap_globals_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let globals_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Globals BindGroupLayout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Globals BindGroup"),
            layout: &globals_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });

        let minimap_globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Minimap Globals BindGroup"),
            layout: &globals_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: minimap_globals_buffer.as_entire_binding(),
            }],
        });

        // BindGroupLayout for curved 3D windows
        let window_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SphericalWindow BindGroupLayout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let basic_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Basic Pipeline Layout"),
                bind_group_layouts: &[Some(&globals_bgl)],
                immediate_size: 0,
            });

        let window_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Window Pipeline Layout"),
                bind_group_layouts: &[Some(&globals_bgl), Some(&window_bgl)],
                immediate_size: 0,
            });

        // 1. Spherical Grid Pipeline (SphereXP Wireframe)
        let grid_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("SphereXP Grid Pipeline"),
            layout: Some(&basic_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_grid"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GridVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_grid"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: Self::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // 2. 3D Volumetric Matrix Code Pipeline (Additive + Alpha Blend)
        let matrix_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Matrix Volumetric Code Pipeline"),
            layout: Some(&basic_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_matrix"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MatrixVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32x2,
                        2 => Float32x4
                    ],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_matrix"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::OVER,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: Self::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // 3. Curved 3D Spherical Windows Pipeline
        let window_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Spherical 3D Window Pipeline"),
            layout: Some(&window_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_window"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<WindowVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32x2,
                        2 => Float32x3
                    ],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_window"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: Self::DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // 4. Holographic Mini-Globe Background Pipeline (draws the orb and clears the HUD depth buffer)
        let minimap_bg_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("MiniGlobe Orb Background Pipeline"),
            layout: Some(&basic_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_minimap_bg"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_minimap_bg"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: Self::DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Sphere Grid and Matrix Code Volume Geometry
        let galaxies = demo_universe();
        let galaxy_count = galaxies.len();

        // The universe boundary first, then one halo per galaxy (ranges kept for the Mini-Globe).
        let mut grid_vertices =
            generate_halo_grid(glam::Vec3::ZERO, UNIVERSE_RADIUS, 24, 14, None, 0.45);
        let universe_grid_range = 0..grid_vertices.len() as u32;
        let mut galaxy_grid_ranges = Vec::with_capacity(galaxies.len());
        let mut galaxy_beacon_ranges = Vec::with_capacity(galaxies.len());
        for galaxy in &galaxies {
            let start = grid_vertices.len() as u32;
            grid_vertices.extend(generate_halo_grid(
                galaxy.center,
                GALAXY_RADIUS,
                24,
                14,
                galaxy.halo_tint,
                1.0,
            ));
            galaxy_grid_ranges.push(start..grid_vertices.len() as u32);
        }
        for galaxy in &galaxies {
            let start = grid_vertices.len() as u32;
            grid_vertices.extend(generate_halo_grid(
                galaxy.center,
                GALAXY_RADIUS,
                12,
                6,
                galaxy.halo_tint,
                0.4,
            ));
            galaxy_beacon_ranges.push(start..grid_vertices.len() as u32);
        }
        let grid_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Sphere Grid Vertex Buffer"),
            contents: bytemuck::cast_slice(&grid_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let mut matrix_vertices = Vec::new();
        let mut matrix_indices = Vec::new();
        let mut galaxy_matrix_ranges = Vec::with_capacity(galaxies.len());
        for galaxy in &galaxies {
            let start = matrix_indices.len() as u32;
            let (verts, idxs) = generate_matrix_code_volume(
                GALAXY_RADIUS,
                galaxy.center,
                galaxy.id as u32,
                matrix_vertices.len() as u32,
            );
            matrix_vertices.extend(verts);
            matrix_indices.extend(idxs);
            galaxy_matrix_ranges.push(start..matrix_indices.len() as u32);
        }
        let matrix_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Matrix Volume Vertex Buffer"),
            contents: bytemuck::cast_slice(&matrix_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let matrix_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Matrix Volume Index Buffer"),
            contents: bytemuck::cast_slice(&matrix_indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Demo windows, grouped into the galaxies of the demo universe
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Window Texture Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let demo_specs = [
            (
                PRIME_GALAXY,
                "Matrix Vault Terminal // run0",
                TrustDomain::CoreVault,
                -0.45_f32,
                0.12_f32,
                9.5_f32,
                0.56_f32,
                0.36_f32,
                0usize,
            ),
            (
                PRIME_GALAXY,
                "3D Security & Kernel Auditor",
                TrustDomain::TrustedWork,
                0.48_f32,
                0.15_f32,
                10.5_f32,
                0.54_f32,
                0.35_f32,
                1usize,
            ),
            (
                PRIME_GALAXY,
                "Spatial File Navigator (No-Thumbnails)",
                TrustDomain::IsolatedApp,
                -0.02_f32,
                -0.34_f32,
                8.8_f32,
                0.58_f32,
                0.36_f32,
                2usize,
            ),
            (
                PRIME_GALAXY,
                "Disposable Quarantine Sandbox (Rear)",
                TrustDomain::DisposableRed,
                2.55_f32,
                0.08_f32,
                10.2_f32,
                0.54_f32,
                0.38_f32,
                3usize,
            ),
            (
                1,
                "Andromeda Build Console",
                TrustDomain::TrustedWork,
                0.0_f32,
                0.10_f32,
                9.8_f32,
                0.56_f32,
                0.36_f32,
                0usize,
            ),
            (
                1,
                "Andromeda Telemetry Scope",
                TrustDomain::IsolatedApp,
                0.62_f32,
                -0.18_f32,
                10.6_f32,
                0.52_f32,
                0.34_f32,
                1usize,
            ),
            (
                2,
                "Triangulum Research Browser",
                TrustDomain::UntrustedNet,
                0.0_f32,
                0.05_f32,
                9.6_f32,
                0.58_f32,
                0.38_f32,
                2usize,
            ),
            (
                2,
                "Triangulum Download Quarantine",
                TrustDomain::DisposableRed,
                -0.60_f32,
                0.22_f32,
                10.8_f32,
                0.50_f32,
                0.34_f32,
                3usize,
            ),
            (
                3,
                "Sombrero Key Vault",
                TrustDomain::CoreVault,
                0.0_f32,
                -0.06_f32,
                9.2_f32,
                0.54_f32,
                0.36_f32,
                0usize,
            ),
        ];

        log::info!(
            "Universe initialized with {} galaxies; viewer starts in galaxy #{} '{}'",
            galaxies.len(),
            galaxies[PRIME_GALAXY].id,
            galaxies[PRIME_GALAXY].name
        );

        let mut windows = Vec::new();
        for (
            id,
            (galaxy_id, title, trust_domain, azimuth, elevation, radius, ang_w, ang_h, variant),
        ) in demo_specs.into_iter().enumerate()
        {
            let home_galaxy = &galaxies[galaxy_id];
            let tex_w = 512;
            let tex_h = 340;
            let rgba = generate_demo_window_rgba(tex_w, tex_h, variant, trust_domain);
            validate_surface_buffer(tex_w, tex_h, &rgba)
                .expect("Surface buffer validation failed against GPU safety bounds");

            let texture = device.create_texture_with_data(
                &queue,
                &wgpu::TextureDescriptor {
                    label: Some(title),
                    size: wgpu::Extent3d {
                        width: tex_w,
                        height: tex_h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &rgba,
            );
            let tex_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            let [tr, tg, tb] = trust_domain.border_color_rgb();
            let state_uniform = WindowStateUniform {
                state: [id as f32, 0.0, if id == 0 { 1.0 } else { 0.0 }, 0.94],
                trust_color: [tr, tg, tb, 0.0],
            };
            let state_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Window State Uniform"),
                contents: bytemuck::cast_slice(&[state_uniform]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Window Bind Group"),
                layout: &window_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&tex_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: state_buffer.as_entire_binding(),
                    },
                ],
            });

            let (verts, idxs) = SphericalWindow::build_curved_mesh(
                home_galaxy.center,
                azimuth,
                elevation,
                radius,
                ang_w,
                ang_h,
            );

            let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Window Curved Mesh VB"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
            let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Window Curved Mesh IB"),
                contents: bytemuck::cast_slice(&idxs),
                usage: wgpu::BufferUsages::INDEX,
            });

            windows.push(WindowRenderEntry {
                win: SphericalWindow {
                    id,
                    title: title.to_string(),
                    trust_domain,
                    galaxy: home_galaxy.id,
                    origin: home_galaxy.center,
                    azimuth,
                    elevation,
                    radius,
                    angular_width: ang_w,
                    angular_height: ang_h,
                    hovered: false,
                    focused: id == 0,
                    vertex_buffer,
                    index_buffer,
                    index_count: idxs.len() as u32,
                    bind_group,
                },
                state_buffer,
            });
        }

        let now = Instant::now();
        // Room for one trust link per window (a chain has fewer links than windows).
        let constellation_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Trust Constellation Vertex Buffer"),
            size: (windows.len().max(1) * 2 * std::mem::size_of::<GridVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            window,
            instance,
            surface: Some(surface),
            device,
            device_lost,
            queue,
            config,
            depth_view,
            camera,
            globals_buffer,
            globals_bind_group,
            minimap,
            minimap_globals_buffer,
            minimap_globals_bind_group,
            minimap_bg_pipeline,
            grid_pipeline,
            grid_vertex_buffer,
            universe_grid_range,
            galaxy_grid_ranges,
            galaxy_beacon_ranges,
            matrix_pipeline,
            matrix_vertex_buffer,
            matrix_index_buffer,
            galaxy_matrix_ranges,
            window_pipeline,
            windows,
            galaxies,
            current_galaxy: PRIME_GALAXY,
            travel_destination: None,
            travel_cargo: None,
            galaxy_visibility: ViewContext {
                current: PRIME_GALAXY,
                home: Some(PRIME_GALAXY),
                ..ViewContext::default()
            }
            .galaxy_visibility(galaxy_count),
            universe_revealed: false,
            travel_focus: None,
            overview: CosmicOverview::default(),
            overview_spotlight: None,
            constellation_buffer,
            constellation_vertex_count: 0,
            warp_hint_shown: None,
            security_policy,
            clipboard_vault: SecureClipboardVault::new(),
            lockdown_active: false,
            start_time: now,
            last_frame: now,
            matrix_speed_mult: 1.0,
            interaction: InteractionState::new(width, height),
        })
    }

    /// Carries the session over from `previous` into this freshly built renderer:
    /// camera, window placement, Mini-Globe, security state, and input state.
    ///
    /// GPU resources are not carried over; this renderer keeps the ones it created.
    fn adopt_session_state(&mut self, previous: SpheradianRenderer) {
        self.camera = previous.camera;
        self.minimap = previous.minimap;

        self.galaxies = previous.galaxies;
        self.current_galaxy = previous.current_galaxy;
        self.travel_destination = previous.travel_destination;
        self.travel_cargo = previous.travel_cargo;
        self.travel_focus = previous.travel_focus;
        self.overview = previous.overview;

        for (entry, old) in self.windows.iter_mut().zip(&previous.windows) {
            entry.win.galaxy = old.win.galaxy;
            entry.win.origin = old.win.origin;
            entry.win.azimuth = old.win.azimuth;
            entry.win.elevation = old.win.elevation;
            entry.win.radius = old.win.radius;
            entry.win.angular_width = old.win.angular_width;
            entry.win.angular_height = old.win.angular_height;
            entry.win.hovered = old.win.hovered;
            entry.win.focused = old.win.focused;
            entry.win.update_geometry(&self.queue);
        }

        self.security_policy = previous.security_policy;
        self.clipboard_vault = previous.clipboard_vault;
        self.lockdown_active = previous.lockdown_active;

        self.start_time = previous.start_time;
        self.last_frame = previous.last_frame;
        self.matrix_speed_mult = previous.matrix_speed_mult;

        self.interaction = previous.interaction;
    }

    /// Center of the galaxy the viewer is currently in, in universe coordinates.
    fn current_galaxy_center(&self) -> glam::Vec3 {
        self.galaxies
            .get(self.current_galaxy)
            .map_or(glam::Vec3::ZERO, |galaxy| galaxy.center)
    }

    /// Short "Name (i/n)" label of the current galaxy for the window title.
    fn galaxy_label(&self, id: GalaxyId) -> String {
        self.galaxies.get(id).map_or_else(
            || "deep space".to_string(),
            |galaxy| {
                format!(
                    "{} ({}/{})",
                    galaxy.name,
                    galaxy.id + 1,
                    self.galaxies.len()
                )
            },
        )
    }

    /// Title shown when nothing is hovered.
    fn idle_title(&self) -> String {
        format!(
            "Spheradian v{} — Galaxy {} | U: Cosmic Zoom-Out | Ctrl+PgUp/PgDn or Ctrl+1-9: Travel | Ctrl+Shift+C/V: Clipboard Vault | 'L': Lockdown | 'G': Mini-Globe",
            env!("CARGO_PKG_VERSION"),
            self.galaxy_label(self.current_galaxy)
        )
    }

    /// Starts an intergalactic flight to the center of `destination`.
    fn travel_to_galaxy(&mut self, destination: GalaxyId) {
        let Some(center) = self.galaxies.get(destination).map(|galaxy| galaxy.center) else {
            return;
        };
        self.leave_overview();
        self.travel_focus = None;
        let msg = format!(
            "Spheradian — Traveling to galaxy {}…",
            self.galaxy_label(destination)
        );
        log::info!("{}", msg);
        self.window.set_title(&msg);
        self.interaction.dragging_window = None;
        self.travel_destination = Some(destination);
        self.travel_cargo = None;
        self.camera.fly_to(center);
    }

    /// Performs a space warp requested by the left + right mouse chord.
    fn warp(&mut self, jump: WarpJump) {
        self.travel_to_galaxy(jump.destination);
        if self.travel_destination == Some(jump.destination) {
            self.travel_cargo = jump.cargo.filter(|&idx| idx < self.windows.len());
        }
    }

    /// Starts the space warp chord when both left and right buttons are now held.
    /// The chord never starts on the Mini-Globe, which has its own drag gestures.
    /// A window being dragged when the chord starts is carried to the destination.
    fn try_begin_warp_chord(&mut self, on_minimap: bool) -> bool {
        let input = &mut self.interaction;
        if !(input.left_mouse_down && input.right_mouse_down)
            || on_minimap
            || input.minimap_dragging
            || input.warp.is_active()
            || self.overview.is_active()
        {
            return false;
        }
        let cargo = input.dragging_window.take();
        input.warp.begin(cargo);
        self.warp_hint_shown = None;
        true
    }

    /// Abandons the chord without traveling.
    fn cancel_warp_chord(&mut self) {
        self.interaction.warp.cancel();
        let msg = "Spheradian [SPACE WARP] — Cancelled";
        log::info!("{}", msg);
        self.window.set_title(msg);
    }

    /// Ends the chord gesture: warps when it was armed and aimed, otherwise cancels it.
    fn release_warp_chord(&mut self) {
        match self.interaction.warp.release() {
            Some(jump) => self.warp(jump),
            None => self.window.set_title(&self.idle_title()),
        }
    }

    /// Re-aims the chord at the galaxy under the center of the view and shows the result.
    fn update_warp_aim(&mut self) {
        if !self.interaction.warp.is_active() {
            return;
        }
        let target = aim_target(&self.galaxies, self.camera.position, self.camera.forward());
        let armed = self.interaction.warp.is_armed();
        let was_shown = self.warp_hint_shown;
        let changed = self.interaction.warp.set_target(target);
        if !changed && was_shown == Some(armed) {
            return;
        }
        self.warp_hint_shown = Some(armed);
        let cargo = self
            .interaction
            .warp
            .cargo()
            .and_then(|idx| self.windows.get(idx))
            .map(|entry| format!(" carrying '{}'", entry.win.title))
            .unwrap_or_default();
        let msg = match (target, armed) {
            (Some(galaxy), true) => format!(
                "Spheradian [SPACE WARP] — Locked on galaxy {}{} | Release to warp, Esc to cancel",
                self.galaxy_label(galaxy),
                cargo
            ),
            (Some(galaxy), false) => format!(
                "Spheradian [SPACE WARP] — Aiming at galaxy {}{} | Keep dragging to lock, Esc to cancel",
                self.galaxy_label(galaxy),
                cargo
            ),
            (None, _) => format!(
                "Spheradian [SPACE WARP] — Drag to aim at a galaxy halo{} | Esc to cancel",
                cargo
            ),
        };
        self.window.set_title(&msg);
    }

    /// Completes a flight: the viewer now lives in `galaxy` and faces its first window.
    ///
    /// Focus is cleared on arrival rather than handed to a window of the new galaxy, so
    /// that traveling never silently moves keyboard or clipboard focus across trust domains.
    /// The only exceptions are windows the user explicitly chose: `cargo`, carried along by
    /// the warp chord, and `chosen`, clicked in the overview.
    fn arrive_at_galaxy(&mut self, galaxy: GalaxyId, cargo: Option<usize>, chosen: Option<usize>) {
        self.set_current_galaxy(galaxy);
        self.interaction.focused_window = None;
        // A carried window keeps its trust domain and its spherical placement; only its
        // galaxy changes.
        if let Some(idx) = cargo {
            self.move_window_to_galaxy(idx, galaxy);
        }
        let chosen = cargo.or(chosen).filter(|&idx| {
            self.windows
                .get(idx)
                .is_some_and(|e| e.win.galaxy == galaxy)
        });
        if chosen.is_some() {
            self.interaction.focused_window = chosen;
        }
        let look_at = chosen
            .and_then(|idx| self.windows.get(idx))
            .or_else(|| self.windows.iter().find(|entry| entry.win.galaxy == galaxy))
            .map(|entry| entry.win.world_center());
        if let Some(target) = look_at {
            self.camera.look_at_point(target);
        }
        let msg = format!(
            "Spheradian — Arrived at galaxy {}",
            self.galaxy_label(galaxy)
        );
        log::info!("{}", msg);
        self.window.set_title(&msg);
    }

    /// Moves a window into another galaxy. It keeps its trust domain and its spherical
    /// placement around the galaxy center.
    fn move_window_to_galaxy(&mut self, idx: usize, galaxy: GalaxyId) -> bool {
        let Some(center) = self.galaxies.get(galaxy).map(|g| g.center) else {
            return false;
        };
        let Some(entry) = self.windows.get_mut(idx) else {
            return false;
        };
        entry.win.galaxy = galaxy;
        entry.win.origin = center;
        entry.win.update_geometry(&self.queue);
        true
    }

    /// Opens the Cosmic Zoom-Out: the camera pulls straight back until every galaxy is in view.
    fn open_overview(&mut self) {
        if self.overview.is_active() {
            return;
        }
        self.interaction.warp.cancel();
        self.interaction.dragging_window = None;
        self.interaction.overview_press = None;
        self.travel_destination = None;
        self.travel_cargo = None;
        self.travel_focus = None;
        self.minimap.expanded = false;
        self.overview.open(
            &self.galaxies,
            self.camera.target_yaw,
            self.camera.target_pitch,
            self.camera.base_fov_y_radians,
            self.camera.aspect,
        );
        let reach = self.overview.max_reach() + GALAXY_RADIUS;
        self.camera.bounds_radius = reach;
        self.camera.far = reach * 2.0 + UNIVERSE_RADIUS;
        self.camera.near = 0.5;
        self.camera.fly_to(self.overview.eye());
        self.overview_spotlight = None;
        log::info!("Cosmic Zoom-Out opened");
        self.window.set_title(&self.overview_title());
    }

    /// Closes the overview and restores the camera limits used inside the universe.
    /// Callers then fly somewhere, usually into a galaxy.
    fn leave_overview(&mut self) {
        if !self.overview.is_active() {
            return;
        }
        self.overview.close();
        self.overview_spotlight = None;
        self.interaction.overview_press = None;
        self.interaction.hovered_window = None;
        self.camera.bounds_radius = UNIVERSE_RADIUS - 4.0;
        self.camera.far = UNIVERSE_RADIUS * 2.5;
        self.camera.near = 0.1;
    }

    /// Title bar text for the overview, describing what is under the pointer.
    fn overview_title(&self) -> String {
        let prefix = "Spheradian [COSMIC ZOOM-OUT]";
        let dragged = self
            .interaction
            .overview_press
            .filter(|press| press.travel_px >= CLICK_SLOP_PX)
            .and_then(|press| press.window)
            .and_then(|idx| self.windows.get(idx));
        if let Some(entry) = dragged {
            let drop = match self.overview_spotlight {
                Some(g) if g != entry.win.galaxy => {
                    format!("release to move it to {}", self.galaxy_label(g))
                }
                _ => "release over another galaxy to move it there".to_string(),
            };
            return format!(
                "{prefix} — Moving [{}] {} | {drop}",
                entry.win.trust_domain.badge_label(),
                entry.win.title
            );
        }
        if let Some(entry) = self
            .interaction
            .hovered_window
            .and_then(|idx| self.windows.get(idx))
        {
            return format!(
                "{prefix} — [{}] {} in {} | Click: dive in and focus it · Drag onto a galaxy: move it",
                entry.win.trust_domain.badge_label(),
                entry.win.title,
                self.galaxy_label(entry.win.galaxy)
            );
        }
        if let Some(galaxy) = self.overview_spotlight {
            let mut domains: Vec<_> = self
                .windows
                .iter()
                .filter(|entry| entry.win.galaxy == galaxy)
                .map(|entry| entry.win.trust_domain)
                .collect();
            let count = domains.len();
            domains.sort();
            domains.dedup();
            let badges: Vec<_> = domains.iter().rev().map(|d| d.badge_label()).collect();
            return format!(
                "{prefix} — Galaxy {}: {count} window(s) · {} | Click: dive in",
                self.galaxy_label(galaxy),
                if badges.is_empty() {
                    "empty".to_string()
                } else {
                    badges.join(" ")
                }
            );
        }
        format!(
            "{prefix} — {} galaxies | Drag: orbit · Scroll: zoom · Click a galaxy: dive in · U/Esc: back",
            self.galaxies.len()
        )
    }

    /// Pointer motion in the overview: orbits, or drags a window toward another galaxy.
    fn overview_pointer_moved(&mut self, dx: f32, dy: f32) {
        let Some(press) = self.interaction.overview_press.as_mut() else {
            return;
        };
        press.travel_px += (dx * dx + dy * dy).sqrt();
        if press.window.is_none() {
            self.overview.orbit(dx, dy);
        } else if press.travel_px >= CLICK_SLOP_PX {
            self.window.set_title(&self.overview_title());
        }
    }

    /// Mouse buttons in the overview. A click dives into a galaxy (focusing the clicked
    /// window, if any); dragging empty space orbits; dragging a window onto another
    /// galaxy moves it there.
    fn overview_button(&mut self, button: MouseButton, pressed: bool) {
        match button {
            MouseButton::Left => self.interaction.left_mouse_down = pressed,
            MouseButton::Right => self.interaction.right_mouse_down = pressed,
            MouseButton::Middle => self.interaction.middle_mouse_down = pressed,
            _ => return,
        }
        if pressed {
            if self.interaction.overview_press.is_none() {
                let window = if button == MouseButton::Left {
                    self.interaction.hovered_window
                } else {
                    None
                };
                self.interaction.overview_press = Some(OverviewPress {
                    window,
                    travel_px: 0.0,
                });
            }
            return;
        }
        let Some(press) = self.interaction.overview_press.take() else {
            return;
        };
        if press.travel_px < CLICK_SLOP_PX {
            if let Some(idx) = self.interaction.hovered_window {
                let galaxy = self.windows[idx].win.galaxy;
                self.travel_to_galaxy(galaxy);
                self.travel_focus = Some(idx);
            } else if let Some(galaxy) = self.overview_spotlight {
                self.travel_to_galaxy(galaxy);
            }
        } else if let Some(idx) = press.window {
            let from = self.windows[idx].win.galaxy;
            if let Some(to) = self.overview_spotlight.filter(|&g| g != from) {
                if self.move_window_to_galaxy(idx, to) {
                    log::info!(
                        "Moved window '{}' from {} to {}",
                        self.windows[idx].win.title,
                        self.galaxy_label(from),
                        self.galaxy_label(to)
                    );
                }
            }
            self.window.set_title(&self.overview_title());
        }
    }

    /// Rebuilds the trust constellation lines between windows of the same trust domain.
    fn update_constellations(&mut self) {
        let nodes: Vec<_> = self
            .windows
            .iter()
            .map(|entry| (entry.win.trust_domain, entry.win.galaxy))
            .collect();
        let spotlight = self.overview_spotlight;
        let mut vertices = Vec::new();
        for (a, b) in trust_links(&nodes) {
            let (wa, wb) = (&self.windows[a].win, &self.windows[b].win);
            let [r, g, bl] = wa.trust_domain.border_color_rgb();
            let alpha = match spotlight {
                None => 0.7,
                Some(spot) if wa.galaxy == spot || wb.galaxy == spot => 0.95,
                Some(_) => 0.08,
            };
            for point in [wa.world_center(), wb.world_center()] {
                vertices.push(GridVertex {
                    position: point.to_array(),
                    color: [r, g, bl, alpha],
                });
            }
        }
        let capacity =
            (self.constellation_buffer.size() / std::mem::size_of::<GridVertex>() as u64) as usize;
        vertices.truncate(capacity - capacity % 2);
        if !vertices.is_empty() {
            self.queue.write_buffer(
                &self.constellation_buffer,
                0,
                bytemuck::cast_slice(&vertices),
            );
        }
        self.constellation_vertex_count = vertices.len() as u32;
    }

    /// Decides how much of each galaxy the main view draws this frame.
    fn update_galaxy_visibility(&mut self) {
        let view = ViewContext {
            current: self.current_galaxy,
            home: galaxy_at(&self.galaxies, self.camera.position),
            aiming: self.interaction.warp.is_active(),
            aim: self.interaction.warp.target(),
            destination: self.travel_destination,
            overview: self.overview.is_active(),
            spotlight: self.overview_spotlight,
        };
        self.universe_revealed = view.reveals_universe();
        self.galaxy_visibility = view.galaxy_visibility(self.galaxies.len());
    }

    /// Whether the windows of `galaxy` are drawn (and can be pointed at) in the main view.
    fn shows_galaxy_windows(&self, galaxy: GalaxyId) -> bool {
        self.galaxy_visibility
            .get(galaxy)
            .is_some_and(|visibility| visibility.windows)
    }

    /// Makes `galaxy` the current one and points the Mini-Globe at it.
    fn set_current_galaxy(&mut self, galaxy: GalaxyId) {
        if let Some(center) = self.galaxies.get(galaxy).map(|g| g.center) {
            self.current_galaxy = galaxy;
            self.minimap.target_focus = center;
        }
    }

    fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            if let Some(surface) = &self.surface {
                surface.configure(&self.device, &self.config);
            }
            self.depth_view =
                Self::create_depth_view(&self.device, new_size.width, new_size.height);
            self.camera.resize(new_size.width, new_size.height);
        }
    }

    /// Performs 3D raycasting against the windows (at their true radial depths `radius`) using the Mini-Globe ray.
    fn raycast_window_in_minimap(&self, px: f32, py: f32) -> Option<usize> {
        let (ray_origin, ray_dir) =
            self.minimap
                .screen_ray(px, py, self.config.width, self.config.height)?;
        let mut best_hit: Option<(usize, f32)> = None;
        for (idx, entry) in self.windows.iter().enumerate() {
            // The Mini-Globe shows only the current galaxy.
            if entry.win.galaxy != self.current_galaxy {
                continue;
            }
            if let Some(t) = entry.win.intersect_ray(ray_origin, ray_dir) {
                if best_hit.is_none_or(|(_, best_t)| t < best_t) {
                    best_hit = Some((idx, t));
                }
            }
        }
        best_hit.map(|(idx, _)| idx)
    }

    /// When the user clicks inside the Mini-Globe, smoothly directs the 3D environment there.
    fn jump_to_minimap_point(&mut self, px: f32, py: f32) {
        if let Some(win_idx) = self.raycast_window_in_minimap(px, py) {
            self.interaction.focused_window = Some(win_idx);
            let target = self.windows[win_idx].win.world_center();
            self.camera.look_at_point(target);
            if self.minimap.expanded {
                self.minimap.expanded = false;
            }
        } else if let Some((click_az, click_el)) =
            self.minimap
                .pick_spherical_point(px, py, self.config.width, self.config.height)
        {
            let target = self.minimap.target_focus
                + spherical_to_local(click_az, click_el, self.minimap.sphere_radius);
            self.camera.look_at_point(target);
            if self.minimap.expanded {
                self.minimap.expanded = false;
            }
        }
    }

    /// Converts the current mouse position into NDC `[-1, 1]` coordinates and performs 3D raycasting
    /// against windows (both in the main view and at their true radial depths inside the Mini-Globe).
    fn update_raycast_hover(&mut self) {
        let w = self.config.width.max(1);
        let h = self.config.height.max(1);

        // 1. First check whether the mouse cursor is hovering over the Mini-Globe
        self.minimap.is_hovered = !self.overview.is_active()
            && self.minimap.contains_cursor(
                self.interaction.cursor_pos.0,
                self.interaction.cursor_pos.1,
                w,
                h,
            );

        if self.minimap.is_hovered {
            let hovered_win = self.raycast_window_in_minimap(
                self.interaction.cursor_pos.0,
                self.interaction.cursor_pos.1,
            );
            self.interaction.hovered_window = hovered_win;
            if let Some(idx) = hovered_win {
                let win = &self.windows[idx].win;
                self.window.set_title(&format!(
                    "Spheradian v{} [Mini-Globe] — [{}] {} (Depth R: {:.1}m — Scroll to adjust depth!)",
                    env!("CARGO_PKG_VERSION"),
                    win.trust_domain.badge_label(),
                    win.title,
                    win.radius
                ));
            } else if let Some((az, el)) = self.minimap.pick_spherical_point(
                self.interaction.cursor_pos.0,
                self.interaction.cursor_pos.1,
                w,
                h,
            ) {
                self.window.set_title(&format!(
                    "Spheradian v{} [Mini-Globe] — Left Click: Jump to ({:.0}°, {:.0}°) | Right Click or 'G': Expand/Collapse Globe",
                    env!("CARGO_PKG_VERSION"),
                    az.to_degrees(),
                    el.to_degrees()
                ));
            }
        } else {
            // 2. Standard 3D window raycasting in the main scene
            let ndc_x = (self.interaction.cursor_pos.0 / w as f32) * 2.0 - 1.0;
            let ndc_y = 1.0 - (self.interaction.cursor_pos.1 / h as f32) * 2.0;
            let (ray_origin, ray_dir) = self.camera.screen_ray(ndc_x, ndc_y);

            let mut best_hit: Option<(usize, f32)> = None;
            for (idx, entry) in self.windows.iter().enumerate() {
                // Windows that are not drawn cannot be hovered or clicked.
                if !self.shows_galaxy_windows(entry.win.galaxy) {
                    continue;
                }
                if let Some(t) = entry.win.intersect_ray(ray_origin, ray_dir) {
                    if best_hit.is_none_or(|(_, best_t)| t < best_t) {
                        best_hit = Some((idx, t));
                    }
                }
            }

            let hit = best_hit.map(|(idx, _)| idx);
            if self.overview.is_active() {
                let spotlight = hit
                    .map(|idx| self.windows[idx].win.galaxy)
                    .or_else(|| pick_galaxy(&self.galaxies, ray_origin, ray_dir));
                if hit != self.interaction.hovered_window || spotlight != self.overview_spotlight {
                    self.interaction.hovered_window = hit;
                    self.overview_spotlight = spotlight;
                    self.window.set_title(&self.overview_title());
                }
            } else if self.interaction.hovered_window != hit {
                self.interaction.hovered_window = hit;
                if self.interaction.warp.is_active() {
                    // The space warp hint owns the title while the chord is held.
                } else if let Some(idx) = self.interaction.hovered_window {
                    let win = &self.windows[idx].win;
                    let net_tag = if win.trust_domain.allows_network() {
                        "NET-SANDBOXED"
                    } else {
                        "AIR-GAPPED"
                    };
                    self.window.set_title(&format!(
                        "Spheradian v{} — [{} | {}] {} in {} (Azimuth: {:.1}°, Elevation: {:.1}°, Depth R: {:.1}m)",
                        env!("CARGO_PKG_VERSION"),
                        win.trust_domain.badge_label(),
                        net_tag,
                        win.title,
                        self.galaxies.get(win.galaxy).map_or("deep space", |g| g.name.as_str()),
                        win.azimuth.to_degrees(),
                        win.elevation.to_degrees(),
                        win.radius
                    ));
                } else {
                    self.window.set_title(&self.idle_title());
                }
            }
        }

        let lockdown_flag = if self.lockdown_active { 1.0 } else { 0.0 };
        for (idx, entry) in self.windows.iter_mut().enumerate() {
            entry.win.hovered = self.interaction.hovered_window == Some(idx);
            entry.win.focused = self.interaction.focused_window == Some(idx);
            let [tr, tg, tb] = entry.win.trust_domain.border_color_rgb();
            let uniform = WindowStateUniform {
                state: [
                    entry.win.id as f32,
                    if entry.win.hovered { 1.0 } else { 0.0 },
                    if entry.win.focused { 1.0 } else { 0.0 },
                    0.94,
                ],
                trust_color: [tr, tg, tb, lockdown_flag],
            };
            self.queue
                .write_buffer(&entry.state_buffer, 0, bytemuck::cast_slice(&[uniform]));
        }
    }

    fn update(&mut self) {
        // Non-blocking. Lets wgpu run pending callbacks, including the device-lost
        // callback, which some loss paths only deliver from a poll.
        let _ = self.device.poll(wgpu::PollType::Poll);

        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;

        // Continuous spatial movement (W/A/S/D + Q/E to fly through the Matrix code corridors)
        let speed = if self.interaction.shift_held {
            18.0
        } else {
            9.5
        } * dt;
        let mut fwd = 0.0;
        let mut right = 0.0;
        let mut up = 0.0;

        if self.interaction.keys_pressed.contains(&KeyCode::KeyW)
            || self.interaction.keys_pressed.contains(&KeyCode::ArrowUp)
        {
            fwd += speed;
        }
        if self.interaction.keys_pressed.contains(&KeyCode::KeyS)
            || self.interaction.keys_pressed.contains(&KeyCode::ArrowDown)
        {
            fwd -= speed;
        }
        if self.interaction.keys_pressed.contains(&KeyCode::KeyD)
            || self.interaction.keys_pressed.contains(&KeyCode::ArrowRight)
        {
            right += speed;
        }
        if self.interaction.keys_pressed.contains(&KeyCode::KeyA)
            || self.interaction.keys_pressed.contains(&KeyCode::ArrowLeft)
        {
            right -= speed;
        }
        if self.interaction.keys_pressed.contains(&KeyCode::KeyE) {
            up += speed;
        }
        if self.interaction.keys_pressed.contains(&KeyCode::KeyQ) {
            up -= speed;
        }

        if self.overview.is_active() {
            // The overview camera orbits the universe; it looks along the orbit direction.
            self.camera.target_yaw = self.overview.yaw;
            self.camera.target_pitch = self.overview.pitch;
            if !self.camera.is_flying() {
                self.camera.target_position = self.overview.eye();
            }
        } else if fwd != 0.0 || right != 0.0 || up != 0.0 {
            self.camera.translate_local(fwd, right, up);
        }

        self.camera.update(dt);
        if !self.camera.is_flying() && !self.overview.is_active() {
            if let Some(destination) = self.travel_destination.take() {
                let cargo = self.travel_cargo.take();
                let chosen = self.travel_focus.take();
                let center = self.galaxies.get(destination).map(|g| g.center);
                // A flight interrupted by manual movement does not count as an arrival,
                // and a carried window stays where it was.
                if center.is_some_and(|c| c.distance(self.camera.position) < 0.5) {
                    self.arrive_at_galaxy(destination, cargo, chosen);
                }
            }
            // Flying by hand into another galaxy's halo makes it the current galaxy.
            if let Some(galaxy) = galaxy_at(&self.galaxies, self.camera.position) {
                if galaxy != self.current_galaxy {
                    self.set_current_galaxy(galaxy);
                    self.window.set_title(&self.idle_title());
                }
            }
        }
        self.minimap.update(dt);
        self.update_warp_aim();
        self.update_galaxy_visibility();
        self.update_raycast_hover();
        if self.overview.is_active() {
            self.update_constellations();
        }

        let elapsed = (now - self.start_time).as_secs_f32();
        let cam_fwd = self.camera.forward();
        // Frontal clear field-of-view reach: guarantees a 100% clear corridor past the focused window
        let active_clear_dist = self
            .interaction
            .hovered_window
            .or(self.interaction.focused_window)
            .and_then(|idx| self.windows.get(idx))
            .map(|entry| (entry.win.world_center() - self.camera.position).length() + 1.5)
            .unwrap_or(11.8)
            .max(11.5);

        // The shader shields up to four windows from the code rain: use the closest ones.
        let mut nearest: Vec<(f32, glam::Vec3, f32)> = self
            .windows
            .iter()
            .map(|entry| {
                let center = entry.win.world_center();
                (
                    center.distance_squared(self.camera.position),
                    center,
                    entry.win.shield_cos(),
                )
            })
            .collect();
        nearest.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut window_shields = [[0.0_f32, 0.0, -10.0, 1.0]; 4];
        for (slot, (_, center, shield_cos)) in window_shields.iter_mut().zip(nearest) {
            *slot = [center.x, center.y, center.z, shield_cos];
        }

        let globals_uniform = GlobalsUniform {
            view_proj: self.camera.view_proj_matrix().to_cols_array_2d(),
            camera_pos: [
                self.camera.position.x,
                self.camera.position.y,
                self.camera.position.z,
                1.0,
            ],
            params: [
                elapsed,
                self.interaction
                    .focused_window
                    .map(|i| i as f32)
                    .unwrap_or(-1.0),
                self.interaction
                    .hovered_window
                    .map(|i| i as f32)
                    .unwrap_or(-1.0),
                self.matrix_speed_mult,
            ],
            camera_forward: [cam_fwd.x, cam_fwd.y, cam_fwd.z, active_clear_dist],
            window_shields,
        };
        self.queue.write_buffer(
            &self.globals_buffer,
            0,
            bytemuck::cast_slice(&[globals_uniform]),
        );

        // Update Mini-Globe HUD uniform (windows are rendered at their true radial depths!)
        let minimap_eye = self.minimap.eye_position();
        let minimap_globals = GlobalsUniform {
            view_proj: self.minimap.view_proj_matrix().to_cols_array_2d(),
            camera_pos: [minimap_eye.x, minimap_eye.y, minimap_eye.z, 1.0],
            params: [
                elapsed,
                0.0,
                if self.minimap.is_hovered { 1.0 } else { 0.0 },
                self.minimap.expansion_anim,
            ],
            camera_forward: [cam_fwd.x, cam_fwd.y, cam_fwd.z, active_clear_dist],
            window_shields,
        };
        self.queue.write_buffer(
            &self.minimap_globals_buffer,
            0,
            bytemuck::cast_slice(&[minimap_globals]),
        );
    }

    /// Replaces a lost surface with a new one for the same window and configures it.
    ///
    /// The old surface is dropped first, because a window can back only one surface
    /// at a time. On failure `self.surface` stays `None` and `render` retries on the
    /// next frame.
    fn recreate_surface(&mut self) {
        drop(self.surface.take());
        match self.instance.create_surface(self.window.clone()) {
            Ok(surface) => {
                surface.configure(&self.device, &self.config);
                self.surface = Some(surface);
            }
            Err(e) => log::debug!("Surface recreation failed, will retry: {e}"),
        }
    }

    /// Renders one frame. Returns `true` when the surface must be reconfigured
    /// before the next frame.
    fn render(&mut self) -> bool {
        let Some(surface) = &self.surface else {
            // The surface was lost and could not be recreated yet: try again this frame.
            self.recreate_surface();
            return false;
        };
        let (output, needs_reconfigure) = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => (texture, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => (texture, true),
            wgpu::CurrentSurfaceTexture::Outdated => return true,
            wgpu::CurrentSurfaceTexture::Lost => {
                log::warn!("Surface lost; recreating it");
                self.recreate_surface();
                return false;
            }
            // Nothing to draw to right now: skip this frame and try again on the next one.
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return false;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                log::warn!("Surface texture acquisition failed validation; frame skipped");
                return false;
            }
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Spheradian Frame Encoder"),
            });

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Spheradian Main Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Deep abyssal Matrix-green background
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.004,
                            g: 0.016,
                            b: 0.008,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // ================================================================
            // MAIN 3D SCENE (Full Screen)
            // ================================================================
            // 1. Draw the SphereXP wireframe grid
            rpass.set_pipeline(&self.grid_pipeline);
            rpass.set_bind_group(0, &self.globals_bind_group, &[]);
            rpass.set_vertex_buffer(0, self.grid_vertex_buffer.slice(..));
            // Inside a galaxy only its own halo is drawn; the rest of the universe
            // appears while aiming a warp, while flying, or in deep space.
            if self.universe_revealed {
                rpass.draw(self.universe_grid_range.clone(), 0..1);
            }
            for (id, visibility) in self.galaxy_visibility.iter().enumerate() {
                let range = match visibility.halo {
                    HaloStyle::Hidden => None,
                    HaloStyle::Beacon => self.galaxy_beacon_ranges.get(id),
                    HaloStyle::Full => self.galaxy_grid_ranges.get(id),
                };
                if let Some(range) = range {
                    rpass.draw(range.clone(), 0..1);
                }
            }
            // Trust constellations: windows of one trust domain linked across the universe.
            if self.overview.is_active() && self.constellation_vertex_count > 0 {
                rpass.set_vertex_buffer(0, self.constellation_buffer.slice(..));
                rpass.draw(0..self.constellation_vertex_count, 0..1);
            }

            // 2. Draw curved 3D windows on the sphere
            rpass.set_pipeline(&self.window_pipeline);
            rpass.set_bind_group(0, &self.globals_bind_group, &[]);
            for entry in &self.windows {
                if !self.shows_galaxy_windows(entry.win.galaxy) {
                    continue;
                }
                rpass.set_bind_group(1, &entry.win.bind_group, &[]);
                rpass.set_vertex_buffer(0, entry.win.vertex_buffer.slice(..));
                rpass.set_index_buffer(entry.win.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                rpass.draw_indexed(0..entry.win.index_count, 0, 0..1);
            }

            // 3. Draw the 3D volumetric Matrix code streams
            rpass.set_pipeline(&self.matrix_pipeline);
            rpass.set_bind_group(0, &self.globals_bind_group, &[]);
            rpass.set_vertex_buffer(0, self.matrix_vertex_buffer.slice(..));
            rpass.set_index_buffer(
                self.matrix_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            for (id, range) in self.galaxy_matrix_ranges.iter().enumerate() {
                if self.galaxy_visibility.get(id).is_some_and(|v| v.rain) {
                    rpass.draw_indexed(range.clone(), 0, 0..1);
                }
            }

            // ================================================================
            // 4. INTERACTIVE 3D MINI-GLOBE HUD (Bottom-Right / Overview)
            // ================================================================
            let (hx, hy, hsize) = self
                .minimap
                .viewport_rect(self.config.width, self.config.height);
            // The Mini-Globe shows one galaxy, so it steps aside in the overview.
            if !self.overview.is_active()
                && hx + hsize <= self.config.width as f32
                && hy + hsize <= self.config.height as f32
            {
                rpass.set_viewport(hx, hy, hsize, hsize, 0.0, 1.0);

                // 4a. Background holographic orb (also resets the Mini-Globe depth buffer to 0.999)
                rpass.set_pipeline(&self.minimap_bg_pipeline);
                rpass.set_bind_group(0, &self.minimap_globals_bind_group, &[]);
                rpass.draw(0..6, 0..1);

                // 4b. 3D spherical grid viewed from the outside inside the Mini-Globe
                rpass.set_pipeline(&self.grid_pipeline);
                rpass.set_bind_group(0, &self.minimap_globals_bind_group, &[]);
                rpass.set_vertex_buffer(0, self.grid_vertex_buffer.slice(..));
                if let Some(range) = self.galaxy_grid_ranges.get(self.current_galaxy) {
                    rpass.draw(range.clone(), 0..1);
                }

                // 4c. 3D windows rendered at their true radial depths inside the Mini-Globe!
                rpass.set_pipeline(&self.window_pipeline);
                rpass.set_bind_group(0, &self.minimap_globals_bind_group, &[]);
                for entry in self
                    .windows
                    .iter()
                    .filter(|entry| entry.win.galaxy == self.current_galaxy)
                {
                    rpass.set_bind_group(1, &entry.win.bind_group, &[]);
                    rpass.set_vertex_buffer(0, entry.win.vertex_buffer.slice(..));
                    rpass.set_index_buffer(
                        entry.win.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    rpass.draw_indexed(0..entry.win.index_count, 0, 0..1);
                }
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        self.queue.present(output);
        needs_reconfigure
    }
}

#[derive(Default)]
struct SpheradianApp {
    renderer: Option<SpheradianRenderer>,
    /// When the last rebuild after a device loss was attempted, to space out retries.
    last_recovery_attempt: Option<Instant>,
}

impl SpheradianApp {
    const RECOVERY_RETRY_INTERVAL: Duration = Duration::from_secs(1);

    /// If the GPU device was lost, rebuilds the renderer on a new device and carries
    /// the session over. Returns `true` when the renderer is usable for this frame.
    ///
    /// When the rebuild fails (for example while the driver is still resetting), the
    /// lost renderer is kept only for its session state, drawing is suspended, and the
    /// rebuild is retried once per `RECOVERY_RETRY_INTERVAL`.
    fn recover_from_device_loss(&mut self) -> bool {
        let Some(renderer) = self.renderer.as_mut() else {
            return false;
        };
        if !renderer.device_lost.load(Ordering::Acquire) {
            return true;
        }

        let now = Instant::now();
        if !should_attempt_recovery(
            self.last_recovery_attempt,
            now,
            Self::RECOVERY_RETRY_INTERVAL,
        ) {
            return false;
        }
        self.last_recovery_attempt = Some(now);

        // A window backs only one surface at a time: release the old one first.
        drop(renderer.surface.take());

        match pollster::block_on(SpheradianRenderer::new(renderer.window.clone())) {
            Ok(mut rebuilt) => {
                if let Some(previous) = self.renderer.take() {
                    rebuilt.adopt_session_state(previous);
                }
                self.renderer = Some(rebuilt);
                self.last_recovery_attempt = None;
                log::info!("GPU device recovered; renderer rebuilt");
                true
            }
            Err(e) => {
                log::error!("Renderer rebuild after device loss failed, will retry: {e}");
                // Input events are ignored while suspended, so releases would be missed:
                // drop held keys and buttons instead of carrying them over as stuck.
                renderer.interaction.release_held();
                false
            }
        }
    }
}

impl ApplicationHandler for SpheradianApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_none() {
            let win_attrs = Window::default_attributes()
                .with_title(concat!(
                    "Spheradian v",
                    env!("CARGO_PKG_VERSION"),
                    " [Rust + wgpu] — Mini-Globe in bottom-right (Rotate / Click) | Scroll/WASD: Fly through Matrix Code"
                ))
                .with_inner_size(PhysicalSize::new(1366, 840));

            let window = Arc::new(
                event_loop
                    .create_window(win_attrs)
                    .expect("Failed to create Spheradian window"),
            );

            let renderer = pollster::block_on(SpheradianRenderer::new(window))
                .unwrap_or_else(|e| panic!("{e}"));
            self.renderer = Some(renderer);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        // Every GPU resource is invalid once the device is lost, so nothing may touch
        // the renderer until it has been rebuilt. Recovery runs on the next redraw.
        let device_lost = self
            .renderer
            .as_ref()
            .is_some_and(|renderer| renderer.device_lost.load(Ordering::Acquire));
        if device_lost {
            match event {
                WindowEvent::CloseRequested => {
                    event_loop.exit();
                    return;
                }
                WindowEvent::RedrawRequested => {
                    if !self.recover_from_device_loss() {
                        // No usable device yet: keep the redraw loop alive to retry.
                        if let Some(renderer) = &self.renderer {
                            renderer.window.request_redraw();
                        }
                        return;
                    }
                }
                _ => return,
            }
        }

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => renderer.resize(size),
            // Button releases are not delivered while unfocused: a chord that loses focus
            // is cancelled so it can never fire later by surprise.
            WindowEvent::Focused(false) if renderer.interaction.warp.is_active() => {
                renderer.cancel_warp_chord();
            }
            WindowEvent::ModifiersChanged(mods) => {
                renderer.interaction.shift_held = mods.state().shift_key();
                renderer.interaction.ctrl_held = mods.state().control_key();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    match event.state {
                        ElementState::Pressed => {
                            renderer.interaction.keys_pressed.insert(code);
                            match code {
                                KeyCode::Escape => {
                                    if renderer.overview.is_active() {
                                        let home = renderer.current_galaxy;
                                        renderer.travel_to_galaxy(home);
                                    } else if renderer.interaction.warp.is_active() {
                                        renderer.cancel_warp_chord();
                                    } else if renderer.minimap.expanded {
                                        renderer.minimap.expanded = false;
                                    } else {
                                        event_loop.exit();
                                    }
                                }
                                // Qubes OS Secure Inter-Domain Clipboard: Ctrl + Shift + C (Copy to Vault)
                                KeyCode::KeyC
                                    if renderer.interaction.ctrl_held
                                        && renderer.interaction.shift_held =>
                                {
                                    if renderer.lockdown_active {
                                        log::warn!(
                                            "[SECURITY] Clipboard copy blocked: Lockdown active"
                                        );
                                    } else if let Some(idx) = renderer.interaction.focused_window {
                                        let win = &renderer.windows[idx].win;
                                        renderer.clipboard_vault.seal_from_window(
                                            win.trust_domain,
                                            &win.title,
                                            format!("sealed-buffer-from-window-{}", win.id),
                                        );
                                        let msg = format!(
                                            "Spheradian [CLIPBOARD VAULT] — Copied & Sealed from [{}] ({}) | Focus target & press Ctrl+Shift+V",
                                            win.trust_domain.badge_label(),
                                            win.title
                                        );
                                        log::info!("{}", msg);
                                        renderer.window.set_title(&msg);
                                    }
                                }
                                // Qubes OS Secure Inter-Domain Clipboard: Ctrl + Shift + V (Paste & Auto-Wipe)
                                KeyCode::KeyV
                                    if renderer.interaction.ctrl_held
                                        && renderer.interaction.shift_held =>
                                {
                                    if renderer.lockdown_active {
                                        log::warn!(
                                            "[SECURITY] Clipboard paste blocked: Lockdown active"
                                        );
                                    } else if let Some(idx) = renderer.interaction.focused_window {
                                        let dest_win = &renderer.windows[idx].win;
                                        match renderer
                                            .clipboard_vault
                                            .unseal_to_window(dest_win.trust_domain)
                                        {
                                            Ok((src_dom, src_title, _payload)) => {
                                                let msg = format!(
                                                    "Spheradian [CLIPBOARD VAULT] — Transfer APPROVED: [{}] ({}) -> [{}] ({}) (Vault auto-wiped!)",
                                                    src_dom.badge_label(),
                                                    src_title,
                                                    dest_win.trust_domain.badge_label(),
                                                    dest_win.title
                                                );
                                                log::info!("{}", msg);
                                                renderer.window.set_title(&msg);
                                            }
                                            Err(violation) => {
                                                let msg = format!(
                                                    "Spheradian [SECURITY ALERT] — {}",
                                                    violation
                                                );
                                                log::warn!("{}", msg);
                                                renderer.window.set_title(&msg);
                                            }
                                        }
                                    }
                                }
                                // Instant Sphere Lockdown Mode ('L'): Blanks all window textures & wipes clipboard
                                KeyCode::KeyL => {
                                    renderer.lockdown_active = !renderer.lockdown_active;
                                    if renderer.lockdown_active {
                                        renderer.clipboard_vault.wipe();
                                        renderer.interaction.dragging_window = None;
                                        let msg = "Spheradian [LOCKDOWN ACTIVE] — All 3D window surfaces encrypted/blanked & Clipboard wiped (Press 'L' to unlock)";
                                        log::warn!("{}", msg);
                                        renderer.window.set_title(msg);
                                    } else {
                                        let msg = "Spheradian [LOCKDOWN LIFTED] — Trusted 3D workspace restored";
                                        log::info!("{}", msg);
                                        renderer.window.set_title(msg);
                                    }
                                }
                                // Runtime Security Audit Report ('F12')
                                KeyCode::F12 => {
                                    let report = SecurityAuditReport::run_audit(
                                        &renderer.security_policy,
                                        include_str!("shader.wgsl"),
                                    );
                                    let summary = report.summary_line();
                                    log::info!("[SECURITY AUDIT] {}", summary);
                                    renderer.window.set_title(&format!(
                                        "Spheradian [SECURITY AUDIT] — {}",
                                        summary
                                    ));
                                }
                                KeyCode::KeyG => {
                                    renderer.minimap.toggle_expanded();
                                }
                                // Cosmic Zoom-Out: see the whole universe, or dive back in
                                KeyCode::KeyU => {
                                    if renderer.overview.is_active() {
                                        let home = renderer.current_galaxy;
                                        renderer.travel_to_galaxy(home);
                                    } else {
                                        renderer.open_overview();
                                    }
                                }
                                // Intergalactic travel: Ctrl + PageUp / PageDown cycles galaxies
                                KeyCode::PageUp | KeyCode::PageDown
                                    if renderer.interaction.ctrl_held =>
                                {
                                    let step = if code == KeyCode::PageDown { 1 } else { -1 };
                                    let destination = cycle_galaxy(
                                        renderer.current_galaxy,
                                        renderer.galaxies.len(),
                                        step,
                                    );
                                    renderer.travel_to_galaxy(destination);
                                }
                                // Intergalactic travel: Ctrl + 1..9 jumps to a galaxy directly
                                _ if renderer.interaction.ctrl_held
                                    && galaxy_hotkey_index(code).is_some() =>
                                {
                                    if let Some(destination) = galaxy_hotkey_index(code) {
                                        renderer.travel_to_galaxy(destination);
                                    }
                                }
                                KeyCode::Space | KeyCode::Tab => {
                                    renderer.interaction.free_look_mode =
                                        !renderer.interaction.free_look_mode;
                                }
                                KeyCode::KeyR | KeyCode::Home => {
                                    let home = renderer.current_galaxy_center();
                                    renderer.camera.reset_to(home);
                                    renderer.minimap.target_orb_yaw = 0.0;
                                    renderer.minimap.target_orb_pitch = 0.32;
                                }
                                KeyCode::KeyM => {
                                    renderer.matrix_speed_mult = if renderer.matrix_speed_mult > 0.2
                                    {
                                        0.15
                                    } else {
                                        1.0
                                    };
                                }
                                KeyCode::F11 | KeyCode::KeyF => {
                                    let is_fs = renderer.window.fullscreen().is_some();
                                    renderer.window.set_fullscreen(if is_fs {
                                        None
                                    } else {
                                        Some(Fullscreen::Borderless(None))
                                    });
                                }
                                _ => {}
                            }
                        }
                        ElementState::Released => {
                            renderer.interaction.keys_pressed.remove(&code);
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let (px, py) = renderer.interaction.cursor_pos;
                let on_minimap = renderer.minimap.contains_cursor(
                    px,
                    py,
                    renderer.config.width,
                    renderer.config.height,
                );

                let pressed = state == ElementState::Pressed;
                let on_minimap = on_minimap && !renderer.overview.is_active();

                match button {
                    _ if renderer.overview.is_active() => renderer.overview_button(button, pressed),
                    MouseButton::Middle => {
                        if pressed && on_minimap {
                            renderer.minimap.toggle_expanded();
                        } else {
                            renderer.interaction.middle_mouse_down = pressed;
                        }
                    }
                    MouseButton::Right => {
                        renderer.interaction.right_mouse_down = pressed;
                        if pressed && renderer.try_begin_warp_chord(on_minimap) {
                            // Left was already held: the space warp chord has started.
                        } else if pressed && on_minimap {
                            renderer.interaction.minimap_dragging = true;
                            renderer.interaction.minimap_drag_distance = 0.0;
                        } else if !pressed && renderer.interaction.warp.is_active() {
                            renderer.release_warp_chord();
                        } else if !pressed && renderer.interaction.minimap_dragging {
                            renderer.interaction.minimap_dragging = false;
                            // Quick right-click on the Mini-Globe expands/collapses the Globe on screen
                            if renderer.interaction.minimap_drag_distance < 6.0 && on_minimap {
                                renderer.minimap.toggle_expanded();
                            }
                        }
                    }
                    MouseButton::Left => {
                        renderer.interaction.left_mouse_down = pressed;
                        if pressed {
                            if renderer.try_begin_warp_chord(on_minimap) {
                                // Right was already held: the space warp chord has started.
                            } else if on_minimap {
                                renderer.interaction.minimap_dragging = true;
                                renderer.interaction.minimap_drag_distance = 0.0;
                                renderer.interaction.dragging_window = None;
                            } else if let Some(hovered) = renderer.interaction.hovered_window {
                                renderer.interaction.focused_window = Some(hovered);
                                renderer.interaction.dragging_window = Some(hovered);
                            } else {
                                renderer.interaction.dragging_window = None;
                            }
                        } else {
                            if renderer.interaction.warp.is_active() {
                                renderer.release_warp_chord();
                            } else if renderer.interaction.minimap_dragging {
                                // Quick click on the Mini-Globe rotates the view there
                                renderer.interaction.minimap_dragging = false;
                                if renderer.interaction.minimap_drag_distance < 6.0 {
                                    renderer.jump_to_minimap_point(px, py);
                                }
                            }
                            renderer.interaction.dragging_window = None;
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let new_pos = (position.x as f32, position.y as f32);
                if let Some(last) = renderer.interaction.last_cursor_pos {
                    let dx = new_pos.0 - last.0;
                    let dy = new_pos.1 - last.1;

                    renderer.interaction.warp.add_drag(dx, dy);

                    if renderer.overview.is_active() {
                        renderer.overview_pointer_moved(dx, dy);
                    } else if renderer.interaction.minimap_dragging {
                        let step_dist = (dx * dx + dy * dy).sqrt();
                        renderer.interaction.minimap_drag_distance += step_dist;
                        // Dragging inside the Mini-Globe rotates the 3D globe
                        renderer.minimap.rotate_globe(dx, dy);
                    } else if let Some(win_idx) = renderer.interaction.dragging_window {
                        // Drag the 3D window along the curved surface of the sphere (azimuth, elevation)
                        let entry = &mut renderer.windows[win_idx];
                        entry.win.azimuth += dx * 0.0032;
                        entry.win.elevation =
                            (entry.win.elevation - dy * 0.0032).clamp(-1.15, 1.15);
                        entry.win.update_geometry(&renderer.queue);
                    } else if renderer.interaction.right_mouse_down
                        || renderer.interaction.middle_mouse_down
                        || renderer.interaction.free_look_mode
                        || renderer.interaction.left_mouse_down
                    {
                        renderer.camera.rotate(dx, dy, 0.0038);
                    }
                }
                renderer.interaction.cursor_pos = new_pos;
                renderer.interaction.last_cursor_pos = Some(new_pos);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let scroll_y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(pos) => (pos.y as f32) * 0.04,
                };

                let (px, py) = renderer.interaction.cursor_pos;
                let on_minimap = renderer.minimap.contains_cursor(
                    px,
                    py,
                    renderer.config.width,
                    renderer.config.height,
                );

                // If the cursor is over a window (in the main scene OR inside the Mini-Globe)
                // or if Shift is held, scrolling adjusts the radial depth of the window inside the globe
                if renderer.overview.is_active() {
                    renderer.overview.zoom_by(scroll_y);
                } else if let Some(win_idx) =
                    renderer
                        .interaction
                        .hovered_window
                        .or(if renderer.interaction.shift_held {
                            renderer.interaction.focused_window
                        } else {
                            None
                        })
                {
                    let entry = &mut renderer.windows[win_idx];
                    entry.win.radius = (entry.win.radius - scroll_y * 0.95).clamp(4.0, 24.5);
                    entry.win.update_geometry(&renderer.queue);
                } else if on_minimap {
                    // Scrolling over empty area of the Mini-Globe rotates the globe horizontally
                    renderer.minimap.rotate_globe(scroll_y * 28.0, 0.0);
                } else {
                    // Scrolling over empty space in the sphere flies forward/backward
                    renderer.camera.translate_local(scroll_y * 1.65, 0.0, 0.0);
                }
            }
            WindowEvent::RedrawRequested => {
                renderer.update();
                if renderer.render() {
                    let size = renderer.window.inner_size();
                    renderer.resize(size);
                }
                renderer.window.request_redraw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(renderer) = &self.renderer {
            renderer.window.request_redraw();
        }
    }
}

/// Maps `1`..`9` (top row or keypad) to galaxy ids `0`..`8`.
fn galaxy_hotkey_index(code: KeyCode) -> Option<GalaxyId> {
    let index = match code {
        KeyCode::Digit1 | KeyCode::Numpad1 => 0,
        KeyCode::Digit2 | KeyCode::Numpad2 => 1,
        KeyCode::Digit3 | KeyCode::Numpad3 => 2,
        KeyCode::Digit4 | KeyCode::Numpad4 => 3,
        KeyCode::Digit5 | KeyCode::Numpad5 => 4,
        KeyCode::Digit6 | KeyCode::Numpad6 => 5,
        KeyCode::Digit7 | KeyCode::Numpad7 => 6,
        KeyCode::Digit8 | KeyCode::Numpad8 => 7,
        KeyCode::Digit9 | KeyCode::Numpad9 => 8,
        _ => return None,
    };
    Some(index)
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    log::info!(
        "Starting Spheradian v{} — 3D Spherical Desktop Environment in Rust...",
        env!("CARGO_PKG_VERSION")
    );

    let event_loop = EventLoop::new().expect("Failed to create EventLoop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = SpheradianApp::default();
    event_loop
        .run_app(&mut app)
        .expect("Error while running Spheradian");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_galaxy_hotkeys_map_digits_to_galaxy_ids() {
        assert_eq!(galaxy_hotkey_index(KeyCode::Digit1), Some(0));
        assert_eq!(galaxy_hotkey_index(KeyCode::Numpad4), Some(3));
        assert_eq!(galaxy_hotkey_index(KeyCode::Digit9), Some(8));
        assert_eq!(galaxy_hotkey_index(KeyCode::Digit0), None);
        assert_eq!(galaxy_hotkey_index(KeyCode::KeyG), None);
    }

    #[test]
    fn test_should_attempt_recovery_respects_retry_interval() {
        let now = Instant::now();
        let interval = Duration::from_secs(1);

        assert!(should_attempt_recovery(None, now, interval));

        let too_soon = now;
        assert!(!should_attempt_recovery(
            Some(too_soon),
            now + Duration::from_millis(999),
            interval
        ));

        assert!(should_attempt_recovery(
            Some(too_soon),
            now + Duration::from_secs(1),
            interval
        ));
    }

    #[test]
    fn test_release_held_inputs_clears_held_state_and_preserves_session_toggles() {
        let mut state = InteractionState {
            cursor_pos: (320.0, 240.0),
            last_cursor_pos: Some((310.0, 235.0)),
            right_mouse_down: true,
            left_mouse_down: true,
            middle_mouse_down: true,
            free_look_mode: true,
            shift_held: true,
            ctrl_held: true,
            keys_pressed: HashSet::from([KeyCode::KeyW, KeyCode::KeyA]),
            hovered_window: Some(1),
            focused_window: Some(0),
            dragging_window: Some(1),
            minimap_dragging: true,
            minimap_drag_distance: 42.0,
            warp: WarpGesture::default(),
            overview_press: Some(OverviewPress {
                window: Some(1),
                travel_px: 3.0,
            }),
        };
        state.warp.begin(Some(1));
        state.warp.add_drag(100.0, 0.0);

        state.release_held();

        assert!(state.keys_pressed.is_empty());
        assert!(!state.left_mouse_down);
        assert!(!state.right_mouse_down);
        assert!(!state.shift_held);
        assert!(!state.ctrl_held);
        assert_eq!(state.dragging_window, None);
        assert!(!state.minimap_dragging);
        assert!(!state.middle_mouse_down);
        // A chord interrupted by a lost device must never fire later.
        assert!(!state.warp.is_active());
        assert_eq!(state.overview_press, None);
        assert_eq!(state.warp.release(), None);

        // Session toggles and non-held pointer/focus state should remain intact.
        assert!(state.free_look_mode);
        assert_eq!(state.cursor_pos, (320.0, 240.0));
        assert_eq!(state.hovered_window, Some(1));
        assert_eq!(state.focused_window, Some(0));
    }
}
