//! The player model (Steve), built on the CPU every frame from its pose and
//! drawn with the skin texture.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3};
use rscraft_assets::RgbaImage;
use rscraft_world::Face;

use crate::gpu::{self, Depth, PipelineDesc};
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
    /// Body yaw in radians.
    pub yaw: f32,
    /// How far the head is turned from the body, in radians.
    pub head_yaw: f32,
    /// Head pitch in radians, up positive.
    pub head_pitch: f32,
    /// The walking animation (vanilla's WalkAnimationState): position and speed 0..1.
    pub limb_position: f32,
    pub limb_speed: f32,
    /// Ticks the player has existed, for the arms' idle sway.
    pub age: f32,
    /// Attack swing progress 0..1, 0 when not swinging.
    pub attack: f32,
    /// Holding an item in the right hand.
    pub holding: bool,
    pub crouching: bool,
    /// 0..1 blend into the horizontal swimming pose.
    pub swim: f32,
    /// Sky light level at the player, 0..1.
    pub light: f32,
    /// The whole model leaning back about its feet, in radians; the
    /// inventory player leans towards the mouse.
    pub lean: f32,
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

const HEAD: Part = Part {
    min: [-4., 24., -4.],
    size: [8., 8., 8.],
    uv: [0., 0.],
    overlay_uv: [32., 0.],
    overlay_inflate: 0.5,
    pivot: [0., 24., 0.],
};
const BODY: Part = Part {
    min: [-4., 12., -2.],
    size: [8., 12., 4.],
    uv: [16., 16.],
    overlay_uv: [16., 32.],
    overlay_inflate: 0.25,
    pivot: [0., 24., 0.],
};
const RIGHT_ARM: Part = Part {
    min: [4., 12., -2.],
    size: [4., 12., 4.],
    uv: [40., 16.],
    overlay_uv: [40., 32.],
    overlay_inflate: 0.25,
    pivot: [5., 22., 0.],
};
const LEFT_ARM: Part = Part {
    min: [-8., 12., -2.],
    size: [4., 12., 4.],
    uv: [32., 48.],
    overlay_uv: [48., 48.],
    overlay_inflate: 0.25,
    pivot: [-5., 22., 0.],
};
const RIGHT_LEG: Part = Part {
    min: [-0.1, 0., -2.],
    size: [4., 12., 4.],
    uv: [0., 16.],
    overlay_uv: [0., 32.],
    overlay_inflate: 0.25,
    pivot: [1.9, 12., 0.],
};
const LEFT_LEG: Part = Part {
    min: [-3.9, 0., -2.],
    size: [4., 12., 4.],
    uv: [16., 48.],
    overlay_uv: [0., 48.],
    overlay_inflate: 0.25,
    pivot: [-1.9, 12., 0.],
};

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

/// What the first-person hand is doing; see `hand::HandAnimation`.
pub struct HandPose {
    /// The camera's view matrix as rendered, view bobbing included.
    pub view: Mat4,
    /// Vertical FOV in radians without the zoom; the hand is drawn as if it
    /// were 70°, so it zooms together with the world but ignores sprinting
    /// and the FOV setting.
    pub fov_y: f32,
    /// Attack swing progress 0..1.
    pub swing: f32,
    /// How far the hand is lowered: 0 in place, 1 hidden.
    pub lowered: f32,
    /// How far the arm lags behind the view, in degrees (pitch down, yaw).
    pub sway: Vec2,
    /// View bobbing: walk distance and strength.
    pub walk: f32,
    pub bob: f32,
    /// Sky light level at the player, 0..1.
    pub light: f32,
    /// Width / height of the view.
    pub aspect: f32,
}

/// Minecraft always draws the hand with this FOV, whatever the zoom or setting.
const HAND_FOV: f32 = 70.0 * std::f32::consts::PI / 180.0;

fn degrees(d: f32) -> f32 {
    d.to_radians()
}

