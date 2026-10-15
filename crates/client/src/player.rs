//! The player model (Steve), built on the CPU every frame from its pose and
//! drawn with the skin texture.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use rscraft_assets::RgbaImage;
use rscraft_world::Face;

use crate::mesh::corners;

const SKIN_SIZE: f32 = 64.0;
/// Model pixels to blocks; the player model is drawn at 15/16 scale like vanilla.
const PIXEL: f32 = 0.9375 / 16.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct EntityVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub normal: [f32; 3],
    /// Sky light level at the entity, 0..1.
    pub light: f32,
}

/// Everything needed to pose the model.
pub struct PlayerPose {
    pub feet: Vec3,
    pub yaw: f32,
    pub head_pitch: f32,
    /// Limb swing angle in radians (positive = right arm forward).
    pub swing: f32,
    /// Sky light level at the player, 0..1.
    pub light: f32,
    /// 0..1 blend into the horizontal swimming pose.
    pub swim: f32,
    /// Walk/swim cycle phase in radians, drives the swimming strokes.
    pub limb_phase: f32,
}

/// A box of the model in model pixels (Y up from the feet, front towards -Z,
/// the player's right towards +X).
struct Part {
    min: [f32; 3],
    size: [f32; 3],
    /// Skin coordinates of the base layer and the outer layer.
    uv: [f32; 2],
    overlay_uv: [f32; 2],
    overlay_inflate: f32,
    pivot: [f32; 3],
}

const HEAD: Part = Part { min: [-4., 24., -4.], size: [8., 8., 8.], uv: [0., 0.], overlay_uv: [32., 0.], overlay_inflate: 0.5, pivot: [0., 24., 0.] };
const BODY: Part = Part { min: [-4., 12., -2.], size: [8., 12., 4.], uv: [16., 16.], overlay_uv: [16., 32.], overlay_inflate: 0.25, pivot: [0., 24., 0.] };
const RIGHT_ARM: Part = Part { min: [4., 12., -2.], size: [4., 12., 4.], uv: [40., 16.], overlay_uv: [40., 32.], overlay_inflate: 0.25, pivot: [5., 22., 0.] };
const LEFT_ARM: Part = Part { min: [-8., 12., -2.], size: [4., 12., 4.], uv: [32., 48.], overlay_uv: [48., 48.], overlay_inflate: 0.25, pivot: [-5., 22., 0.] };
const RIGHT_LEG: Part = Part { min: [0., 0., -2.], size: [4., 12., 4.], uv: [0., 16.], overlay_uv: [0., 32.], overlay_inflate: 0.25, pivot: [2., 12., 0.] };
const LEFT_LEG: Part = Part { min: [-4., 0., -2.], size: [4., 12., 4.], uv: [16., 48.], overlay_uv: [0., 48.], overlay_inflate: 0.25, pivot: [-2., 12., 0.] };

/// Skin region of a box face, following Minecraft's box UV layout:
/// `[right][front][left][back]` below `[top][bottom]`.
fn face_region(face: Face, [u, v]: [f32; 2], [w, h, d]: [f32; 3]) -> [f32; 4] {
    match face {
        Face::Up => [u + d, v, w, d],
        Face::Down => [u + d + w, v, w, d],
        Face::East => [u, v + d, d, h],
        Face::North => [u + d, v + d, w, h],
        Face::West => [u + d + w, v + d, d, h],
        Face::South => [u + 2.0 * d + w, v + d, w, h],
    }
}

fn push_box(out: &mut Vec<EntityVertex>, part: &Part, uv: [f32; 2], inflate: f32, transform: Mat4, light: f32) {
    const UVS: [[f32; 2]; 4] = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
    let min = Vec3::from(part.min) - inflate;
    let size = Vec3::from(part.size) + inflate * 2.0;
    for face in Face::ALL {
        let [ru, rv, rw, rh] = face_region(face, uv, part.size);
        let normal = transform.transform_vector3(Vec3::from(face.offset().map(|c| c as f32))).normalize();
        let quad: [EntityVertex; 4] = std::array::from_fn(|i| {
            let corner = Vec3::from(corners(face)[i]);
            let [mut cu, mut cv] = UVS[i];
            // Top and bottom are laid out differently from block faces.
            match face {
                Face::Up => (cu, cv) = (1.0 - cu, 1.0 - cv),
                Face::Down => cu = 1.0 - cu,
                _ => {}
            }
            EntityVertex {
                position: transform.transform_point3(min + corner * size).to_array(),
                uv: [(ru + cu * rw) / SKIN_SIZE, (rv + cv * rh) / SKIN_SIZE],
                normal: normal.to_array(),
                light,
            }
        });
        out.extend_from_slice(&[quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]]);
    }
}

/// The right arm as seen in first person, in model pixels: the shoulder is
/// at the origin and the hand points along -Y.
const FIRST_PERSON_ARM: Part = Part { min: [-2., -12., -2.], size: [4., 12., 4.], uv: [40., 16.], overlay_uv: [40., 32.], overlay_inflate: 0.25, pivot: [0., 0., 0.] };

