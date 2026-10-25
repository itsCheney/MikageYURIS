//! GPU compositor: draws the layer tree with wgpu, Metal on Apple devices.
//!
//! It draws what the CPU compositor (`yuris_host::compose`) draws and stays
//! pixel-compatible with it, give or take rounding: the CPU compositor is
//! the reference, and the tests compare the two.
//!
//! A frame is the main window's set (`:/W0`) drawn over black into a
//! screen-sized texture, which is then scaled to fit the output, with black
//! bars. Each image layer is a texture, uploaded once and kept while its
//! image lives. Layers are quads in Z order; a set with an opacity or a
//! mode is drawn into a texture of its own first, as the CPU compositor
//! composes it into a canvas. Normal, add, subtract and screen are blend
//! states; multiply, lighten, darken and difference read a copy of the
//! destination in the shader. A layer or set in a rule transition (TID)
//! samples its rule image in the shader too.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use yuris_format::vfs::RgbaImage;
use yuris_host::layers::{LayerId, LayerTree, Transition};

/// The format of the frame and of composed sets.
const CANVAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Frames a texture may go unused before it is dropped.
const KEEP_FRAMES: u64 = 180;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
    duv: [f32; 2],
    k: f32,
    premul: f32,
    mode: f32,
    ruv: [f32; 2],
    rule: [f32; 2],
}

/// A clip rectangle in target pixels: [x0, x1) × [y0, y1).
#[derive(Debug, Clone, Copy)]
struct Clip {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

impl Clip {
    fn intersect(self, o: Clip) -> Clip {
        Clip { x0: self.x0.max(o.x0), y0: self.y0.max(o.y0), x1: self.x1.min(o.x1), y1: self.y1.min(o.y1) }
    }

    fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }
}

/// What a quad samples.
#[derive(Debug, Clone, Copy)]
enum Source {
    /// A cached image (key into `images`).
    Image(usize),
    /// A composed set (index into `targets`).
    Target(usize),
}

#[derive(Debug)]
struct Draw {
    source: Source,
    /// First of the quad's six vertices.
    first: u32,
    mode: i64,
    scissor: Clip,
    /// The rule image of a transition (key into `images`).
    rule: Option<usize>,
}

/// One render target's draws.
#[derive(Debug)]
struct Pass {
    /// Index into `targets`, or None for the frame.
    target: Option<usize>,
    width: u32,
    height: u32,
    /// Where the target's origin is on screen.
    origin: (i64, i64),
    draws: Vec<Draw>,
}

struct CachedImage {
    image: Weak<RgbaImage>,
    _texture: wgpu::Texture,
    bind: wgpu::BindGroup,
    /// Bound as a rule image.
    rule_bind: wgpu::BindGroup,
    used: u64,
}

/// An offscreen texture: the frame, a composed set, or a copy of one.
struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Bound as a quad's source.
    bind: wgpu::BindGroup,
    /// Bound as the destination copy of `fs_dst`.
    dst_bind: wgpu::BindGroup,
    width: u32,
    height: u32,
    busy: bool,
    used: u64,
}

/// The GPU compositor. Create it with the device the output belongs to;
/// everything runs on the caller's thread.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    out_format: wgpu::TextureFormat,
    max_texture: u32,
    src_layout: wgpu::BindGroupLayout,
    dst_layout: wgpu::BindGroupLayout,
    nearest: wgpu::Sampler,
    linear: wgpu::Sampler,
    /// A 1x1 texture bound where no destination copy or rule is read.
    _blank: wgpu::Texture,
    blank_bind: wgpu::BindGroup,
    pipe_normal: wgpu::RenderPipeline,
    pipe_add: wgpu::RenderPipeline,
    pipe_subtract: wgpu::RenderPipeline,
    pipe_screen: wgpu::RenderPipeline,
    pipe_dst: wgpu::RenderPipeline,
    pipe_present: wgpu::RenderPipeline,
    images: HashMap<usize, CachedImage>,
    targets: Vec<Target>,
    frame: Option<Target>,
    vertices: Vec<Vertex>,
    vbuf: wgpu::Buffer,
    frame_no: u64,
    /// Images too large for one texture, noted once each.
    skipped: std::collections::HashSet<(u32, u32)>,
}