/// View space (X right, Y up, -Z forward) of first-person objects, mapped
/// to world space. Like vanilla's item-in-hand pose stack: view bobbing
/// (GameRenderer.bobView) and the arm's lag when turning, drawn with a 70°
/// projection whatever the FOV.
fn hand_space(pose: &HandPose) -> Mat4 {
    use std::f32::consts::PI;
    let k = (pose.fov_y / 2.0).tan() / (HAND_FOV / 2.0).tan();
    // Vanilla bobs by the walk distance counted backwards.
    let (w, bob) = (-pose.walk * PI, pose.bob);
    let bob_view = Mat4::from_translation(Vec3::new(w.sin() * bob * 0.5, -(w.cos() * bob).abs(), 0.0))
        * Mat4::from_rotation_z(degrees(w.sin() * bob * 3.0))
        * Mat4::from_rotation_x(degrees((w - 0.2).cos().abs() * bob * 5.0));
    let sway = Mat4::from_rotation_x(degrees(pose.sway.x)) * Mat4::from_rotation_y(degrees(pose.sway.y));
    // Vanilla's offsets are made for 16:9 and push the hand off the right
    // edge of narrower windows; shift it left to where 16:9 shows it.
    let narrow = (pose.aspect / (16.0 / 9.0)).min(1.0);
    let shift = Mat4::from_translation(Vec3::new(0.6 * (narrow - 1.0), 0.0, 0.0));
    pose.view.inverse() * Mat4::from_scale(Vec3::new(k, k, 1.0)) * shift * bob_view * sway
}

/// Where a block held in the right hand is drawn in first person, as the
/// transform of the unit cube 0..1: vanilla's item arm placement and swing
/// (applyItemArmTransform, swingArm), then the block model's own
/// first-person display transform (turned 45°, scaled to 0.4).
pub fn held_block_first_person(pose: &HandPose) -> Mat4 {
    use std::f32::consts::{PI, TAU};
    let a = pose.swing;
    let sa = a.sqrt();
    let item = Mat4::from_translation(Vec3::new(0.56, -0.52 - 0.6 * pose.lowered, -0.72))
        * Mat4::from_translation(Vec3::new(-0.4 * (sa * PI).sin(), 0.2 * (sa * TAU).sin(), -0.2 * (a * PI).sin()))
        * Mat4::from_rotation_y(degrees(45.0 - 20.0 * (a * a * PI).sin()))
        * Mat4::from_rotation_z(degrees(-20.0 * (sa * PI).sin()))
        * Mat4::from_rotation_x(degrees(-80.0 * (sa * PI).sin()))
        * Mat4::from_rotation_y(degrees(-45.0))
        * Mat4::from_rotation_y(degrees(45.0))
        * Mat4::from_scale(Vec3::splat(0.4))
        * Mat4::from_translation(Vec3::splat(-0.5));
    hand_space(pose) * item
}

/// Where a block held in the right hand is drawn in third person, as the
/// transform of the unit cube 0..1: at the end of the arm however it is
/// posed (ItemInHandLayer), then the block model's third-person display
/// transform (tipped 75°, turned 45°, scaled to 0.375).
pub fn held_block_third_person(pose: &PlayerPose) -> Mat4 {
    let arm = setup_anim(pose).right_arm;
    let pivot = vanilla_space().transform_point3(Vec3::from(RIGHT_ARM.pivot));
    body_transform(pose)
        * vanilla_space()
        * Mat4::from_translation(pivot + arm.offset)
        * arm.rotation()
        // Model pixels to blocks.
        * Mat4::from_scale(Vec3::splat(16.0))
        * Mat4::from_rotation_x(degrees(-90.0))
        * Mat4::from_rotation_y(degrees(180.0))
        * Mat4::from_translation(Vec3::new(1.0, 2.0, -10.0) / 16.0)
        * Mat4::from_translation(Vec3::new(0.0, 2.5, 0.0) / 16.0)
        * Mat4::from_rotation_x(degrees(75.0))
        * Mat4::from_rotation_y(degrees(45.0))
        * Mat4::from_scale(Vec3::splat(0.375))
        * Mat4::from_translation(Vec3::splat(-0.5))
}