/// What the first-person arm is doing.
pub struct HandPose {
    pub eye: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    /// Swing progress 0..1 (0 = not swinging).
    pub swing: f32,
    /// How far the hand is raised when switching items: 1 = in place, 0 = hidden.
    pub equip: f32,
    /// View bobbing offset in view space.
    pub bob: Vec3,
    /// Sky light level at the player, 0..1.
    pub light: f32,
    /// Width / height of the view.
    pub aspect: f32,
    /// Vertical FOV in radians without the zoom; the hand is drawn as if it
    /// were 70°, so it zooms together with the world but ignores sprinting
    /// and the FOV setting.
    pub fov_y: f32,
}

/// Strength of the arm swing animation; 1.0 is the original, bigger swing.
const SWING_AMOUNT: f32 = 1.0;

/// Minecraft always draws the hand with this FOV, whatever the zoom or setting.
const HAND_FOV: f32 = 70.0 * std::f32::consts::PI / 180.0;

/// Lowers the hand while switching items, like vanilla (0.6 below when hidden).
fn equip_offset(pose: &HandPose) -> Vec3 {
    Vec3::new(0.0, -0.6 * (1.0 - pose.equip), 0.0)
}

/// Camera-space to world-space for first-person objects, with the FOV
/// correction that keeps them the same size when sprinting.
fn first_person_view(pose: &HandPose) -> Mat4 {
    let view = Mat4::from_translation(pose.eye) * Mat4::from_rotation_y(-pose.yaw) * Mat4::from_rotation_x(pose.pitch);
    let k = (pose.fov_y / 2.0).tan() / (HAND_FOV / 2.0).tan();
    view * Mat4::from_scale(Vec3::new(k, k, 1.0))
}

/// Where a block held in the right hand is drawn in first person, as the
/// transform of the unit cube 0..1. Follows vanilla's ItemInHandRenderer:
/// placed at the bottom right, turned 45°, scaled to 0.4, and swung by the
/// same attack animation as items.
pub fn held_block_first_person(pose: &HandPose) -> Mat4 {
    use std::f32::consts::{PI, TAU};
    let s = pose.swing;
    let (sqrt_s, deg) = (s.sqrt(), |d: f32| d.to_radians());
    let push = Vec3::new(
        -0.4 * (sqrt_s * PI).sin(),
        0.2 * (sqrt_s * TAU).sin(),
        -0.2 * (s * PI).sin(),
    ) * SWING_AMOUNT;
    let twist = (s * s * PI).sin() * SWING_AMOUNT;
    let tilt = (sqrt_s * PI).sin() * SWING_AMOUNT;
    // Vanilla's offsets assume a 16:9 window; in narrower ones the block
    // would slide off the right edge, so keep its place on screen instead.
    let x = 0.56 * (pose.aspect / (16.0 / 9.0)).min(1.0);
    let item = Mat4::from_translation(Vec3::new(x, -0.52, -0.72) + push + pose.bob + equip_offset(pose))
        * Mat4::from_rotation_y(deg(45.0 - 20.0 * twist))
        * Mat4::from_rotation_z(deg(-20.0 * tilt))
        * Mat4::from_rotation_x(deg(-80.0 * tilt))
        * Mat4::from_rotation_y(deg(-45.0))
        // The block model's own first-person display transform.
        * Mat4::from_rotation_y(deg(45.0))
        * Mat4::from_scale(Vec3::splat(0.4))
        * Mat4::from_translation(Vec3::splat(-0.5));
    first_person_view(pose) * item
}

/// Where a block held in the right hand is drawn in third person, as the
/// transform of the unit cube 0..1.
pub fn held_block_third_person(pose: &PlayerPose) -> Mat4 {
    // At the bottom of the right arm, a little in front of it, 6 pixels wide.
    part_transform(pose, &RIGHT_ARM, pose.swing)
        * Mat4::from_translation(Vec3::new(6.0, 11.0, -3.0))
        * Mat4::from_rotation_y(45f32.to_radians())
        * Mat4::from_scale(Vec3::splat(6.0))
        * Mat4::from_translation(Vec3::splat(-0.5))
}

/// Model-pixel to world transform of the whole body. When swimming the body
/// tips forward to lie along the look direction, centred on the 0.6 block
/// tall swimming hitbox (vanilla's setupRotations).
fn body_transform(pose: &PlayerPose) -> Mat4 {
    let s = pose.swim;
    let tip = (-std::f32::consts::FRAC_PI_2 + pose.head_pitch) * s;
    Mat4::from_translation(pose.feet + Vec3::Y * 0.3 * s)
        * Mat4::from_rotation_y(-pose.yaw)
        * Mat4::from_rotation_x(tip)
        * Mat4::from_translation(Vec3::Y * -0.9 * s)
        * Mat4::from_scale(Vec3::splat(PIXEL))
}

/// Model-pixel to world transform of a body part rotated about its pivot.
fn part_transform(pose: &PlayerPose, part: &Part, angle: f32) -> Mat4 {
    let world = body_transform(pose);
    let pivot = Vec3::from(part.pivot);
    world * Mat4::from_translation(pivot) * Mat4::from_rotation_x(angle) * Mat4::from_translation(-pivot)
}