fn blend(color: wgpu::BlendComponent) -> wgpu::BlendState {
    wgpu::BlendState { color, alpha: wgpu::BlendComponent::OVER }
}

impl Renderer {
    /// A renderer drawing to outputs of `out_format`.
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, out_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yuris compose"),
            source: wgpu::ShaderSource::Wgsl(include_str!("compose.wgsl").into()),
        });
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let src_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yuris source"),
            entries: &[
                tex_entry(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let dst_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yuris destination"),
            entries: &[tex_entry(0)],
        });
        // Source, destination copy, rule.
        let simple = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("yuris quads"),
            bind_group_layouts: &[&src_layout, &dst_layout, &dst_layout],
            push_constant_ranges: &[],
        });
        let with_dst = &simple;
        let attrs = wgpu::vertex_attr_array![
            0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32, 4 => Float32, 5 => Float32,
            6 => Float32x2, 7 => Float32x2
        ];
        let pipeline = |layout: &wgpu::PipelineLayout, fs: &str, format, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attrs,
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
        };
        use wgpu::{BlendComponent as C, BlendFactor as F, BlendOperation as O};
        let pipe_normal = pipeline(&simple, "fs", CANVAS_FORMAT, Some(blend(C::OVER)));
        let pipe_add = pipeline(
            &simple,
            "fs",
            CANVAS_FORMAT,
            Some(blend(C { src_factor: F::One, dst_factor: F::One, operation: O::Add })),
        );
        let pipe_subtract = pipeline(
            &simple,
            "fs",
            CANVAS_FORMAT,
            Some(blend(C { src_factor: F::One, dst_factor: F::One, operation: O::ReverseSubtract })),
        );
        let pipe_screen = pipeline(
            &simple,
            "fs",
            CANVAS_FORMAT,
            Some(blend(C { src_factor: F::One, dst_factor: F::OneMinusSrc, operation: O::Add })),
        );
        let pipe_dst = pipeline(with_dst, "fs_dst", CANVAS_FORMAT, None);
        let pipe_present = pipeline(&simple, "fs", out_format, None);
        let sampler = |filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("yuris sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        let nearest = sampler(wgpu::FilterMode::Nearest);
        let linear = sampler(wgpu::FilterMode::Linear);
        let vbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yuris quads"),
            size: 64 * 1024,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let max_texture = device.limits().max_texture_dimension_2d;
        let blank = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yuris blank"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CANVAS_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            blank.as_image_copy(),
            &[255; 4],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        let blank_view = blank.create_view(&Default::default());
        let blank_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("yuris blank"),
            layout: &dst_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&blank_view) }],
        });
        Self {
            device,
            queue,
            out_format,
            max_texture,
            src_layout,
            dst_layout,
            nearest,
            linear,
            _blank: blank,
            blank_bind,
            pipe_normal,
            pipe_add,
            pipe_subtract,
            pipe_screen,
            pipe_dst,
            pipe_present,
            images: HashMap::new(),
            targets: Vec::new(),
            frame: None,
            vertices: Vec::new(),
            vbuf,
            frame_no: 0,
            skipped: Default::default(),
        }
    }

    /// A renderer on a device of its own, drawing to `Rgba8Unorm` outputs:
    /// for tests and tools ([`Self::render_image`]). None without a GPU.
    pub fn headless() -> Option<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("yuris headless"),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .ok()?;
        Some(Self::new(device, queue, CANVAS_FORMAT))
    }

    /// The device.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The output format given at creation.
    pub fn out_format(&self) -> wgpu::TextureFormat {
        self.out_format
    }

    /// Number of image textures held.
    pub fn cached_images(&self) -> usize {
        self.images.len()
    }

    /// Draws window 0 of `tree` (a `screen`-sized frame) into `out`, an
    /// `out_size` texture of the output format, scaled to fit with black
    /// bars. Returns the frame's rectangle in `out` (x, y, w, h), for
    /// mapping touches back to the screen.
    pub fn render(
        &mut self,
        tree: &LayerTree,
        screen: (u32, u32),
        out: &wgpu::TextureView,
        out_size: (u32, u32),
    ) -> [f32; 4] {
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("yuris frame") });
        self.encode_frame(&mut encoder, tree, screen);
        let rect = fit(screen, out_size);
        let first = self.vertices.len() as u32;
        quad(&mut self.vertices, out_size, rect, [0.0, 0.0, 1.0, 1.0], 1.0, true, 0, None);
        self.upload_vertices();
        let frame = self.frame.as_ref().expect("frame target");
        let present_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("yuris present"),
            layout: &self.src_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&frame.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.linear) },
            ],
        });
        {
            let mut rp = begin(&mut encoder, out, wgpu::LoadOp::Clear(wgpu::Color::BLACK));
            rp.set_pipeline(&self.pipe_present);
            rp.set_vertex_buffer(0, self.vbuf.slice(..));
            rp.set_bind_group(0, &present_bind, &[]);
            rp.set_bind_group(1, &self.blank_bind, &[]);
            rp.set_bind_group(2, &self.blank_bind, &[]);
            rp.draw(first..first + 6, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        self.end_frame();
        rect
    }

    /// Draws window 0 of `tree` and reads the frame back, straight alpha
    /// like `yuris_host::compose::render_window`. Waits for the GPU.
    pub fn render_image(&mut self, tree: &LayerTree, screen: (u32, u32)) -> RgbaImage {
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("yuris frame") });
        self.encode_frame(&mut encoder, tree, screen);
        self.upload_vertices();
        let (w, h) = (screen.0.max(1), screen.1.max(1));
        let row = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yuris readback"),
            size: u64::from(row) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let frame = self.frame.as_ref().expect("frame target");
        encoder.copy_texture_to_buffer(
            frame.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::Wait);
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        {
            let mapped = slice.get_mapped_range();
            for y in 0..h as usize {
                data.extend_from_slice(&mapped[y * row as usize..y * row as usize + w as usize * 4]);
            }
        }
        buf.unmap();
        self.end_frame();
        // The frame is opaque, so premultiplied and straight are the same.
        RgbaImage { width: w, height: h, data }
    }

    /// Builds and encodes the frame's passes into the frame target.
    fn encode_frame(&mut self, encoder: &mut wgpu::CommandEncoder, tree: &LayerTree, screen: (u32, u32)) {
        self.frame_no += 1;
        self.vertices.clear();
        let (w, h) = (screen.0.max(1), screen.1.max(1));
        if self.frame.as_ref().is_none_or(|f| f.width != w || f.height != h) {
            self.frame = Some(self.make_target(w, h));
        }
        let mut passes = vec![Pass { target: None, width: w, height: h, origin: (0, 0), draws: Vec::new() }];
        if let Some(set) = tree.window_set(0) {
            let (x, y) = tree.get(set).map_or((0, 0), |l| (l.x, l.y));
            let clip = Clip { x0: 0, y0: 0, x1: i64::from(w), y1: i64::from(h) };
            self.walk(tree, set, 0, x, y, clip, &mut passes);
        }
        // A set's pass comes after the pass that draws it, and its own
        // sets' passes after it: drawn back to front, each is ready when
        // it is needed.
        for pass in passes.iter().rev() {
            self.encode_pass(encoder, pass);
        }
    }

    /// Adds the children of `set` to pass `p`, with the set's origin at
    /// (ox, oy) in the pass's target. Mirrors `draw_children` of the CPU
    /// compositor.
    #[allow(clippy::too_many_arguments)]
    fn walk(&mut self, tree: &LayerTree, set: LayerId, p: usize, ox: i64, oy: i64, clip: Clip, passes: &mut Vec<Pass>) {
        for id in tree.draw_order(set) {
            let Some(l) = tree.get(id) else { continue };
            if !l.enabled || l.alpha <= 0 {
                continue;
            }
            let (x, y) = (ox + l.x, oy + l.y);
            let tr = tree.transition(l);
            let rule = tr.as_ref().and_then(|t| Some((self.image(&t.rule)?, t)));
            let at = passes[p].origin;
            if !l.is_set {
                let (iw, ih) = (l.image.width, l.image.height);
                if iw == 0 || ih == 0 {
                    continue;
                }
                let Some(key) = self.image(&l.image) else { continue };
                let size = (passes[p].width, passes[p].height);
                let first = self.vertices.len() as u32;
                quad(
                    &mut self.vertices,
                    size,
                    [x as f32, y as f32, iw as f32, ih as f32],
                    [0.0, 0.0, 1.0, 1.0],
                    k(l.alpha),
                    false,
                    l.mode,
                    rule.map(|(_, t)| (t, at)),
                );
                passes[p].draws.push(Draw {
                    source: Source::Image(key),
                    first,
                    mode: l.mode,
                    scissor: clip,
                    rule: rule.map(|(r, _)| r),
                });
                continue;
            }
            let set_clip = if l.sx > 0 && l.sy > 0 {
                clip.intersect(Clip { x0: x, y0: y, x1: x + l.sx, y1: y + l.sy })
            } else {
                clip
            };
            if l.alpha >= 256 && l.mode == 0 && rule.is_none() {
                self.walk(tree, id, p, x, y, set_clip, passes);
                continue;
            }
            let (w, h) = if l.sx > 0 && l.sy > 0 {
                (l.sx as u32, l.sy as u32)
            } else {
                (passes[p].width, passes[p].height)
            };
            if w > self.max_texture || h > self.max_texture || set_clip.is_empty() {
                continue;
            }
            let (bx, by) = if l.sx > 0 && l.sy > 0 { (x, y) } else { (0, 0) };
            let t = self.take_target(w, h);
            let child = passes.len();
            passes.push(Pass { target: Some(t), width: w, height: h, origin: (at.0 + bx, at.1 + by), draws: Vec::new() });
            let full = Clip { x0: 0, y0: 0, x1: i64::from(w), y1: i64::from(h) };
            self.walk(tree, id, child, x - bx, y - by, full, passes);
            let size = (passes[p].width, passes[p].height);
            let first = self.vertices.len() as u32;
            quad(
                &mut self.vertices,
                size,
                [bx as f32, by as f32, w as f32, h as f32],
                [0.0, 0.0, 1.0, 1.0],
                k(l.alpha),
                true,
                l.mode,
                rule.map(|(_, t)| (t, at)),
            );
            passes[p].draws.push(Draw {
                source: Source::Target(t),
                first,
                mode: l.mode,
                scissor: set_clip,
                rule: rule.map(|(r, _)| r),
            });
        }
    }

    fn encode_pass(&mut self, encoder: &mut wgpu::CommandEncoder, pass: &Pass) {
        let clear = if pass.target.is_none() { wgpu::Color::BLACK } else { wgpu::Color::TRANSPARENT };
        let bounds = Clip { x0: 0, y0: 0, x1: i64::from(pass.width), y1: i64::from(pass.height) };
        let mut load = wgpu::LoadOp::Clear(clear);
        let mut i = 0;
        loop {
            // Draws up to the next one that reads the destination.
            let end = pass.draws[i..].iter().position(|d| reads_dst(d.mode)).map_or(pass.draws.len(), |n| i + n);
            {
                let target = self.pass_target(pass.target);
                let mut rp = begin(encoder, &target.view, load);
                rp.set_vertex_buffer(0, self.vbuf.slice(..));
                for d in &pass.draws[i..end] {
                    let Some(s) = scissor(d.scissor.intersect(bounds)) else { continue };
                    rp.set_pipeline(match d.mode {
                        1 => &self.pipe_add,
                        2 => &self.pipe_subtract,
                        5 => &self.pipe_screen,
                        _ => &self.pipe_normal,
                    });
                    rp.set_bind_group(0, self.source_bind(d.source), &[]);
                    rp.set_bind_group(1, &self.blank_bind, &[]);
                    rp.set_bind_group(2, self.rule_bind(d.rule), &[]);
                    rp.set_scissor_rect(s[0], s[1], s[2], s[3]);
                    rp.draw(d.first..d.first + 6, 0..1);
                }
            }
            load = wgpu::LoadOp::Load;
            if end == pass.draws.len() {
                break;
            }
            let d = &pass.draws[end];
            i = end + 1;
            let Some(s) = scissor(d.scissor.intersect(bounds)) else { continue };
            // Copy the target, then draw the quad reading the copy.
            let copy = self.take_target(pass.width, pass.height);
            {
                let src = self.pass_target(pass.target);
                encoder.copy_texture_to_texture(
                    src.texture.as_image_copy(),
                    self.targets[copy].texture.as_image_copy(),
                    wgpu::Extent3d { width: pass.width, height: pass.height, depth_or_array_layers: 1 },
                );
            }
            let target = self.pass_target(pass.target);
            let mut rp = begin(encoder, &target.view, wgpu::LoadOp::Load);
            rp.set_vertex_buffer(0, self.vbuf.slice(..));
            rp.set_pipeline(&self.pipe_dst);
            rp.set_bind_group(0, self.source_bind(d.source), &[]);
            rp.set_bind_group(1, &self.targets[copy].dst_bind, &[]);
            rp.set_bind_group(2, self.rule_bind(d.rule), &[]);
            rp.set_scissor_rect(s[0], s[1], s[2], s[3]);
            rp.draw(d.first..d.first + 6, 0..1);
        }
    }

    fn pass_target(&self, t: Option<usize>) -> &Target {
        match t {
            Some(i) => &self.targets[i],
            None => self.frame.as_ref().expect("frame target"),
        }
    }

    fn rule_bind(&self, rule: Option<usize>) -> &wgpu::BindGroup {
        rule.and_then(|key| self.images.get(&key)).map_or(&self.blank_bind, |c| &c.rule_bind)
    }

    fn source_bind(&self, s: Source) -> &wgpu::BindGroup {
        match s {
            Source::Image(key) => &self.images[&key].bind,
            Source::Target(i) => &self.targets[i].bind,
        }
    }

    /// The cache key of an image's texture, uploading it on first use.
    /// The cache holds a weak reference, so the image's allocation (and
    /// with it the key) stays unique while the entry lives; an image
    /// changed in place through `Arc::make_mut` moves to a new allocation
    /// because of that reference, and is uploaded again.
    fn image(&mut self, img: &Arc<RgbaImage>) -> Option<usize> {
        let key = Arc::as_ptr(img) as usize;
        if let Some(c) = self.images.get_mut(&key) {
            if c.image.strong_count() > 0 {
                c.used = self.frame_no;
                return Some(key);
            }
            self.images.remove(&key);
        }
        let (w, h) = (img.width, img.height);
        if w > self.max_texture || h > self.max_texture || img.data.len() < (w * h * 4) as usize {
            if self.skipped.insert((w, h)) {
                tracing::warn!("image {w}x{h} is larger than the GPU's {} limit; not drawn", self.max_texture);
            }
            return None;
        }
        let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yuris image"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CANVAS_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            texture.as_image_copy(),
            &img.data[..(w * h * 4) as usize],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
            size,
        );
        let view = texture.create_view(&Default::default());
        let bind = self.source_bind_group(&view);
        let rule_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("yuris rule"),
            layout: &self.dst_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
        });
        self.images
            .insert(key, CachedImage { image: Arc::downgrade(img), _texture: texture, bind, rule_bind, used: self.frame_no });
        Some(key)
    }

    fn source_bind_group(&self, view: &wgpu::TextureView) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("yuris source"),
            layout: &self.src_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.nearest) },
            ],
        })
    }

    fn make_target(&self, width: u32, height: u32) -> Target {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yuris canvas"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CANVAS_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let bind = self.source_bind_group(&view);
        let dst_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("yuris destination"),
            layout: &self.dst_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
        });
        Target { texture, view, bind, dst_bind, width, height, busy: false, used: self.frame_no }
    }

    /// A free target of this size for this frame.
    fn take_target(&mut self, width: u32, height: u32) -> usize {
        let i = match self.targets.iter().position(|t| !t.busy && t.width == width && t.height == height) {
            Some(i) => i,
            None => {
                let t = self.make_target(width, height);
                self.targets.push(t);
                self.targets.len() - 1
            }
        };
        self.targets[i].busy = true;
        self.targets[i].used = self.frame_no;
        i
    }

    fn upload_vertices(&mut self) {
        let bytes: &[u8] = bytemuck::cast_slice(&self.vertices);
        if bytes.len() as u64 > self.vbuf.size() {
            self.vbuf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("yuris quads"),
                size: (bytes.len() as u64).next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !bytes.is_empty() {
            self.queue.write_buffer(&self.vbuf, 0, bytes);
        }
    }

    /// Frees the frame's targets and drops textures not drawn for a while
    /// (or whose images are gone).
    fn end_frame(&mut self) {
        let now = self.frame_no;
        for t in &mut self.targets {
            t.busy = false;
        }
        // Targets are only reused at the same position in the list, so
        // keep the order stable: drop stale ones from the end only.
        while self.targets.last().is_some_and(|t| now - t.used > KEEP_FRAMES) {
            self.targets.pop();
        }
        self.images.retain(|_, c| c.image.strong_count() > 0 && now - c.used <= KEEP_FRAMES);
    }
}