/// Model-pixel to world transform of the whole body (our part coordinates:
/// Y up from the feet). When swimming the body tips forward to lie along the
/// look direction, centred on the 0.6 block tall swimming hitbox (vanilla's
/// setupRotations); crouching lowers it by two pixels.
fn body_transform(pose: &PlayerPose) -> Mat4 {
    let s = pose.swim;
    let tip = (-std::f32::consts::FRAC_PI_2 + pose.head_pitch) * s;
    let crouch = if pose.crouching { -2.0 / 16.0 } else { 0.0 };
    Mat4::from_translation(pose.feet)
        * Mat4::from_rotation_x(-pose.lean)
        * Mat4::from_translation(Vec3::Y * (0.3 * s + crouch))
        * Mat4::from_rotation_y(-pose.yaw)
        * Mat4::from_rotation_x(tip)
        * Mat4::from_translation(Vec3::Y * -0.9 * s)
        * Mat4::from_scale(Vec3::splat(PIXEL))
}

/// Our part coordinates to vanilla's model space and back (it is its own
/// inverse): vanilla's Y points down from the neck and X is mirrored.
fn vanilla_space() -> Mat4 {
    Mat4::from_translation(Vec3::new(0.0, 24.0, 0.0)) * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0))
}

/// A model part's pose in vanilla's terms (ModelPart's x/y/z and rotations).
#[derive(Clone, Copy, Default)]
struct PartPose {
    /// Rotations about X, Y and Z in radians, applied Z·Y·X like vanilla.
    rot: Vec3,
    /// Movement of the pivot from its rest position, in pixels.
    offset: Vec3,
}

impl PartPose {
    fn rotation(self) -> Mat4 {
        Mat4::from_rotation_z(self.rot.z) * Mat4::from_rotation_y(self.rot.y) * Mat4::from_rotation_x(self.rot.x)
    }
}

#[derive(Default)]
struct ModelPose {
    head: PartPose,
    body: PartPose,
    right_arm: PartPose,
    left_arm: PartPose,
    right_leg: PartPose,
    left_leg: PartPose,
}

/// Model-pixel to world transform of a posed part, for its boxes in our
/// part coordinates.
fn part_transform(pose: &PlayerPose, part: &Part, part_pose: PartPose) -> Mat4 {
    let pivot = vanilla_space().transform_point3(Vec3::from(part.pivot));
    body_transform(pose)
        * vanilla_space()
        * Mat4::from_translation(pivot + part_pose.offset)
        * part_pose.rotation()
        * Mat4::from_translation(-pivot)
        * vanilla_space()
}

/// Rotation from `from` towards `to` by `t`, the short way round (Mth.rotLerpRad).
fn rot_lerp(t: f32, from: f32, to: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let diff = (to - from + PI).rem_euclid(TAU) - PI;
    from + t * diff
}

fn lerp(t: f32, from: f32, to: f32) -> f32 {
    from + (to - from) * t
}

