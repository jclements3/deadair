//! The optics renderer: one geometry pass, three optic post pipelines
//! (SDD §4). Headless-capable — used by golden tests and the editor's
//! thermal preview as well as the game window.

use crate::draw::{Camera, DrawList, Shape};
use crate::mesh;
use crate::palette::{Agc, TempSample, ThermalPalette};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

/// Which optic pipeline to run (SRS FR-O1: exactly one at a time).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpticMode {
    Eye,
    Nv,
    Thermal,
}

/// Per-frame optic tuning fed by weather, battery state, and device tier.
#[derive(Debug, Clone)]
pub struct OpticSettings {
    pub mode: OpticMode,
    pub palette: ThermalPalette,
    /// Draw the circular scope vignette (scoped view vs. unaided view).
    pub scope_mask: bool,
    /// Frame counter for NV grain animation.
    pub frame: u32,
    /// Grain seed (per-device).
    pub seed: u32,
    /// NV gain (weather-degraded NV runs hotter gain → grainier).
    pub nv_gain: f32,
    /// WeatherMods.nv_visibility.
    pub nv_visibility: f32,
    /// Naked-eye exposure multiplier.
    pub eye_exposure: f32,
    /// Simulated sensor resolution (square side): the whole optic chain
    /// renders at this size and is soft-upscaled to the view, exactly like
    /// a real device scaling its 256×192 core to the eyepiece display.
    /// `None` = native (the unaided eye has no sensor).
    pub sensor_res: Option<u32>,
}

impl Default for OpticSettings {
    fn default() -> Self {
        Self {
            mode: OpticMode::Eye,
            palette: ThermalPalette::WhiteHot,
            scope_mask: false,
            frame: 0,
            seed: 1,
            nv_gain: 1.0,
            nv_visibility: 1.0,
            eye_exposure: 1.0,
            sensor_res: None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct OpticParams {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    model: [[f32; 4]; 4],
    albedo_emissive: [f32; 4],
    temp_glass: [f32; 4],
}

/// One ground-projected heat decal instance (`shaders/decal.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DecalInstance {
    center_radius: [f32; 4],
    params: [f32; 4],
}

/// One eyeshine point sprite (`shaders/eyeshine.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct EyeInstance {
    pos_strength: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BlurParams {
    p: [f32; 4],
}

/// Upload an instance array, tolerating the empty case (wgpu rejects
/// zero-sized vertex buffers, and an unused one-instance buffer is free).
fn instance_buffer<T: Pod>(device: &wgpu::Device, v: &[T]) -> wgpu::Buffer {
    let empty = vec![0u8; std::mem::size_of::<T>()];
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: if v.is_empty() {
            &empty
        } else {
            bytemuck::cast_slice(v)
        },
        usage: wgpu::BufferUsages::VERTEX,
    })
}

/// Bounding-sphere radius of a primitive, for AGC coverage weighting.
fn bounding_radius(shape: &Shape) -> f32 {
    match *shape {
        Shape::Box { half } => half.length(),
        Shape::Cylinder { radius, height } => {
            (radius * radius + 0.25 * height * height).sqrt()
        }
        Shape::Sphere { radius } => radius,
        Shape::GroundPatch { half } => half,
        Shape::Mesh { .. } => 1.0,
    }
}

/// Fraction of the frame a bounding sphere covers, as a solid-angle ratio.
/// Crude on purpose: the AGC only needs to know "big" from "speck".
fn screen_coverage(cam: &Camera, center: glam::Vec3, radius: f32) -> f32 {
    let dist = (center - cam.eye).length().max(0.05);
    let theta = (radius.max(0.0) / dist).atan();
    let fov_y = cam.fov_y_deg.to_radians().max(1e-3);
    let fov_x = 2.0 * ((fov_y * 0.5).tan() * cam.aspect.max(1e-3)).atan();
    (std::f32::consts::PI * theta * theta / (fov_y * fov_x)).clamp(0.0, 1.0)
}

/// Fraction of the frame above the horizon, from the camera's pitch. Sky is
/// the coldest thing in any night scene, so the AGC needs its real weight
/// rather than treating it as one more sample.
fn sky_fraction(cam: &Camera) -> f32 {
    let dir = (cam.look - cam.eye).normalize_or_zero();
    let half = (cam.fov_y_deg.to_radians() * 0.5).clamp(1e-3, 1.5);
    // NDC height of the horizon line (positive = camera is looking down).
    let ndc = (-dir.y.clamp(-0.999, 0.999).asin()).tan() / half.tan();
    ((1.0 - ndc.clamp(-1.0, 1.0)) * 0.5).clamp(0.0, 1.0)
}