/// Opacity 0..=256 as a factor, as the CPU compositor applies it.
fn k(alpha: i64) -> f32 {
    alpha.clamp(0, 256) as f32 / 256.0
}

fn reads_dst(mode: i64) -> bool {
    matches!(mode, 3 | 6 | 7 | 8)
}

/// Two triangles for the rectangle `r` (x, y, w, h, in pixels of a
/// `size` target), sampling `uv` (u0, v0, u1, v1). The destination
/// coordinates are the same rectangle in a copy of the target. A rule
/// transition comes with the screen position of the target's origin.
#[allow(clippy::too_many_arguments)]
fn quad(
    out: &mut Vec<Vertex>,
    size: (u32, u32),
    r: [f32; 4],
    uv: [f32; 4],
    k: f32,
    premul: bool,
    mode: i64,
    rule: Option<(&Transition, (i64, i64))>,
) {
    let (tw, th) = (size.0 as f32, size.1 as f32);
    let corner = |px: f32, py: f32, u: f32, v: f32| {
        let (ruv, params) = match rule {
            Some((t, at)) => (
                [
                    (px + (at.0 - t.x) as f32) / t.rule.width as f32,
                    (py + (at.1 - t.y) as f32) / t.rule.height as f32,
                ],
                [t.t, t.vague],
            ),
            None => ([0.0; 2], [0.0; 2]),
        };
        Vertex {
            pos: [px / tw * 2.0 - 1.0, 1.0 - py / th * 2.0],
            uv: [u, v],
            duv: [px / tw, py / th],
            k,
            premul: if premul { 1.0 } else { 0.0 },
            mode: mode as f32,
            ruv,
            rule: params,
        }
    };
    let (x0, y0, x1, y1) = (r[0], r[1], r[0] + r[2], r[1] + r[3]);
    let a = corner(x0, y0, uv[0], uv[1]);
    let b = corner(x1, y0, uv[2], uv[1]);
    let c = corner(x0, y1, uv[0], uv[3]);
    let d = corner(x1, y1, uv[2], uv[3]);
    out.extend_from_slice(&[a, b, c, c, b, d]);
}

fn scissor(c: Clip) -> Option<[u32; 4]> {
    if c.is_empty() {
        return None;
    }
    Some([c.x0 as u32, c.y0 as u32, (c.x1 - c.x0) as u32, (c.y1 - c.y0) as u32])
}

/// The largest rectangle with the screen's aspect that fits `out`,
/// centred: (x, y, w, h).
pub fn fit(screen: (u32, u32), out: (u32, u32)) -> [f32; 4] {
    let (sw, sh) = (screen.0.max(1) as f32, screen.1.max(1) as f32);
    let (ow, oh) = (out.0.max(1) as f32, out.1.max(1) as f32);
    let scale = (ow / sw).min(oh / sh);
    let (w, h) = ((sw * scale).round(), (sh * scale).round());
    [((ow - w) / 2.0).floor(), ((oh - h) / 2.0).floor(), w, h]
}

fn begin<'a>(encoder: &'a mut wgpu::CommandEncoder, view: &'a wgpu::TextureView, load: wgpu::LoadOp<wgpu::Color>) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("yuris pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    })
}