/// Poses the model like vanilla's HumanoidModel.setupAnim: walking, the
/// arm holding an item, the attack swing, crouching, the arms' idle sway
/// and swimming strokes.
fn setup_anim(pose: &PlayerPose) -> ModelPose {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};
    let mut m = ModelPose::default();
    m.head.rot = Vec3::new(-pose.head_pitch, pose.head_yaw, 0.0);

    let (position, speed) = (pose.limb_position, pose.limb_speed);
    let walk = position * 0.6662;
    m.right_arm.rot.x = (walk + PI).cos() * speed;
    m.left_arm.rot.x = walk.cos() * speed;
    m.right_leg.rot = Vec3::new(walk.cos() * 1.4 * speed, 0.005, 0.005);
    m.left_leg.rot = Vec3::new((walk + PI).cos() * 1.4 * speed, -0.005, -0.005);
    if pose.holding {
        m.right_arm.rot.x = m.right_arm.rot.x * 0.5 - PI / 10.0;
    }

    let a = pose.attack;
    if a > 0.0 {
        // The body twists into the swing, carrying the shoulders along.
        let twist = (a.sqrt() * TAU).sin() * 0.2;
        m.body.rot.y = twist;
        m.right_arm.offset = Vec3::new(5.0 - twist.cos() * 5.0, 0.0, twist.sin() * 5.0);
        m.left_arm.offset = Vec3::new(twist.cos() * 5.0 - 5.0, 0.0, -twist.sin() * 5.0);
        m.right_arm.rot.y += twist;
        m.left_arm.rot.y += twist;
        m.left_arm.rot.x += twist;
        let out_quart = 1.0 - (1.0 - a).powi(4);
        let lift = (out_quart * PI).sin() * 1.2 + (a * PI).sin() * -(m.head.rot.x - 0.7) * 0.75;
        m.right_arm.rot.x -= lift;
        m.right_arm.rot.y += twist * 2.0;
        m.right_arm.rot.z += (a * PI).sin() * -0.4;
    }

    if pose.crouching {
        m.body.rot.x = 0.5;
        m.right_arm.rot.x += 0.4;
        m.left_arm.rot.x += 0.4;
        m.right_leg.offset.z += 4.0;
        m.left_leg.offset.z += 4.0;
        m.head.offset.y += 4.2;
        m.body.offset.y += 3.2;
        m.left_arm.offset.y += 3.2;
        m.right_arm.offset.y += 3.2;
    }

    let (sway_z, sway_x) = ((pose.age * 0.09).cos() * 0.05 + 0.05, (pose.age * 0.067).sin() * 0.05);
    m.right_arm.rot += Vec3::new(sway_x, 0.0, sway_z);
    m.left_arm.rot -= Vec3::new(sway_x, 0.0, sway_z);

    let swim = pose.swim;
    if swim > 0.0 {
        m.head.rot.x = rot_lerp(swim, m.head.rot.x, -FRAC_PI_4);
        // The arm swinging an attack doesn't swim.
        let (right, left) = (if a > 0.0 { 0.0 } else { swim }, swim);
        let quadratic = |x: f32| -65.0 * x + x * x;
        let stroke = position % 26.0;
        let (x, z_left, z_right) = if stroke < 14.0 {
            let reach = 1.870_796_4 * quadratic(stroke) / quadratic(14.0);
            (0.0, PI + reach, PI - reach)
        } else if stroke < 22.0 {
            let t = (stroke - 14.0) / 8.0;
            (FRAC_PI_2 * t, 5.012_389 - 1.870_796_4 * t, 1.270_796_3 + 1.870_796_4 * t)
        } else {
            let t = (stroke - 22.0) / 4.0;
            (FRAC_PI_2 - FRAC_PI_2 * t, PI, PI)
        };
        m.left_arm.rot = Vec3::new(
            rot_lerp(left, m.left_arm.rot.x, x),
            rot_lerp(left, m.left_arm.rot.y, PI),
            rot_lerp(left, m.left_arm.rot.z, z_left),
        );
        m.right_arm.rot = Vec3::new(
            lerp(right, m.right_arm.rot.x, x),
            lerp(right, m.right_arm.rot.y, PI),
            lerp(right, m.right_arm.rot.z, z_right),
        );
        m.left_leg.rot.x = lerp(swim, m.left_leg.rot.x, 0.3 * (position / 3.0 + PI).cos());
        m.right_leg.rot.x = lerp(swim, m.right_leg.rot.x, 0.3 * (position / 3.0).cos());
    }
    m
}