struct GpuMesh {
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    index_count: u32,
}

fn upload_mesh(device: &wgpu::Device, m: &mesh::MeshData) -> GpuMesh {
    GpuMesh {
        vbuf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&m.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        }),
        ibuf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&m.indices),
            usage: wgpu::BufferUsages::INDEX,
        }),
        index_count: m.indices.len() as u32,
    }
}

/// Resolution-dependent render targets: rebuilt when the simulated sensor
/// resolution changes (a handful of small textures — cheap, and only on
/// optic swaps).
struct Targets {
    res: u32,
    color_tex: wgpu::Texture,
    temp_tex: wgpu::Texture,
    depth_tex: wgpu::Texture,
    /// The optic pass lands here at sensor res, then upscales to `out_tex`.
    optic_mid: wgpu::Texture,
    bloom_a: wgpu::Texture,
    bloom_b: wgpu::Texture,
    bloom_h_bind: wgpu::BindGroup,
    bloom_v_bind: wgpu::BindGroup,
}

fn make_targets(
    device: &wgpu::Device,
    res: u32,
    bloom_layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
) -> Targets {
    let mk = |w: u32, h: u32, fmt, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: fmt,
            usage,
            view_formats: &[],
        })
    };
    let rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
    let color_tex = mk(res, res, COLOR_FMT, rt);
    let temp_tex = mk(res, res, TEMP_FMT, rt);
    let depth_tex = mk(res, res, DEPTH_FMT, rt);
    let optic_mid = mk(res, res, OUT_FMT, rt);
    let (bw, bh) = ((res / 2).max(1), (res / 2).max(1));
    let bloom_a = mk(bw, bh, COLOR_FMT, rt);
    let bloom_b = mk(bw, bh, COLOR_FMT, rt);
    let mk_blur_buf = |v: [f32; 4]| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("blur"),
            contents: bytemuck::cast_slice(&v),
            usage: wgpu::BufferUsages::UNIFORM,
        })
    };
    let bloom_h_buf = mk_blur_buf([1.5 / bw as f32, 0.0, 6.0, 0.0]);
    let bloom_v_buf = mk_blur_buf([0.0, 1.5 / bh as f32, 1.0, 0.0]);
    let mk_bloom_bind = |buf: &wgpu::Buffer, src: &wgpu::Texture| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom"),
            layout: bloom_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &src.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    };
    let bloom_h_bind = mk_bloom_bind(&bloom_h_buf, &color_tex);
    let bloom_v_bind = mk_bloom_bind(&bloom_v_buf, &bloom_a);
    Targets {
        res,
        color_tex,
        temp_tex,
        depth_tex,
        optic_mid,
        bloom_a,
        bloom_b,
        bloom_h_bind,
        bloom_v_bind,
    }
}

/// Renders DrawLists through the three optic pipelines at a fixed size.
pub struct Renderer {
    width: u32,
    height: u32,
    targets: Targets,
    out_tex: wgpu::Texture,
    geom_pipeline: wgpu::RenderPipeline,
    decal_pipeline: wgpu::RenderPipeline,
    eyeshine_pipeline: wgpu::RenderPipeline,
    bloom_h_pipeline: wgpu::RenderPipeline,
    bloom_v_pipeline: wgpu::RenderPipeline,
    optic_pipeline: wgpu::RenderPipeline,
    globals_buf: wgpu::Buffer,
    optic_buf: wgpu::Buffer,
    geom_bind: wgpu::BindGroup,
    optic_layout: wgpu::BindGroupLayout,
    bloom_layout: wgpu::BindGroupLayout,
    upscale_pipeline: wgpu::RenderPipeline,
    upscale_layout: wgpu::BindGroupLayout,
    upscale_buf: wgpu::Buffer,
    sampler: wgpu::Sampler,
    palette_cache: Option<(ThermalPalette, wgpu::Texture)>,
    box_mesh: GpuMesh,
    cyl_mesh: GpuMesh,
    sphere_mesh: GpuMesh,
    ground_mesh: GpuMesh,
    /// Thermal auto-gain window state, advanced per frame.
    pub agc: Agc,
}