/// Builds the first-person right arm in world space, placed in front of the
/// camera at the bottom right of the screen.
pub fn build_hand_mesh(pose: &HandPose) -> Vec<EntityVertex> {
    let s = pose.swing;
    let swing_down = (s * std::f32::consts::PI).sin() * SWING_AMOUNT;
    let swing_side = (s.sqrt() * std::f32::consts::PI).sin() * SWING_AMOUNT;
    // View space: X right, Y up, -Z forward.
    let shoulder =
        Vec3::new(0.36 - swing_side * 0.25, -0.66 + swing_down * 0.1, -0.3 - swing_down * 0.15) + pose.bob + equip_offset(pose);
    // Angled up and in towards the centre of the screen, like vanilla.
    let arm = Mat4::from_translation(shoulder)
        * Mat4::from_rotation_y(0.15 + swing_side * 0.4)
        * Mat4::from_rotation_z(-0.35)
        * Mat4::from_rotation_x(2.0 - swing_down * 0.7)
        * Mat4::from_scale(Vec3::splat(1.0 / 16.0));
    let transform = first_person_view(pose) * arm;
    let mut out = Vec::with_capacity(72);
    let arm = &FIRST_PERSON_ARM;
    push_box(&mut out, arm, arm.uv, 0.0, transform, pose.light);
    push_box(&mut out, arm, arm.overlay_uv, arm.overlay_inflate, transform, pose.light);
    out
}

/// Builds the posed model as a triangle list in world space.
pub fn build_mesh(pose: &PlayerPose) -> Vec<EntityVertex> {
    let rotated = |part: &Part, angle: f32| part_transform(pose, part, angle);
    let s = pose.swim;
    let lerp = |a: f32, b: f32| a + (b - a) * s;
    let phase = pose.limb_phase;
    // Swimming: the head looks ahead (45° up from the body), the arms crawl
    // in alternating full strokes and the legs flutter, like vanilla.
    let (stroke_right, stroke_left) = (phase % std::f32::consts::TAU, (phase + std::f32::consts::PI) % std::f32::consts::TAU);
    let kick = 0.3 * (phase * 2.0).cos();
    let parts = [
        (&HEAD, lerp(pose.head_pitch, std::f32::consts::FRAC_PI_4)),
        (&BODY, 0.0),
        (&RIGHT_ARM, lerp(pose.swing, stroke_right)),
        (&LEFT_ARM, lerp(-pose.swing, stroke_left)),
        (&RIGHT_LEG, lerp(-pose.swing, kick)),
        (&LEFT_LEG, lerp(pose.swing, -kick)),
    ];
    let mut out = Vec::with_capacity(parts.len() * 2 * 36);
    for (part, angle) in parts {
        let transform = rotated(part, angle);
        push_box(&mut out, part, part.uv, 0.0, transform, pose.light);
        push_box(&mut out, part, part.overlay_uv, part.overlay_inflate, transform, pose.light);
    }
    out
}

/// GPU side of the player model.
pub struct PlayerRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    body_vertices: u32,
    hand_vertices: u32,
}

impl PlayerRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        skin: &RgbaImage,
        globals_layout: &wgpu::BindGroupLayout,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let size = wgpu::Extent3d { width: skin.width, height: skin.height, depth_or_array_layers: 1 };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("skin"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &skin.pixels,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * skin.width), rows_per_image: None },
            size,
        );
        let view = texture.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("skin sampler"), ..Default::default() });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("skin layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skin bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });

        let shader = crate::renderer::shader_module(device, "entity shader", include_str!("entity.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("entity pipeline layout"),
            bind_group_layouts: &[Some(globals_layout), Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("entity pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<EntityVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Float32],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        // Two layers × six faces × six vertices, for six body parts plus the hand.
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("player vertices"),
            size: (2 * 7 * 36 * size_of::<EntityVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { pipeline, bind_group, vertex_buffer, body_vertices: 0, hand_vertices: 0 }
    }

    /// Uploads this frame's body and first-person hand; call before the
    /// render pass starts.
    pub fn prepare(&mut self, queue: &wgpu::Queue, body: Option<&[EntityVertex]>, hand: Option<&[EntityVertex]>) {
        let (body, hand) = (body.unwrap_or_default(), hand.unwrap_or_default());
        let vertices: Vec<EntityVertex> = body.iter().chain(hand).copied().collect();
        if !vertices.is_empty() {
            queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        }
        self.body_vertices = body.len() as u32;
        self.hand_vertices = hand.len() as u32;
    }

    fn draw_range(&self, pass: &mut wgpu::RenderPass<'_>, range: std::ops::Range<u32>) {
        if range.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(range, 0..1);
    }

    pub fn draw_body(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_range(pass, 0..self.body_vertices);
    }

    pub fn has_hand(&self) -> bool {
        self.hand_vertices > 0
    }

    /// The hand goes in its own pass with a cleared depth buffer, so it is
    /// never hidden by blocks the camera is close to.
    pub fn draw_hand(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_range(pass, self.body_vertices..self.body_vertices + self.hand_vertices);
    }
}