/// Builds the first-person right arm in world space, placed and swung like
/// vanilla's ItemInHandRenderer.renderPlayerArm.
pub fn build_hand_mesh(pose: &HandPose) -> Vec<EntityVertex> {
    use std::f32::consts::{PI, TAU};
    let a = pose.swing;
    let sa = a.sqrt();
    let arm = Mat4::from_translation(Vec3::new(
        0.64 - 0.3 * (sa * PI).sin(),
        -0.6 + 0.4 * (sa * TAU).sin() - 0.6 * pose.lowered,
        -0.72 - 0.4 * (a * PI).sin(),
    )) * Mat4::from_rotation_y(degrees(45.0 + 70.0 * (sa * PI).sin()))
        * Mat4::from_rotation_z(degrees(-20.0 * (a * a * PI).sin()))
        * Mat4::from_translation(Vec3::new(-1.0, 3.6, 3.5))
        * Mat4::from_rotation_z(degrees(120.0))
        * Mat4::from_rotation_x(degrees(200.0))
        * Mat4::from_rotation_y(degrees(-135.0))
        * Mat4::from_translation(Vec3::new(5.6, 0.0, 0.0));
    // The arm part itself, in vanilla's model space (Y down, X mirrored from
    // ours): at its shoulder pivot and tilted out by 0.1 rad, 1/16 per pixel.
    let pivot = Vec3::new(-5.0, 2.0, 0.0);
    let part = Mat4::from_scale(Vec3::splat(1.0 / 16.0))
        * Mat4::from_translation(pivot)
        * Mat4::from_rotation_z(0.1)
        * Mat4::from_translation(-pivot)
        * Mat4::from_translation(Vec3::new(0.0, 24.0, 0.0))
        * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0));
    let transform = hand_space(pose) * arm * part;
    let mut out = Vec::with_capacity(72);
    push_box(&mut out, &RIGHT_ARM, RIGHT_ARM.uv, 0.0, transform, pose.light);
    push_box(&mut out, &RIGHT_ARM, RIGHT_ARM.overlay_uv, RIGHT_ARM.overlay_inflate, transform, pose.light);
    out
}

/// Builds the posed model as a triangle list in world space.
pub fn build_mesh(pose: &PlayerPose) -> Vec<EntityVertex> {
    let m = setup_anim(pose);
    let parts = [
        (&HEAD, m.head),
        (&BODY, m.body),
        (&RIGHT_ARM, m.right_arm),
        (&LEFT_ARM, m.left_arm),
        (&RIGHT_LEG, m.right_leg),
        (&LEFT_LEG, m.left_leg),
    ];
    let mut out = Vec::with_capacity(parts.len() * 2 * 36);
    for (part, part_pose) in parts {
        let transform = part_transform(pose, part, part_pose);
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
    /// The player shown in the inventory screen.
    preview_buffer: wgpu::Buffer,
    preview_vertices: u32,
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
        let view = gpu::upload_texture(device, queue, "skin", skin.width, skin.height, &skin.pixels)
            .create_view(&Default::default());
        let sampler =
            device.create_sampler(&wgpu::SamplerDescriptor { label: Some("skin sampler"), ..Default::default() });
        let (layout, bind_group) =
            gpu::texture_binding(device, "skin", &view, &sampler, wgpu::TextureViewDimension::D2);

        let shader = crate::renderer::shader_module(device, "entity shader", include_str!("shaders/entity.wgsl"));
        let pipeline_layout = gpu::pipeline_layout(device, "entity pipeline layout", &[globals_layout, &layout]);
        let pipeline = PipelineDesc {
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<EntityVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Float32],
            })],
            blend: wgpu::BlendState::REPLACE,
            depth: Some(Depth::opaque(depth_format)),
            ..PipelineDesc::new("entity pipeline", &pipeline_layout, &shader, color_format)
        }
        .build(device);

        // Two layers × six faces × six vertices, for six body parts plus the hand.
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("player vertices"),
            size: (2 * 7 * 36 * size_of::<EntityVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let preview_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("player preview vertices"),
            size: (2 * 6 * 36 * size_of::<EntityVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            bind_group,
            vertex_buffer,
            preview_buffer,
            preview_vertices: 0,
            body_vertices: 0,
            hand_vertices: 0,
        }
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
    /// Uploads the inventory-screen player; call before the render pass.
    pub fn prepare_preview(&mut self, queue: &wgpu::Queue, vertices: &[EntityVertex]) {
        if !vertices.is_empty() {
            queue.write_buffer(&self.preview_buffer, 0, bytemuck::cast_slice(vertices));
        }
        self.preview_vertices = vertices.len() as u32;
    }

    pub fn draw_preview(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.preview_vertices == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.preview_buffer.slice(..));
        pass.draw(0..self.preview_vertices, 0..1);
    }

    pub fn draw_hand(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_range(pass, self.body_vertices..self.body_vertices + self.hand_vertices);
    }
}
