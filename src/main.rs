mod camera;
mod minimap;
mod sphere_env;
mod window3d;

use bytemuck::{Pod, Zeroable};
use camera::SphereCamera;
use minimap::MiniGlobeHud;
use sphere_env::{generate_matrix_code_volume, generate_sphere_grid, GridVertex, MatrixVertex};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;
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
}

struct WindowRenderEntry {
    win: SphericalWindow,
    state_buffer: wgpu::Buffer,
}

struct XSphereRenderer {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth_view: wgpu::TextureView,

    camera: SphereCamera,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,

    // Mini-Mapa do Globo 3D (HUD no canto inferior direito)
    minimap: MiniGlobeHud,
    minimap_globals_buffer: wgpu::Buffer,
    minimap_globals_bind_group: wgpu::BindGroup,
    minimap_bg_pipeline: wgpu::RenderPipeline,

    grid_pipeline: wgpu::RenderPipeline,
    grid_vertex_buffer: wgpu::Buffer,
    grid_vertex_count: u32,

    matrix_pipeline: wgpu::RenderPipeline,
    matrix_vertex_buffer: wgpu::Buffer,
    matrix_index_buffer: wgpu::Buffer,
    matrix_index_count: u32,

    window_pipeline: wgpu::RenderPipeline,
    windows: Vec<WindowRenderEntry>,

    start_time: Instant,
    last_frame: Instant,
    matrix_speed_mult: f32,

    // Estado de interação espacial 3D
    cursor_pos: (f32, f32),
    last_cursor_pos: Option<(f32, f32)>,
    right_mouse_down: bool,
    left_mouse_down: bool,
    free_look_mode: bool,
    shift_held: bool,
    keys_pressed: HashSet<KeyCode>,
    hovered_window: Option<usize>,
    focused_window: Option<usize>,
    dragging_window: Option<usize>,

    // Interação com o Mini-Globo
    minimap_dragging: bool,
    minimap_drag_distance: f32,
}

impl XSphereRenderer {
    const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
    const SPHERE_RADIUS: f32 = 26.0;

    fn create_depth_view(
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> wgpu::TextureView {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("XSphereDE Depth Buffer"),
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

    async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let surface = instance
            .create_surface(window.clone())
            .expect("Falha ao criar superfície wgpu");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("Nenhum adaptador de GPU compatível encontrado");

        log::info!("GPU selecionada para XSphereDE: {:?}", adapter.get_info().name);

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("XSphereDE Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await
            .expect("Falha ao inicializar dispositivo gráfico");

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let present_mode = if surface_caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
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
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let depth_view = Self::create_depth_view(&device, width, height);
        let camera = SphereCamera::new(width, height);
        let minimap = MiniGlobeHud::new(Self::SPHERE_RADIUS);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("XSphereDE Unified Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        // Uniform global (câmera principal + tempo)
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

        // BindGroupLayout para janelas 3D curvas
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
                bind_group_layouts: &[&globals_bgl],
                push_constant_ranges: &[],
            });

        let window_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Window Pipeline Layout"),
                bind_group_layouts: &[&globals_bgl, &window_bgl],
                push_constant_ranges: &[],
            });