const COLOR_FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const TEMP_FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
const DEPTH_FMT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const OUT_FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl Renderer {
    pub fn new(gpu: &crate::gpu::Gpu, width: u32, height: u32) -> Self {
        Self::new_on(&gpu.device, width, height)
    }

    /// Like [`Renderer::new`] but on a borrowed device (e.g. eframe's).
    pub fn new_on(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let mk_tex_sized = |w: u32, h: u32, fmt, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: fmt,
                usage,
                view_formats: &[],
            })
        };
        let mk_tex = |fmt, usage| mk_tex_sized(width, height, fmt, usage);
        let rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let _ = rt;
        let out_tex = mk_tex(
            OUT_FMT,
            // COPY_SRC: headless readback; TEXTURE_BINDING: the game shows
            // this texture through egui, which samples it in a bind group.
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        // Bloom runs at half resolution — the blur hides the upsample and it
        // keeps the headless llvmpipe tests quick.
        let (bw, bh) = ((width / 2).max(1), (height / 2).max(1));

        let geom_shader =
            device.create_shader_module(wgpu::include_wgsl!("shaders/geom.wgsl"));
        let decal_shader =
            device.create_shader_module(wgpu::include_wgsl!("shaders/decal.wgsl"));
        let eyeshine_shader =
            device.create_shader_module(wgpu::include_wgsl!("shaders/eyeshine.wgsl"));
        let bloom_shader =
            device.create_shader_module(wgpu::include_wgsl!("shaders/bloom.wgsl"));
        let optic_shader =
            device.create_shader_module(wgpu::include_wgsl!("shaders/optic.wgsl"));

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let optic_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: std::mem::size_of::<OpticParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let geom_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
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
        let geom_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &geom_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<mesh::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
        };
        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Instance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![
                2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4,
                6 => Float32x4, 7 => Float32x4
            ],
        };

        let geom_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&geom_layout],
            push_constant_ranges: &[],
        });
        let geom_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("geom"),
            layout: Some(&geom_pl),
            vertex: wgpu::VertexState {
                module: &geom_shader,
                entry_point: Some("vs_main"),
                buffers: &[vertex_layout.clone(), instance_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &geom_shader,
                entry_point: Some("fs_main"),
                targets: &[
                    Some(COLOR_FMT.into()),
                    Some(TEMP_FMT.into()),
                ],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FMT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let additive = Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        });

        // Heat decals: additive into the temperature target only. The color
        // target is write-masked off, which is *why* decals cannot leak into
        // the eye/NV pipelines — they never touch the light buffer.
        let decal_instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<DecalInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![2 => Float32x4, 3 => Float32x4],
        };
        let decal_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("heat-decal"),
            layout: Some(&geom_pl),
            vertex: wgpu::VertexState {
                module: &decal_shader,
                entry_point: Some("vs_main"),
                buffers: &[vertex_layout.clone(), decal_instance_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &decal_shader,
                entry_point: Some("fs_main"),
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: COLOR_FMT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::empty(),
                    }),
                    Some(wgpu::ColorTargetState {
                        format: TEMP_FMT,
                        blend: additive,
                        write_mask: wgpu::ColorWrites::RED,
                    }),
                ],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FMT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        // Eyeshine: additive point sprites into the color+emissive target,
        // temperature target masked off. Depth test is disabled — the caller
        // decides which eyes are visible (see `DrawList::eyeshine`).
        let eye_instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<EyeInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x4],
        };
        let eyeshine_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("eyeshine"),
            layout: Some(&geom_pl),
            vertex: wgpu::VertexState {
                module: &eyeshine_shader,
                entry_point: Some("vs_main"),
                buffers: &[eye_instance_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &eyeshine_shader,
                entry_point: Some("fs_main"),
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: COLOR_FMT,
                        blend: additive,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: TEMP_FMT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::empty(),
                    }),
                ],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FMT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        // Separable emissive bloom.
        let bloom_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bloom_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bloom_layout],
            push_constant_ranges: &[],
        });
        let mk_bloom_pipeline = |entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("bloom"),
                layout: Some(&bloom_pl),
                vertex: wgpu::VertexState {
                    module: &bloom_shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &bloom_shader,
                    entry_point: Some(entry),
                    targets: &[Some(COLOR_FMT.into())],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let bloom_h_pipeline = mk_bloom_pipeline("fs_extract_h");
        let bloom_v_pipeline = mk_bloom_pipeline("fs_blur_v");

        let optic_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let optic_pl =device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&optic_layout],
            push_constant_ranges: &[],
        });
        let optic_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("optic"),
            layout: Some(&optic_pl),
            vertex: wgpu::VertexState {
                module: &optic_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &optic_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(OUT_FMT.into())],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Blur taps/gain rationale lives in make_targets (rebuilt per
        // sensor resolution).
        let targets = make_targets(device, width.min(height), &bloom_layout, &sampler);

        // Device-scaler upscale: sensor-res optic output -> native out_tex.
        let upscale_shader =
            device.create_shader_module(wgpu::include_wgsl!("shaders/blit.wgsl"));
        let upscale_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("upscale"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let upscale_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&upscale_layout],
            push_constant_ranges: &[],
        });
        let upscale_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("upscale"),
            layout: Some(&upscale_pl),
            vertex: wgpu::VertexState {
                module: &upscale_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &upscale_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(OUT_FMT.into())],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let upscale_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("upscale params"),
            contents: bytemuck::cast_slice(&[1.0f32, 1.0, 0.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        Self {
            width,
            height,
            targets,
            out_tex,
            geom_pipeline,
            decal_pipeline,
            eyeshine_pipeline,
            bloom_h_pipeline,
            bloom_v_pipeline,
            optic_pipeline,
            globals_buf,
            optic_buf,
            geom_bind,
            optic_layout,
            bloom_layout,
            upscale_pipeline,
            upscale_layout,
            upscale_buf,
            sampler,
            palette_cache: None,
            box_mesh: upload_mesh(device, &mesh::unit_box()),
            cyl_mesh: upload_mesh(device, &mesh::unit_cylinder(20)),
            sphere_mesh: upload_mesh(device, &mesh::unit_sphere(14, 20)),
            ground_mesh: upload_mesh(device, &mesh::ground_patch(16)),
            agc: Agc::new(),
        }
    }

    fn palette_tex(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, palette: ThermalPalette) -> wgpu::TextureView {
        let stale = match &self.palette_cache {
            Some((p, _)) => *p != palette,
            None => true,
        };
        if stale {
            let lut = palette.lut();
            let mut data = Vec::with_capacity(256 * 4);
            for c in lut {
                data.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
            let tex = device.create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("palette"),
                    size: wgpu::Extent3d {
                        width: 256,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &data,
            );
            self.palette_cache = Some((palette, tex));
        }
        self.palette_cache
            .as_ref()
            .expect("palette cache populated above")
            .1
            .create_view(&Default::default())
    }

    /// Render one frame. `dt` advances the thermal AGC window.
    pub fn render(
        &mut self,
        gpu: &crate::gpu::Gpu,
        list: &DrawList,
        cam: &Camera,
        settings: &OpticSettings,
        dt: f32,
    ) {
        self.render_on(&gpu.device, &gpu.queue, list, cam, settings, dt)
    }

    /// Like [`Renderer::render`] but on borrowed device/queue.
    pub fn render_on(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        list: &DrawList,
        cam: &Camera,
        settings: &OpticSettings,
        dt: f32,
    ) {
        // Simulated sensor: the whole optic chain renders at the device's
        // native resolution, then soft-upscales to the eyepiece view.
        let native = self.width.min(self.height);
        let want = settings
            .sensor_res
            .unwrap_or(native)
            .clamp(64, native);
        if self.targets.res != want {
            self.targets = make_targets(device, want, &self.bloom_layout, &self.sampler);
        }
        // Sort instances by primitive.
        let mut boxes = Vec::new();
        let mut cyls = Vec::new();
        let mut spheres = Vec::new();
        let mut grounds = Vec::new();
        let mut samples: Vec<TempSample> = Vec::with_capacity(list.items.len() + 1);
        // Sky is a real, large part of the histogram — weight it by how much
        // of the frame is actually above the horizon.
        let sky_frac = sky_fraction(cam);
        samples.push(TempSample {
            temp_f: list.sky_temp_f,
            weight: sky_frac,
        });
        let ground_weight = (1.0 - sky_frac).max(0.0)
            / list
                .items
                .iter()
                .filter(|i| matches!(i.shape, Shape::GroundPatch { .. }))
                .count()
                .max(1) as f32;
        for item in &list.items {
            let center = item.world.transform_point3(glam::Vec3::ZERO);
            let weight = match item.shape {
                Shape::GroundPatch { .. } => ground_weight,
                _ => screen_coverage(cam, center, bounding_radius(&item.shape)),
            };
            samples.push(TempSample {
                temp_f: item.temp_f,
                weight,
            });
            // Ground-scale surfaces get static thermal/albedo mottling
            // (temp_glass.z): real ground is never a flat field — the
            // HIKMICRO footage shows dark mottled texture everywhere. The
            // hash keys off world position, so the pattern is bolted to
            // the terrain: zero temporal shimmer. Heuristic: ground
            // patches, plus boxes big enough to be zone ground slabs.
            let ground_noise = match item.shape {
                Shape::GroundPatch { .. } => 6.5,
                Shape::Box { half } if half.x > 20.0 && half.y < 1.0 => 6.5,
                _ => 0.0,
            };
            let inst = |scale: glam::Mat4| Instance {
                model: (item.world * scale).to_cols_array_2d(),
                albedo_emissive: [
                    item.albedo[0],
                    item.albedo[1],
                    item.albedo[2],
                    item.emissive,
                ],
                temp_glass: [
                    item.temp_f,
                    if item.glass { 1.0 } else { 0.0 },
                    ground_noise,
                    0.0,
                ],
            };
            match item.shape {
                Shape::Box { half } => boxes.push(inst(glam::Mat4::from_scale(half))),
                Shape::Cylinder { radius, height } => cyls.push(inst(glam::Mat4::from_scale(
                    glam::Vec3::new(radius, height, radius),
                ))),
                Shape::Sphere { radius } => {
                    spheres.push(inst(glam::Mat4::from_scale(glam::Vec3::splat(radius))))
                }
                Shape::GroundPatch { half } => grounds.push(inst(glam::Mat4::from_scale(
                    glam::Vec3::new(half, 1.0, half),
                ))),
                Shape::Mesh { .. } => {} // registered meshes: later
            }
        }
        // Heat decals: ground-projected warm discs, thermal only (SDD §2.3).
        let decals: Vec<DecalInstance> = list
            .heat_decals
            .iter()
            .map(|d| DecalInstance {
                center_radius: [d.pos.x, d.pos.y, d.pos.z, d.radius_m.max(0.01)],
                params: [d.delta_f, 0.0, 0.0, 0.0],
            })
            .collect();
        for d in &list.heat_decals {
            samples.push(TempSample {
                temp_f: list.ambient_f + d.delta_f,
                weight: screen_coverage(cam, d.pos, d.radius_m),
            });
        }
        // Eyeshine: NV-only retro-reflections (see `DrawList::eyeshine`).
        let eyes: Vec<EyeInstance> = if settings.mode == OpticMode::Nv {
            list.eyeshine
                .iter()
                .map(|e| EyeInstance {
                    pos_strength: [e.pos.x, e.pos.y, e.pos.z, e.strength],
                })
                .collect()
        } else {
            Vec::new()
        };

        // Advance the thermal AGC over this frame's coverage-weighted
        // temperature histogram (percentile window — see palette::Agc).
        if samples.len() <= 1 {
            self.agc
                .update(list.sky_temp_f.min(list.ambient_f), list.ambient_f + 1.0, dt);
        } else {
            self.agc.update_weighted(&samples, dt);
        }

        queue.write_buffer(
            &self.globals_buf,
            0,
            bytemuck::bytes_of(&Globals {
                view_proj: cam.view_proj().to_cols_array_2d(),
                params: [
                    list.moonlight,
                    list.ambient_f,
                    list.sky_temp_f,
                    self.width as f32 / self.height.max(1) as f32,
                ],
            }),
        );
        let mode_f = match settings.mode {
            OpticMode::Eye => 0.0,
            OpticMode::Nv => 1.0,
            OpticMode::Thermal => 2.0,
        };
        queue.write_buffer(
            &self.optic_buf,
            0,
            bytemuck::bytes_of(&OpticParams {
                a: [
                    mode_f,
                    settings.frame as f32,
                    settings.seed as f32,
                    settings.nv_gain,
                ],
                b: [
                    self.agc.lo_f,
                    self.agc.hi_f,
                    list.sky_temp_f,
                    if settings.scope_mask { 1.0 } else { 0.0 },
                ],
                c: [
                    settings.eye_exposure,
                    settings.nv_visibility,
                    list.moonlight,
                    self.width as f32 / self.height as f32,
                ],
            }),
        );

        let palette_view = self.palette_tex(device, queue, settings.palette);

        let mk_inst_buf = |v: &[Instance]| instance_buffer(device, v);
        let bufs = [
            (mk_inst_buf(&boxes), boxes.len() as u32, &self.box_mesh),
            (mk_inst_buf(&cyls), cyls.len() as u32, &self.cyl_mesh),
            (mk_inst_buf(&spheres), spheres.len() as u32, &self.sphere_mesh),
            (mk_inst_buf(&grounds), grounds.len() as u32, &self.ground_mesh),
        ];

        let color_view = self.targets.color_tex.create_view(&Default::default());
        let temp_view = self.targets.temp_tex.create_view(&Default::default());
        let depth_view = self.targets.depth_tex.create_view(&Default::default());
        let out_view = self.out_tex.create_view(&Default::default());
        let mid_view = self.targets.optic_mid.create_view(&Default::default());
        let bloom_a_view = self.targets.bloom_a.create_view(&Default::default());
        let bloom_view = self.targets.bloom_b.create_view(&Default::default());
        let decal_buf = instance_buffer(device, &decals);
        let eye_buf = instance_buffer(device, &eyes);

        let optic_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.optic_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.optic_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&temp_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&palette_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&bloom_view),
                },
            ],
        });

        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("geom"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &color_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            // Alpha is the emissive channel, so it must clear
                            // to 0 — `Color::BLACK` has alpha 1 and would
                            // make the empty sky a full-strength light source
                            // (and, once bloom exists, blow out the frame).
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &temp_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.geom_pipeline);
            pass.set_bind_group(0, &self.geom_bind, &[]);
            for (ibuf, count, gmesh) in &bufs {
                if *count == 0 {
                    continue;
                }
                pass.set_vertex_buffer(0, gmesh.vbuf.slice(..));
                pass.set_vertex_buffer(1, ibuf.slice(..));
                pass.set_index_buffer(gmesh.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..gmesh.index_count, 0, 0..*count);
            }
            if !decals.is_empty() {
                pass.set_pipeline(&self.decal_pipeline);
                pass.set_vertex_buffer(0, self.ground_mesh.vbuf.slice(..));
                pass.set_vertex_buffer(1, decal_buf.slice(..));
                pass.set_index_buffer(
                    self.ground_mesh.ibuf.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                pass.draw_indexed(
                    0..self.ground_mesh.index_count,
                    0,
                    0..decals.len() as u32,
                );
            }
            if !eyes.is_empty() {
                pass.set_pipeline(&self.eyeshine_pipeline);
                pass.set_vertex_buffer(0, eye_buf.slice(..));
                pass.draw(0..6, 0..eyes.len() as u32);
            }
        }
        // Emissive bloom: extract+blur H into bloom_a, blur V into bloom_b.
        // Thermal never reads it, so skip the work entirely there.
        if settings.mode != OpticMode::Thermal {
            for (pipeline, bind, target) in [
                (&self.bloom_h_pipeline, &self.targets.bloom_h_bind, &bloom_a_view),
                (&self.bloom_v_pipeline, &self.targets.bloom_v_bind, &bloom_view),
            ] {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("bloom"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, bind, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("optic"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &mid_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.optic_pipeline);
            pass.set_bind_group(0, &optic_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            // Device-scaler pass: linear stretch of the sensor image to the
            // eyepiece. At native res this is a 1:1 copy; at 192 it *is*
            // the Mk I look.
            let up_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("upscale"),
                layout: &self.upscale_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.upscale_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&mid_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("upscale"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &out_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.upscale_pipeline);
            pass.set_bind_group(0, &up_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([enc.finish()]);
    }

    /// Read back the last rendered frame as tightly-packed RGBA8.
    pub fn read_rgba(&self, gpu: &crate::gpu::Gpu) -> Vec<u8> {
        self.read_rgba_on(&gpu.device, &gpu.queue)
    }

    /// Like [`Renderer::read_rgba`] but on borrowed device/queue.
    pub fn read_rgba_on(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<u8> {
        let bytes_per_row_unpadded = self.width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let bytes_per_row = bytes_per_row_unpadded.div_ceil(align) * align;
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (bytes_per_row * self.height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &self.out_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &buf,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| {
            r.expect("readback map failed");
        });
        device.poll(wgpu::Maintain::Wait);
        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity((self.width * self.height * 4) as usize);
        for row in 0..self.height {
            let start = (row * bytes_per_row) as usize;
            out.extend_from_slice(&data[start..start + bytes_per_row_unpadded as usize]);
        }
        out
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// View of the final optic output (for the present blit / egui image).
    pub fn output_view(&self) -> wgpu::TextureView {
        self.out_tex.create_view(&Default::default())
    }
}
