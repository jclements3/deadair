//! The optics renderer: one geometry pass, three optic post pipelines
//! (SDD §4). Headless-capable — used by golden tests and the editor's
//! thermal preview as well as the game window.

use crate::draw::{Camera, DrawList, Shape};
use crate::mesh;
use crate::palette::{Agc, ThermalPalette};
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

/// Renders DrawLists through the three optic pipelines at a fixed size.
pub struct Renderer {
    width: u32,
    height: u32,
    color_tex: wgpu::Texture,
    temp_tex: wgpu::Texture,
    depth_tex: wgpu::Texture,
    out_tex: wgpu::Texture,
    geom_pipeline: wgpu::RenderPipeline,
    optic_pipeline: wgpu::RenderPipeline,
    globals_buf: wgpu::Buffer,
    optic_buf: wgpu::Buffer,
    geom_bind: wgpu::BindGroup,
    optic_layout: wgpu::BindGroupLayout,
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
        let device = &gpu.device;
        let mk_tex = |fmt, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width,
                    height,
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
        let rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let color_tex = mk_tex(COLOR_FMT, rt);
        let temp_tex = mk_tex(TEMP_FMT, rt);
        let depth_tex = mk_tex(DEPTH_FMT, rt);
        let out_tex = mk_tex(
            OUT_FMT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );

        let geom_shader =
            device.create_shader_module(wgpu::include_wgsl!("shaders/geom.wgsl"));
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
                buffers: &[vertex_layout, instance_layout],
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
            ],
        });
        let optic_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
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

        Self {
            width,
            height,
            color_tex,
            temp_tex,
            depth_tex,
            out_tex,
            geom_pipeline,
            optic_pipeline,
            globals_buf,
            optic_buf,
            geom_bind,
            optic_layout,
            sampler,
            palette_cache: None,
            box_mesh: upload_mesh(device, &mesh::unit_box()),
            cyl_mesh: upload_mesh(device, &mesh::unit_cylinder(20)),
            sphere_mesh: upload_mesh(device, &mesh::unit_sphere(14, 20)),
            ground_mesh: upload_mesh(device, &mesh::ground_patch(16)),
            agc: Agc::new(),
        }
    }

    fn palette_tex(&mut self, gpu: &crate::gpu::Gpu, palette: ThermalPalette) -> wgpu::TextureView {
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
            let tex = gpu.device.create_texture_with_data(
                &gpu.queue,
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
        // Sort instances by primitive.
        let mut boxes = Vec::new();
        let mut cyls = Vec::new();
        let mut spheres = Vec::new();
        let mut grounds = Vec::new();
        let mut t_lo = f32::MAX;
        let mut t_hi = f32::MIN;
        for item in &list.items {
            t_lo = t_lo.min(item.temp_f);
            t_hi = t_hi.max(item.temp_f);
            let inst = |scale: glam::Mat4| Instance {
                model: (item.world * scale).to_cols_array_2d(),
                albedo_emissive: [
                    item.albedo[0],
                    item.albedo[1],
                    item.albedo[2],
                    item.emissive,
                ],
                temp_glass: [item.temp_f, if item.glass { 1.0 } else { 0.0 }, 0.0, 0.0],
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
        // Advance thermal AGC toward this frame's window (include sky).
        if t_lo > t_hi {
            t_lo = list.ambient_f;
            t_hi = list.ambient_f + 1.0;
        }
        self.agc.update(t_lo.min(list.sky_temp_f), t_hi, dt);

        gpu.queue.write_buffer(
            &self.globals_buf,
            0,
            bytemuck::bytes_of(&Globals {
                view_proj: cam.view_proj().to_cols_array_2d(),
                params: [list.moonlight, list.ambient_f, list.sky_temp_f, 0.0],
            }),
        );
        let mode_f = match settings.mode {
            OpticMode::Eye => 0.0,
            OpticMode::Nv => 1.0,
            OpticMode::Thermal => 2.0,
        };
        gpu.queue.write_buffer(
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

        let palette_view = self.palette_tex(gpu, settings.palette);

        let mk_inst_buf = |v: &Vec<Instance>| {
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: if v.is_empty() {
                        &[0u8; std::mem::size_of::<Instance>()]
                    } else {
                        bytemuck::cast_slice(v)
                    },
                    usage: wgpu::BufferUsages::VERTEX,
                })
        };
        let bufs = [
            (mk_inst_buf(&boxes), boxes.len() as u32, &self.box_mesh),
            (mk_inst_buf(&cyls), cyls.len() as u32, &self.cyl_mesh),
            (mk_inst_buf(&spheres), spheres.len() as u32, &self.sphere_mesh),
            (mk_inst_buf(&grounds), grounds.len() as u32, &self.ground_mesh),
        ];

        let color_view = self.color_tex.create_view(&Default::default());
        let temp_view = self.temp_tex.create_view(&Default::default());
        let depth_view = self.depth_tex.create_view(&Default::default());
        let out_view = self.out_tex.create_view(&Default::default());

        let optic_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
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
            ],
        });

        let mut enc = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("geom"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &color_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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
        }
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("optic"),
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
            pass.set_pipeline(&self.optic_pipeline);
            pass.set_bind_group(0, &optic_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit([enc.finish()]);
    }

    /// Read back the last rendered frame as tightly-packed RGBA8.
    pub fn read_rgba(&self, gpu: &crate::gpu::Gpu) -> Vec<u8> {
        let bytes_per_row_unpadded = self.width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let bytes_per_row = bytes_per_row_unpadded.div_ceil(align) * align;
        let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (bytes_per_row * self.height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
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
        gpu.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| {
            r.expect("readback map failed");
        });
        gpu.device.poll(wgpu::Maintain::Wait);
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