        // 1. Pipeline da Grade Esférica (SphereXP Wireframe)
        let grid_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("SphereXP Grid Pipeline"),
            layout: Some(&basic_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_grid"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GridVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
                }],
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
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // 2. Pipeline Volumétrico 3D do Código Matrix (Additive + Alpha Blend)
        let matrix_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Matrix Volumetric Code Pipeline"),
            layout: Some(&basic_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_matrix"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MatrixVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32x2,
                        2 => Float32x4
                    ],
                }],
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
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // 3. Pipeline das Janelas 3D Curvas na Esfera
        let window_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Spherical 3D Window Pipeline"),
            layout: Some(&window_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_window"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<WindowVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32x2,
                        2 => Float32x3
                    ],
                }],
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
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // 4. Pipeline do Fundo Holográfico do Mini-Globo (desenha o orbe e limpa o depth buffer do HUD)
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
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Geometria da Esfera e do Volume de Código Matrix
        let grid_vertices = generate_sphere_grid(Self::SPHERE_RADIUS, 24, 14);
        let grid_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Sphere Grid Vertex Buffer"),
            contents: bytemuck::cast_slice(&grid_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let (matrix_vertices, matrix_indices) = generate_matrix_code_volume(Self::SPHERE_RADIUS);
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

        // Cria 4 janelas 3D curvas distribuídas ao redor da esfera
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Window Texture Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let demo_specs = [
            (
                "Terminal Matrix // XSphereShell",
                -0.45_f32,
                0.12_f32,
                9.5_f32,
                0.56_f32,
                0.36_f32,
                0usize,
            ),
            (
                "Monitor de Telemetria 3D",
                0.48_f32,
                0.15_f32,
                10.5_f32,
                0.54_f32,
                0.35_f32,
                1usize,
            ),
            (
                "Navegador de Arquivos Espacial",
                -0.02_f32,
                -0.34_f32,
                8.8_f32,
                0.58_f32,
                0.36_f32,
                2usize,
            ),
            (
                "Radar da Esfera Desktop (Atrás)",
                2.55_f32,
                0.08_f32,
                10.2_f32,
                0.54_f32,
                0.38_f32,
                3usize,
            ),
        ];

        let mut windows = Vec::new();
        for (id, (title, azimuth, elevation, radius, ang_w, ang_h, variant)) in
            demo_specs.into_iter().enumerate()
        {
            let tex_w = 512;
            let tex_h = 340;
            let rgba = generate_demo_window_rgba(tex_w, tex_h, variant);

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

            let state_uniform = WindowStateUniform {
                state: [id as f32, 0.0, if id == 0 { 1.0 } else { 0.0 }, 0.94],
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

            let (verts, idxs) =
                SphericalWindow::build_curved_mesh(azimuth, elevation, radius, ang_w, ang_h);

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
        Self {
            window,
            surface,
            device,
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
            grid_vertex_count: grid_vertices.len() as u32,
            matrix_pipeline,
            matrix_vertex_buffer,
            matrix_index_buffer,
            matrix_index_count: matrix_indices.len() as u32,
            window_pipeline,
            windows,
            start_time: now,
            last_frame: now,
            matrix_speed_mult: 1.0,
            cursor_pos: (width as f32 * 0.5, height as f32 * 0.5),
            last_cursor_pos: None,
            right_mouse_down: false,
            left_mouse_down: false,
            free_look_mode: false,
            shift_held: bool::default(),
            keys_pressed: HashSet::new(),
            hovered_window: None,
            focused_window: Some(0),
            dragging_window: None,
            minimap_dragging: false,
            minimap_drag_distance: 0.0,
        }
    }

    fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
            self.depth_view =
                Self::create_depth_view(&self.device, new_size.width, new_size.height);
            self.camera.resize(new_size.width, new_size.height);
        }
    }

    /// Faz raycasting 3D nas janelas (em suas profundidades reais `radius`) usando o raio do Mini-Globo.
    fn raycast_window_in_minimap(&self, px: f32, py: f32) -> Option<usize> {
        let (ray_origin, ray_dir) =
            self.minimap
                .screen_ray(px, py, self.config.width, self.config.height)?;
        let mut best_hit: Option<(usize, f32)> = None;
        for (idx, entry) in self.windows.iter().enumerate() {
            if let Some(t) = entry.win.intersect_ray(ray_origin, ray_dir) {
                if best_hit.is_none_or(|(_, best_t)| t < best_t) {
                    best_hit = Some((idx, t));
                }
            }
        }
        best_hit.map(|(idx, _)| idx)
    }

    /// Quando o usuário clica no Mini-Globo, direciona o ambiente 3D exatamente para lá!
    fn jump_to_minimap_point(&mut self, px: f32, py: f32) {
        if let Some(win_idx) = self.raycast_window_in_minimap(px, py) {
            self.focused_window = Some(win_idx);
            let w = &self.windows[win_idx].win;
            self.camera.look_at_spherical(w.azimuth, w.elevation);
            if self.minimap.expanded {
                self.minimap.expanded = false;
            }
        } else if let Some((click_az, click_el)) =
            self.minimap
                .pick_spherical_point(px, py, self.config.width, self.config.height)
        {
            self.camera.look_at_spherical(click_az, click_el);
            if self.minimap.expanded {
                self.minimap.expanded = false;
            }
        }
    }

    /// Converte a posição atual do mouse em coordenadas NDC [-1, 1] e faz raycasting 3D nas janelas
    /// (tanto na visão principal quanto nas profundidades reais dentro do Mini-Globo).
    fn update_raycast_hover(&mut self) {
        let w = self.config.width.max(1);
        let h = self.config.height.max(1);

        // 1. Verifica primeiro se o mouse está sobre o Mini-Globo
        self.minimap.is_hovered =
            self.minimap
                .contains_cursor(self.cursor_pos.0, self.cursor_pos.1, w, h);

        if self.minimap.is_hovered {
            let hovered_win = self.raycast_window_in_minimap(self.cursor_pos.0, self.cursor_pos.1);
            self.hovered_window = hovered_win;
            if let Some(idx) = hovered_win {
                let win = &self.windows[idx].win;
                self.window.set_title(&format!(
                    "XSphereDE [Mini-Globo] — Janela: [{}] (Profundidade R: {:.1}m — Use Scroll para mudar profundidade!)",
                    win.title, win.radius
                ));
            } else if let Some((az, el)) =
                self.minimap
                    .pick_spherical_point(self.cursor_pos.0, self.cursor_pos.1, w, h)
            {
                self.window.set_title(&format!(
                    "XSphereDE [Mini-Globo] — Clique Esq: Ir para ({:.0}°, {:.0}°) | Clique Dir ou 'G': Expandir/Recolher Globo",
                    az.to_degrees(),
                    el.to_degrees()
                ));
            }
        } else {
            // 2. Raycasting normal nas janelas 3D da cena principal
            let ndc_x = (self.cursor_pos.0 / w as f32) * 2.0 - 1.0;
            let ndc_y = 1.0 - (self.cursor_pos.1 / h as f32) * 2.0;
            let (ray_origin, ray_dir) = self.camera.screen_ray(ndc_x, ndc_y);

            let mut best_hit: Option<(usize, f32)> = None;
            for (idx, entry) in self.windows.iter().enumerate() {
                if let Some(t) = entry.win.intersect_ray(ray_origin, ray_dir) {
                    if best_hit.is_none_or(|(_, best_t)| t < best_t) {
                        best_hit = Some((idx, t));
                    }
                }
            }

            if self.hovered_window != best_hit.map(|(idx, _)| idx) {
                self.hovered_window = best_hit.map(|(idx, _)| idx);
                if let Some(idx) = self.hovered_window {
                    let win = &self.windows[idx].win;
                    self.window.set_title(&format!(
                        "XSphereDE — Em foco: [{}] (Azimute: {:.1}°, Elevação: {:.1}°, Profundidade R: {:.1}m — Scroll/Shift+Scroll altera profundidade)",
                        win.title,
                        win.azimuth.to_degrees(),
                        win.elevation.to_degrees(),
                        win.radius
                    ));
                } else {
                    self.window.set_title(
                        "XSphereDE [Rust + wgpu] — Mini-Globo no canto inf. direito (Clique Dir ou 'G': Expandir) | Scroll/WASD: Navegar",
                    );
                }
            }
        }

        for (idx, entry) in self.windows.iter_mut().enumerate() {
            entry.win.hovered = self.hovered_window == Some(idx);
            entry.win.focused = self.focused_window == Some(idx);
            let uniform = WindowStateUniform {
                state: [
                    entry.win.id as f32,
                    if entry.win.hovered { 1.0 } else { 0.0 },
                    if entry.win.focused { 1.0 } else { 0.0 },
                    0.94,
                ],
            };
            self.queue
                .write_buffer(&entry.state_buffer, 0, bytemuck::cast_slice(&[uniform]));
        }
    }

    fn update(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;

        // Movimentação espacial contínua (W/A/S/D + Q/E para navegar por entre os códigos Matrix)
        let speed = if self.shift_held { 18.0 } else { 9.5 } * dt;
        let mut fwd = 0.0;
        let mut right = 0.0;
        let mut up = 0.0;

        if self.keys_pressed.contains(&KeyCode::KeyW)
            || self.keys_pressed.contains(&KeyCode::ArrowUp)
        {
            fwd += speed;
        }
        if self.keys_pressed.contains(&KeyCode::KeyS)
            || self.keys_pressed.contains(&KeyCode::ArrowDown)
        {
            fwd -= speed;
        }
        if self.keys_pressed.contains(&KeyCode::KeyD)
            || self.keys_pressed.contains(&KeyCode::ArrowRight)
        {
            right += speed;
        }
        if self.keys_pressed.contains(&KeyCode::KeyA)
            || self.keys_pressed.contains(&KeyCode::ArrowLeft)
        {
            right -= speed;
        }
        if self.keys_pressed.contains(&KeyCode::KeyE) {
            up += speed;
        }
        if self.keys_pressed.contains(&KeyCode::KeyQ) {
            up -= speed;
        }

        if fwd != 0.0 || right != 0.0 || up != 0.0 {
            self.camera.translate_local(fwd, right, up);
        }

        self.camera.update(dt);
        self.minimap.update(dt);
        self.update_raycast_hover();

        let elapsed = (now - self.start_time).as_secs_f32();
        let cam_fwd = self.camera.forward();
        // Alcance frontal de visão limpa: garante corredor 100% limpo até depois da janela em foco
        let active_clear_dist = self
            .hovered_window
            .or(self.focused_window)
            .and_then(|idx| self.windows.get(idx))
            .map(|entry| (entry.win.world_center() - self.camera.position).length() + 1.5)
            .unwrap_or(11.8)
            .max(11.5);

        let mut window_shields = [[0.0_f32, 0.0, -10.0, 1.0]; 4];
        for (i, entry) in self.windows.iter().take(4).enumerate() {
            let center = entry.win.world_center();
            window_shields[i] = [center.x, center.y, center.z, entry.win.shield_cos()];
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
                self.focused_window.map(|i| i as f32).unwrap_or(-1.0),
                self.hovered_window.map(|i| i as f32).unwrap_or(-1.0),
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

        // Atualiza uniform do Mini-Globo HUD (janelas são renderizadas nas suas profundidades reais!)
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

    fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("XSphereDE Frame Encoder"),
            });

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("XSphereDE Main Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Fundo verde-abissal escuro do universo Matrix
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
            });

            // ================================================================
            // CENA 3D PRINCIPAL (Tela Inteira)
            // ================================================================
            // 1. Desenha a grade esférica do SphereXP
            rpass.set_pipeline(&self.grid_pipeline);
            rpass.set_bind_group(0, &self.globals_bind_group, &[]);
            rpass.set_vertex_buffer(0, self.grid_vertex_buffer.slice(..));
            rpass.draw(0..self.grid_vertex_count, 0..1);

            // 2. Desenha as janelas 3D curvas na esfera
            rpass.set_pipeline(&self.window_pipeline);
            rpass.set_bind_group(0, &self.globals_bind_group, &[]);
            for entry in &self.windows {
                rpass.set_bind_group(1, &entry.win.bind_group, &[]);
                rpass.set_vertex_buffer(0, entry.win.vertex_buffer.slice(..));
                rpass.set_index_buffer(
                    entry.win.index_buffer.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                rpass.draw_indexed(0..entry.win.index_count, 0, 0..1);
            }

            // 3. Desenha o volume tridimensional de fileiras de código Matrix
            rpass.set_pipeline(&self.matrix_pipeline);
            rpass.set_bind_group(0, &self.globals_bind_group, &[]);
            rpass.set_vertex_buffer(0, self.matrix_vertex_buffer.slice(..));
            rpass.set_index_buffer(
                self.matrix_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            rpass.draw_indexed(0..self.matrix_index_count, 0, 0..1);

            // ================================================================
            // 4. MINI-MAPA DO GLOBO 3D INTERATIVO (HUD no canto inf. direito)
            // ================================================================
            let (hx, hy, hsize) = self
                .minimap
                .viewport_rect(self.config.width, self.config.height);
            if hx + hsize <= self.config.width as f32 && hy + hsize <= self.config.height as f32 {
                rpass.set_viewport(hx, hy, hsize, hsize, 0.0, 1.0);

                // 4a. Orbe holográfico de fundo (também reseta o depth buffer do Mini-Globo para 0.999)
                rpass.set_pipeline(&self.minimap_bg_pipeline);
                rpass.set_bind_group(0, &self.minimap_globals_bind_group, &[]);
                rpass.draw(0..6, 0..1);

                // 4b. Grade esférica 3D vista por fora no Mini-Globo
                rpass.set_pipeline(&self.grid_pipeline);
                rpass.set_bind_group(0, &self.minimap_globals_bind_group, &[]);
                rpass.set_vertex_buffer(0, self.grid_vertex_buffer.slice(..));
                rpass.draw(0..self.grid_vertex_count, 0..1);

                // 4c. Janelas 3D renderizadas em suas profundidades radiais reais dentro do Mini-Globo!
                rpass.set_pipeline(&self.window_pipeline);
                rpass.set_bind_group(0, &self.minimap_globals_bind_group, &[]);
                for entry in &self.windows {
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
        output.present();
        Ok(())
    }
}

#[derive(Default)]
struct XSphereApp {
    renderer: Option<XSphereRenderer>,
}

impl ApplicationHandler for XSphereApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_none() {
            let win_attrs = Window::default_attributes()
                .with_title(
                    "XSphereDE [Rust + wgpu] — Mini-Globo no canto inf. direito (Girar / Clicar) | Scroll/WASD: Voar pelos Códigos Matrix",
                )
                .with_inner_size(PhysicalSize::new(1366, 840));

            let window = Arc::new(
                event_loop
                    .create_window(win_attrs)
                    .expect("Falha ao criar janela do XSphereDE"),
            );

            let renderer = pollster::block_on(XSphereRenderer::new(window));
            self.renderer = Some(renderer);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => renderer.resize(size),
            WindowEvent::ModifiersChanged(mods) => {
                renderer.shift_held = mods.state().shift_key();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    match event.state {
                        ElementState::Pressed => {
                            renderer.keys_pressed.insert(code);
                            match code {
                                KeyCode::Escape => {
                                    if renderer.minimap.expanded {
                                        renderer.minimap.expanded = false;
                                    } else {
                                        event_loop.exit();
                                    }
                                }
                                KeyCode::KeyG => {
                                    renderer.minimap.toggle_expanded();
                                }
                                KeyCode::Space | KeyCode::Tab => {
                                    renderer.free_look_mode = !renderer.free_look_mode;
                                }
                                KeyCode::KeyR | KeyCode::Home => {
                                    renderer.camera.reset_to_center();
                                    renderer.minimap.target_orb_yaw = 0.0;
                                    renderer.minimap.target_orb_pitch = 0.32;
                                }
                                KeyCode::KeyM => {
                                    renderer.matrix_speed_mult =
                                        if renderer.matrix_speed_mult > 0.2 {
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
                            renderer.keys_pressed.remove(&code);
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let (px, py) = renderer.cursor_pos;
                let on_minimap = renderer.minimap.contains_cursor(
                    px,
                    py,
                    renderer.config.width,
                    renderer.config.height,
                );

                match button {
                    MouseButton::Middle => {
                        if state == ElementState::Pressed && on_minimap {
                            renderer.minimap.toggle_expanded();
                        } else {
                            renderer.right_mouse_down = state == ElementState::Pressed;
                        }
                    }
                    MouseButton::Right => {
                        renderer.right_mouse_down = state == ElementState::Pressed;
                        if state == ElementState::Pressed && on_minimap {
                            renderer.minimap_dragging = true;
                            renderer.minimap_drag_distance = 0.0;
                        } else if state == ElementState::Released && renderer.minimap_dragging {
                            renderer.minimap_dragging = false;
                            // Clique direito rápido no Mini-Globo expande/recolhe o Globo na tela!
                            if renderer.minimap_drag_distance < 6.0 && on_minimap {
                                renderer.minimap.toggle_expanded();
                            }
                        }
                    }
                    MouseButton::Left => {
                        renderer.left_mouse_down = state == ElementState::Pressed;
                        if state == ElementState::Pressed {
                            if on_minimap {
                                renderer.minimap_dragging = true;
                                renderer.minimap_drag_distance = 0.0;
                                renderer.dragging_window = None;
                            } else if let Some(hovered) = renderer.hovered_window {
                                renderer.focused_window = Some(hovered);
                                renderer.dragging_window = Some(hovered);
                            } else {
                                renderer.dragging_window = None;
                            }
                        } else {
                            // Soltou o botão esquerdo: se foi um clique rápido no Mini-Globo, gira o ambiente até lá!
                            if renderer.minimap_dragging {
                                renderer.minimap_dragging = false;
                                if renderer.minimap_drag_distance < 6.0 {
                                    renderer.jump_to_minimap_point(px, py);
                                }
                            }
                            renderer.dragging_window = None;
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let new_pos = (position.x as f32, position.y as f32);
                if let Some(last) = renderer.last_cursor_pos {
                    let dx = new_pos.0 - last.0;
                    let dy = new_pos.1 - last.1;

                    if renderer.minimap_dragging {
                        let step_dist = (dx * dx + dy * dy).sqrt();
                        renderer.minimap_drag_distance += step_dist;
                        // Arrastar no Mini-Globo gira o globo 3D!
                        renderer.minimap.rotate_globe(dx, dy);
                    } else if let Some(win_idx) = renderer.dragging_window {
                        // Arrasta a janela 3D ao longo da superfície curva da esfera (azimuth, elevation)
                        let entry = &mut renderer.windows[win_idx];
                        entry.win.azimuth += dx * 0.0032;
                        entry.win.elevation =
                            (entry.win.elevation - dy * 0.0032).clamp(-1.15, 1.15);
                        entry.win.update_geometry(&renderer.queue);
                    } else if renderer.right_mouse_down
                        || renderer.free_look_mode
                        || renderer.left_mouse_down
                    {
                        renderer.camera.rotate(dx, dy, 0.0038);
                    }
                }
                renderer.cursor_pos = new_pos;
                renderer.last_cursor_pos = Some(new_pos);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let scroll_y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(pos) => (pos.y as f32) * 0.04,
                };

                let (px, py) = renderer.cursor_pos;
                let on_minimap = renderer.minimap.contains_cursor(
                    px,
                    py,
                    renderer.config.width,
                    renderer.config.height,
                );

                // Se o cursor está sobre uma janela (na cena principal OU dentro do Mini-Globo!)
                // ou se Shift está pressionado, o Scroll altera a profundidade radial da janela dentro do globo!
                if let Some(win_idx) = renderer.hovered_window.or(if renderer.shift_held {
                    renderer.focused_window
                } else {
                    None
                }) {
                    let entry = &mut renderer.windows[win_idx];
                    entry.win.radius = (entry.win.radius - scroll_y * 0.95).clamp(4.0, 24.5);
                    entry.win.update_geometry(&renderer.queue);
                } else if on_minimap {
                    // Scroll sobre área vazia do Mini-Globo gira o globo horizontalmente
                    renderer.minimap.rotate_globe(scroll_y * 28.0, 0.0);
                } else {
                    // Scroll sobre espaço vazio da esfera voa para frente/trás
                    renderer.camera.translate_local(scroll_y * 1.65, 0.0, 0.0);
                }
            }
            WindowEvent::RedrawRequested => {
                renderer.update();
                match renderer.render() {
                    Ok(_) => {}
                    Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                        let size = renderer.window.inner_size();
                        renderer.resize(size);
                    }
                    Err(wgpu::SurfaceError::OutOfMemory) => event_loop.exit(),
                    Err(e) => log::warn!("Erro de superfície: {:?}", e),
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

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    log::info!("Iniciando XSphereDE — Ambiente Desktop 3D Esférico em Rust...");

    let event_loop = EventLoop::new().expect("Falha ao criar EventLoop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = XSphereApp::default();
    event_loop
        .run_app(&mut app)
        .expect("Erro durante a execução do XSphereDE");
}
