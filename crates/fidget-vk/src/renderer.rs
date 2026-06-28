//! Minimal but complete Vulkan renderer built on `ash`.
//!
//! Everything visible (the ball, its glow halo, the motion trail and the
//! particles) is drawn as an *instanced soft circle* ("blob"). A single
//! pipeline with additive blending over a dark background gives the neon glow
//! look without a separate post-processing bloom pass. The CPU fills an
//! instance buffer each frame from the simulation state.

use std::ffi::CStr;

use anyhow::{anyhow, Context, Result};
use ash::vk;
use egui::{ClippedPrimitive, TextureId, TexturesDelta};
use egui_ash_renderer::{Options as EguiRendererOptions, Renderer as EguiRenderer};
use glam::{Vec2, Vec3, Vec4};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// Per-instance vertex data. Layout must match `shaders/blob.vert`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Instance {
    pub center: [f32; 2],
    pub half: [f32; 2],
    pub color: [f32; 4],
    pub softness: f32,
    pub material: f32,
    /// xy = stretch/roll axis, z = visual roll angle in radians.
    pub roll: [f32; 4],
}

pub struct EguiDrawData<'a> {
    pub textures_delta: &'a TexturesDelta,
    pub clipped_primitives: &'a [ClippedPrimitive],
    pub pixels_per_point: f32,
}

const MAX_INSTANCES: usize = 8192;
const MAX_RUBBER_VERTICES: usize = 16_384;
const MAX_RUBBER_INDICES: usize = 65_536;
const FRAMES_IN_FLIGHT: usize = 2;
const RUBBER_RING_SEGMENTS: usize = 14;

// Embedded compiled shaders (produced by build.rs).
const VERT_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/blob.vert.spv"));
const FRAG_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/blob.frag.spv"));
const MESH_VERT_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ball_mesh.vert.spv"));
const MESH_FRAG_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ball_mesh.frag.spv"));
const RUBBER_VERT_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/rubber_mesh.vert.spv"));
const RUBBER_FRAG_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/rubber_mesh.frag.spv"));
const SOCCER_TEXTURE_PNG: &[u8] = include_bytes!("../../../assets/soccer_ball_material.png");
const SOCCER_GLB: &[u8] =
    include_bytes!("../../../assets/Meshy_AI_Soccer_ball_0628153454_texture.glb");

/// A GPU buffer plus its backing memory and (optional) persistent mapping.
struct Buffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    mapped: *mut u8,
}

struct Texture {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    sampler: vk::Sampler,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct MeshVertex {
    pos: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}

/// Dynamic 3D tube vertex for the rubber band. Layout must match
/// `shaders/rubber_mesh.vert`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RubberVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
    /// x = normalized path length, y = joint strength, z = local tube radius.
    pub rubber: [f32; 4],
}

#[derive(Default)]
pub struct RubberBandMesh {
    pub vertices: Vec<RubberVertex>,
    pub indices: Vec<u32>,
}

impl RubberBandMesh {
    pub fn with_capacity(vertices: usize, indices: usize) -> Self {
        Self {
            vertices: Vec::with_capacity(vertices),
            indices: Vec::with_capacity(indices),
        }
    }

    pub fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
    }

    pub fn rebuild(
        &mut self,
        path: &[Vec2],
        joints: &[Vec2],
        primary: Vec4,
        accent: Vec4,
        radius: f32,
    ) {
        self.clear();
        if path.len() < 2 {
            return;
        }

        let samples = sample_rubber_path(path);
        if samples.len() < 2 {
            return;
        }

        append_tube(self, &samples, joints, primary, accent, radius);
        for &joint in joints {
            append_joint_bulb(self, joint, radius * 1.55, primary, accent);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

struct BallMesh {
    vertex_buffer: Buffer,
    index_buffer: Buffer,
    index_count: u32,
    texture: Texture,
}

/// Swapchain-dependent resources, recreated on resize.
struct SwapchainBundle {
    swapchain: vk::SwapchainKHR,
    extent: vk::Extent2D,
    views: Vec<vk::ImageView>,
    framebuffers: Vec<vk::Framebuffer>,
}

pub struct Renderer {
    _entry: ash::Entry,
    instance: ash::Instance,
    surface_loader: ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    physical_device: vk::PhysicalDevice,
    device: ash::Device,
    queue: vk::Queue,
    swapchain_loader: ash::khr::swapchain::Device,

    swap: SwapchainBundle,
    render_pass: vk::RenderPass,
    descriptor_set_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    descriptor_set: vk::DescriptorSet,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    mesh_pipeline_layout: vk::PipelineLayout,
    mesh_pipeline: vk::Pipeline,
    rubber_pipeline_layout: vk::PipelineLayout,
    rubber_pipeline: vk::Pipeline,
    egui_renderer: Option<EguiRenderer>,

    command_pool: vk::CommandPool,
    command_buffers: Vec<vk::CommandBuffer>,
    quad_buffer: Buffer,
    instance_buffers: Vec<Buffer>,
    rubber_vertex_buffers: Vec<Buffer>,
    rubber_index_buffers: Vec<Buffer>,
    ball_mesh: BallMesh,

    image_available: Vec<vk::Semaphore>,
    render_finished: Vec<vk::Semaphore>,
    in_flight: Vec<vk::Fence>,
    pending_egui_texture_frees: Vec<Vec<TextureId>>,
    current_frame: usize,

    /// Background clear colour (dark so additive glow pops).
    pub clear_color: [f32; 4],
}

impl Renderer {
    pub fn new(
        display_handle: RawDisplayHandle,
        window_handle: RawWindowHandle,
        window_size: (u32, u32),
    ) -> Result<Self> {
        let entry = unsafe { ash::Entry::load().context("failed to load Vulkan loader")? };

        // --- Instance ------------------------------------------------------
        let app_info = vk::ApplicationInfo::default()
            .application_name(c"fidget-vk")
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(c"fidget-vk")
            .api_version(vk::make_api_version(0, 1, 1, 0));

        let extensions = ash_window::enumerate_required_extensions(display_handle)
            .context("failed to query required surface extensions")?
            .to_vec();

        let instance_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&extensions);
        let instance = unsafe { entry.create_instance(&instance_info, None)? };

        // --- Surface -------------------------------------------------------
        let surface_loader = ash::khr::surface::Instance::new(&entry, &instance);
        let surface = unsafe {
            ash_window::create_surface(&entry, &instance, display_handle, window_handle, None)
                .context("failed to create Vulkan surface")?
        };

        // --- Physical device + queue family --------------------------------
        let (physical_device, queue_family) = pick_device(&instance, &surface_loader, surface)?;
        let mem_props = unsafe { instance.get_physical_device_memory_properties(physical_device) };

        let device_name = unsafe {
            let props = instance.get_physical_device_properties(physical_device);
            CStr::from_ptr(props.device_name.as_ptr())
                .to_string_lossy()
                .into_owned()
        };
        log::info!("selected GPU: {device_name} (queue family {queue_family})");

        // --- Logical device + queue ----------------------------------------
        let priorities = [1.0f32];
        let queue_info = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family)
            .queue_priorities(&priorities);
        let device_extensions = [ash::khr::swapchain::NAME.as_ptr()];
        let device_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(std::slice::from_ref(&queue_info))
            .enabled_extension_names(&device_extensions);
        let device = unsafe { instance.create_device(physical_device, &device_info, None)? };
        let queue = unsafe { device.get_device_queue(queue_family, 0) };
        let swapchain_loader = ash::khr::swapchain::Device::new(&instance, &device);

        // --- Swapchain + render pass ---------------------------------------
        let surface_format = choose_surface_format(&surface_loader, physical_device, surface)?;
        let render_pass = create_render_pass(&device, surface_format.format)?;
        let swap = create_swapchain(
            &swapchain_loader,
            &surface_loader,
            &device,
            physical_device,
            surface,
            surface_format,
            render_pass,
            window_size,
            vk::SwapchainKHR::null(),
        )?;

        // --- Pipeline descriptors + pipeline -------------------------------
        let descriptor_set_layout = create_descriptor_set_layout(&device)?;
        let (pipeline_layout, pipeline) =
            create_pipeline(&device, render_pass, descriptor_set_layout)?;
        let (mesh_pipeline_layout, mesh_pipeline) =
            create_mesh_pipeline(&device, render_pass, descriptor_set_layout)?;
        let (rubber_pipeline_layout, rubber_pipeline) =
            create_rubber_pipeline(&device, render_pass)?;
        let egui_renderer = EguiRenderer::with_default_allocator(
            &instance,
            physical_device,
            device.clone(),
            render_pass,
            EguiRendererOptions {
                in_flight_frames: FRAMES_IN_FLIGHT,
                enable_depth_test: false,
                enable_depth_write: false,
                srgb_framebuffer: false,
            },
        )
        .map_err(|e| anyhow!("failed to create egui renderer: {e}"))?;

        // --- Command pool + buffers ----------------------------------------
        let command_pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(queue_family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )?
        };
        let command_buffers = unsafe {
            device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(FRAMES_IN_FLIGHT as u32),
            )?
        };

        // Static quad (two triangles) covering [-1, 1] for instanced draws.
        let quad: [[f32; 2]; 6] = [
            [-1.0, -1.0],
            [1.0, -1.0],
            [1.0, 1.0],
            [-1.0, -1.0],
            [1.0, 1.0],
            [-1.0, 1.0],
        ];
        let quad_buffer = create_host_buffer(
            &device,
            &mem_props,
            std::mem::size_of_val(&quad) as vk::DeviceSize,
            vk::BufferUsageFlags::VERTEX_BUFFER,
        )?;
        unsafe {
            std::ptr::copy_nonoverlapping(
                quad.as_ptr() as *const u8,
                quad_buffer.mapped,
                std::mem::size_of_val(&quad),
            );
        }

        let mut instance_buffers = Vec::with_capacity(FRAMES_IN_FLIGHT);
        for _ in 0..FRAMES_IN_FLIGHT {
            instance_buffers.push(create_host_buffer(
                &device,
                &mem_props,
                (MAX_INSTANCES * std::mem::size_of::<Instance>()) as vk::DeviceSize,
                vk::BufferUsageFlags::VERTEX_BUFFER,
            )?);
        }
        let mut rubber_vertex_buffers = Vec::with_capacity(FRAMES_IN_FLIGHT);
        let mut rubber_index_buffers = Vec::with_capacity(FRAMES_IN_FLIGHT);
        for _ in 0..FRAMES_IN_FLIGHT {
            rubber_vertex_buffers.push(create_host_buffer(
                &device,
                &mem_props,
                (MAX_RUBBER_VERTICES * std::mem::size_of::<RubberVertex>()) as vk::DeviceSize,
                vk::BufferUsageFlags::VERTEX_BUFFER,
            )?);
            rubber_index_buffers.push(create_host_buffer(
                &device,
                &mem_props,
                (MAX_RUBBER_INDICES * std::mem::size_of::<u32>()) as vk::DeviceSize,
                vk::BufferUsageFlags::INDEX_BUFFER,
            )?);
        }

        let ball_mesh = create_ball_mesh(&device, &mem_props, command_pool, queue, SOCCER_GLB)
            .context("failed to load GLB soccer ball mesh")?;
        let (descriptor_pool, descriptor_set) =
            create_texture_descriptor(&device, descriptor_set_layout, &ball_mesh.texture)?;

        // --- Sync primitives ----------------------------------------------
        let mut image_available = Vec::new();
        let mut render_finished = Vec::new();
        let mut in_flight = Vec::new();
        let mut pending_egui_texture_frees = Vec::new();
        for _ in 0..FRAMES_IN_FLIGHT {
            unsafe {
                image_available
                    .push(device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?);
                render_finished
                    .push(device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?);
                in_flight.push(device.create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                )?);
                pending_egui_texture_frees.push(Vec::new());
            }
        }

        Ok(Self {
            _entry: entry,
            instance,
            surface_loader,
            surface,
            physical_device,
            device,
            queue,
            swapchain_loader,
            swap,
            render_pass,
            descriptor_set_layout,
            descriptor_pool,
            descriptor_set,
            pipeline_layout,
            pipeline,
            mesh_pipeline_layout,
            mesh_pipeline,
            rubber_pipeline_layout,
            rubber_pipeline,
            egui_renderer: Some(egui_renderer),
            command_pool,
            command_buffers,
            quad_buffer,
            instance_buffers,
            rubber_vertex_buffers,
            rubber_index_buffers,
            ball_mesh,
            image_available,
            render_finished,
            in_flight,
            pending_egui_texture_frees,
            current_frame: 0,
            clear_color: [0.0, 0.0, 0.0, 0.0],
        })
    }

    #[allow(dead_code)]
    pub fn extent(&self) -> (u32, u32) {
        (self.swap.extent.width, self.swap.extent.height)
    }

    /// Recreate the swapchain (e.g. after a resize). Safe to call with a
    /// zero-sized window (it becomes a no-op until a real size arrives).
    pub fn resize(&mut self, size: (u32, u32)) -> Result<()> {
        if size.0 == 0 || size.1 == 0 {
            return Ok(());
        }
        unsafe { self.device.device_wait_idle()? };
        let surface_format =
            choose_surface_format(&self.surface_loader, self.physical_device, self.surface)?;
        let old = self.swap.swapchain;
        let new = create_swapchain(
            &self.swapchain_loader,
            &self.surface_loader,
            &self.device,
            self.physical_device,
            self.surface,
            surface_format,
            self.render_pass,
            size,
            old,
        )?;
        self.destroy_swapchain();
        self.swap = new;
        Ok(())
    }

    /// Render one frame from the given instances. Returns `Ok(false)` if the
    /// swapchain needs recreation (the caller should call `resize`).
    pub fn render(
        &mut self,
        instances: &[Instance],
        rubber_band: &RubberBandMesh,
        egui: Option<EguiDrawData<'_>>,
    ) -> Result<bool> {
        let frame = self.current_frame;
        let fence = self.in_flight[frame];
        unsafe {
            self.device.wait_for_fences(&[fence], true, u64::MAX)?;
        }
        if !self.pending_egui_texture_frees[frame].is_empty() {
            let ids = std::mem::take(&mut self.pending_egui_texture_frees[frame]);
            self.egui_renderer
                .as_mut()
                .expect("egui renderer")
                .free_textures(&ids)
                .map_err(|e| anyhow!("failed to free egui textures: {e}"))?;
        }

        let acquire = unsafe {
            self.swapchain_loader.acquire_next_image(
                self.swap.swapchain,
                u64::MAX,
                self.image_available[frame],
                vk::Fence::null(),
            )
        };
        let image_index = match acquire {
            Ok((idx, _suboptimal)) => idx,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => return Ok(false),
            Err(e) => return Err(anyhow!("acquire_next_image failed: {e}")),
        };

        unsafe { self.device.reset_fences(&[fence])? };

        if let Some(egui) = egui.as_ref() {
            self.egui_renderer
                .as_mut()
                .expect("egui renderer")
                .set_textures(self.queue, self.command_pool, &egui.textures_delta.set)
                .map_err(|e| anyhow!("failed to upload egui textures: {e}"))?;
        }

        // Upload blob instances for this frame. The material=1 ball body
        // instance becomes the transform for the GLB mesh draw instead.
        let mut count = 0usize;
        let mut ball_instance = None;
        unsafe {
            let dst = self.instance_buffers[frame].mapped as *mut Instance;
            for instance in instances {
                if instance.material > 0.5 {
                    ball_instance = Some(*instance);
                } else if count < MAX_INSTANCES {
                    dst.add(count).write(*instance);
                    count += 1;
                }
            }
        }

        let (_rubber_vertex_count, rubber_index_count) =
            self.upload_rubber_band(frame, rubber_band);

        self.record_command_buffer(
            frame,
            image_index as usize,
            count as u32,
            rubber_index_count,
            ball_instance,
            egui.as_ref(),
        )?;

        let wait = [self.image_available[frame]];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let signal = [self.render_finished[frame]];
        let cmd = [self.command_buffers[frame]];
        let submit = vk::SubmitInfo::default()
            .wait_semaphores(&wait)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&cmd)
            .signal_semaphores(&signal);
        unsafe { self.device.queue_submit(self.queue, &[submit], fence)? };

        let swapchains = [self.swap.swapchain];
        let indices = [image_index];
        let present = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal)
            .swapchains(&swapchains)
            .image_indices(&indices);
        let present_result = unsafe { self.swapchain_loader.queue_present(self.queue, &present) };

        self.current_frame = (frame + 1) % FRAMES_IN_FLIGHT;
        if let Some(egui) = egui {
            self.pending_egui_texture_frees[frame].extend(egui.textures_delta.free.iter().copied());
        }

        match present_result {
            Ok(false) => Ok(true),
            Ok(true) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => Ok(false),
            Err(e) => Err(anyhow!("queue_present failed: {e}")),
        }
    }

    fn upload_rubber_band(&mut self, frame: usize, rubber_band: &RubberBandMesh) -> (u32, u32) {
        if rubber_band.is_empty()
            || rubber_band.vertices.len() > MAX_RUBBER_VERTICES
            || rubber_band.indices.len() > MAX_RUBBER_INDICES
        {
            return (0, 0);
        }

        unsafe {
            std::ptr::copy_nonoverlapping(
                rubber_band.vertices.as_ptr() as *const u8,
                self.rubber_vertex_buffers[frame].mapped,
                std::mem::size_of_val(rubber_band.vertices.as_slice()),
            );
            std::ptr::copy_nonoverlapping(
                rubber_band.indices.as_ptr() as *const u8,
                self.rubber_index_buffers[frame].mapped,
                std::mem::size_of_val(rubber_band.indices.as_slice()),
            );
        }

        (
            rubber_band.vertices.len() as u32,
            rubber_band.indices.len() as u32,
        )
    }

    fn record_command_buffer(
        &mut self,
        frame: usize,
        image_index: usize,
        count: u32,
        rubber_index_count: u32,
        ball_instance: Option<Instance>,
        egui: Option<&EguiDrawData<'_>>,
    ) -> Result<()> {
        let cmd = self.command_buffers[frame];
        unsafe {
            self.device
                .reset_command_buffer(cmd, vk::CommandBufferResetFlags::empty())?;
            self.device.begin_command_buffer(
                cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;

            let clear = [vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: self.clear_color,
                },
            }];
            let rp_begin = vk::RenderPassBeginInfo::default()
                .render_pass(self.render_pass)
                .framebuffer(self.swap.framebuffers[image_index])
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: self.swap.extent,
                })
                .clear_values(&clear);
            self.device
                .cmd_begin_render_pass(cmd, &rp_begin, vk::SubpassContents::INLINE);

            self.device
                .cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
            self.device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline_layout,
                0,
                &[self.descriptor_set],
                &[],
            );

            let viewport = vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: self.swap.extent.width as f32,
                height: self.swap.extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            let scissor = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: self.swap.extent,
            };
            self.device.cmd_set_viewport(cmd, 0, &[viewport]);
            self.device.cmd_set_scissor(cmd, 0, &[scissor]);

            let resolution = [
                self.swap.extent.width as f32,
                self.swap.extent.height as f32,
            ];
            self.device.cmd_push_constants(
                cmd,
                self.pipeline_layout,
                vk::ShaderStageFlags::VERTEX,
                0,
                bytemuck_bytes(&resolution),
            );

            self.device.cmd_bind_vertex_buffers(
                cmd,
                0,
                &[self.quad_buffer.buffer, self.instance_buffers[frame].buffer],
                &[0, 0],
            );

            if count > 0 {
                self.device.cmd_draw(cmd, 6, count, 0, 0);
            }

            if rubber_index_count > 0 {
                let rubber_push = [
                    self.swap.extent.width as f32,
                    self.swap.extent.height as f32,
                    0.0,
                    0.0,
                ];
                self.device.cmd_bind_pipeline(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.rubber_pipeline,
                );
                self.device.cmd_push_constants(
                    cmd,
                    self.rubber_pipeline_layout,
                    vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                    0,
                    bytemuck_bytes(&rubber_push),
                );
                self.device.cmd_bind_vertex_buffers(
                    cmd,
                    0,
                    &[self.rubber_vertex_buffers[frame].buffer],
                    &[0],
                );
                self.device.cmd_bind_index_buffer(
                    cmd,
                    self.rubber_index_buffers[frame].buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device
                    .cmd_draw_indexed(cmd, rubber_index_count, 1, 0, 0, 0);
            }

            if let Some(ball) = ball_instance {
                let mesh_push = [
                    [
                        self.swap.extent.width as f32,
                        self.swap.extent.height as f32,
                        ball.center[0],
                        ball.center[1],
                    ],
                    [ball.half[0], ball.half[1], ball.roll[0], ball.roll[1]],
                    [ball.roll[2], 0.0, 0.0, 0.0],
                ];
                self.device.cmd_bind_pipeline(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.mesh_pipeline,
                );
                self.device.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.mesh_pipeline_layout,
                    0,
                    &[self.descriptor_set],
                    &[],
                );
                self.device.cmd_push_constants(
                    cmd,
                    self.mesh_pipeline_layout,
                    vk::ShaderStageFlags::VERTEX,
                    0,
                    bytemuck_bytes(&mesh_push),
                );
                self.device.cmd_bind_vertex_buffers(
                    cmd,
                    0,
                    &[self.ball_mesh.vertex_buffer.buffer],
                    &[0],
                );
                self.device.cmd_bind_index_buffer(
                    cmd,
                    self.ball_mesh.index_buffer.buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device
                    .cmd_draw_indexed(cmd, self.ball_mesh.index_count, 1, 0, 0, 0);
            }

            if let Some(egui) = egui {
                self.egui_renderer
                    .as_mut()
                    .expect("egui renderer")
                    .cmd_draw(
                        cmd,
                        self.swap.extent,
                        egui.pixels_per_point,
                        egui.clipped_primitives,
                    )
                    .map_err(|e| anyhow!("failed to draw egui HUD: {e}"))?;
            }

            self.device.cmd_end_render_pass(cmd);
            self.device.end_command_buffer(cmd)?;
        }
        Ok(())
    }

    fn destroy_swapchain(&mut self) {
        unsafe {
            for &fb in &self.swap.framebuffers {
                self.device.destroy_framebuffer(fb, None);
            }
            for &view in &self.swap.views {
                self.device.destroy_image_view(view, None);
            }
            self.swapchain_loader
                .destroy_swapchain(self.swap.swapchain, None);
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            drop(self.egui_renderer.take());
            for &s in &self.image_available {
                self.device.destroy_semaphore(s, None);
            }
            for &s in &self.render_finished {
                self.device.destroy_semaphore(s, None);
            }
            for &f in &self.in_flight {
                self.device.destroy_fence(f, None);
            }
            for b in &self.instance_buffers {
                destroy_buffer(&self.device, b);
            }
            for b in &self.rubber_index_buffers {
                destroy_buffer(&self.device, b);
            }
            for b in &self.rubber_vertex_buffers {
                destroy_buffer(&self.device, b);
            }
            destroy_buffer(&self.device, &self.ball_mesh.index_buffer);
            destroy_buffer(&self.device, &self.ball_mesh.vertex_buffer);
            destroy_texture(&self.device, &self.ball_mesh.texture);
            self.device
                .destroy_descriptor_pool(self.descriptor_pool, None);
            self.device
                .destroy_descriptor_set_layout(self.descriptor_set_layout, None);
            destroy_buffer(&self.device, &self.quad_buffer);
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_pipeline(self.rubber_pipeline, None);
            self.device
                .destroy_pipeline_layout(self.rubber_pipeline_layout, None);
            self.device.destroy_pipeline(self.mesh_pipeline, None);
            self.device
                .destroy_pipeline_layout(self.mesh_pipeline_layout, None);
            self.device.destroy_pipeline(self.pipeline, None);
            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);
            self.destroy_swapchain();
            self.device.destroy_render_pass(self.render_pass, None);
            self.device.destroy_device(None);
            self.surface_loader.destroy_surface(self.surface, None);
            self.instance.destroy_instance(None);
        }
    }
}

// --- free helpers ---------------------------------------------------------

fn bytemuck_bytes<T>(v: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v as *const T as *const u8, std::mem::size_of::<T>()) }
}

fn sample_rubber_path(path: &[Vec2]) -> Vec<Vec2> {
    let mut samples = Vec::with_capacity(path.len() * 8);
    for i in 0..path.len() - 1 {
        let p1 = path[i];
        let p2 = path[i + 1];
        let len = p1.distance(p2);
        if len <= 0.5 {
            continue;
        }

        let steps = (len / 18.0).ceil().clamp(2.0, 18.0) as usize;
        for step in 0..steps {
            if i > 0 && step == 0 {
                continue;
            }

            let t = step as f32 / steps as f32;
            let point = if path.len() > 2 {
                let p0 = if i == 0 { p1 } else { path[i - 1] };
                let p3 = if i + 2 < path.len() { path[i + 2] } else { p2 };
                catmull_rom(p0, p1, p2, p3, t)
            } else {
                p1.lerp(p2, t)
            };
            samples.push(point);
        }
    }

    if let Some(&last) = path.last() {
        samples.push(last);
    }
    samples
}

fn catmull_rom(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t: f32) -> Vec2 {
    let t2 = t * t;
    let t3 = t2 * t;
    ((p1 * 2.0)
        + (p2 - p0) * t
        + (p0 * 2.0 - p1 * 5.0 + p2 * 4.0 - p3) * t2
        + (-p0 + p1 * 3.0 - p2 * 3.0 + p3) * t3)
        * 0.5
}

fn append_tube(
    mesh: &mut RubberBandMesh,
    samples: &[Vec2],
    joints: &[Vec2],
    primary: Vec4,
    accent: Vec4,
    radius: f32,
) {
    let mut lengths = Vec::with_capacity(samples.len());
    lengths.push(0.0);
    for i in 1..samples.len() {
        let next = lengths[i - 1] + samples[i].distance(samples[i - 1]);
        lengths.push(next);
    }
    let total = lengths.last().copied().unwrap_or(1.0).max(1.0);
    let base = mesh.vertices.len() as u32;

    for (i, &point) in samples.iter().enumerate() {
        let tangent = tangent_at(samples, i);
        let side = Vec2::new(-tangent.y, tangent.x);
        let length_t = lengths[i] / total;
        let joint = joint_strength(point, joints, radius);
        let local_radius = radius * (1.0 + joint * 0.62);
        let stripe = (length_t * std::f32::consts::TAU * 5.5).sin() * 0.5 + 0.5;
        let color = primary.lerp(accent, (joint * 0.72 + stripe * 0.08).clamp(0.0, 1.0));

        for segment in 0..RUBBER_RING_SEGMENTS {
            let angle = segment as f32 / RUBBER_RING_SEGMENTS as f32 * std::f32::consts::TAU;
            let (sin, cos) = angle.sin_cos();
            let normal = Vec3::new(side.x * cos, side.y * cos, sin).normalize();
            let offset = side * (cos * local_radius);
            mesh.vertices.push(RubberVertex {
                pos: [point.x + offset.x, point.y + offset.y, sin * local_radius],
                normal: normal.to_array(),
                color: [color.x, color.y, color.z, 0.92],
                rubber: [length_t, joint, local_radius, 0.0],
            });
        }
    }

    for ring in 0..samples.len() - 1 {
        for segment in 0..RUBBER_RING_SEGMENTS {
            let next = (segment + 1) % RUBBER_RING_SEGMENTS;
            let a = base + (ring * RUBBER_RING_SEGMENTS + segment) as u32;
            let b = base + (ring * RUBBER_RING_SEGMENTS + next) as u32;
            let c = base + ((ring + 1) * RUBBER_RING_SEGMENTS + segment) as u32;
            let d = base + ((ring + 1) * RUBBER_RING_SEGMENTS + next) as u32;
            mesh.indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
}

fn tangent_at(samples: &[Vec2], i: usize) -> Vec2 {
    let tangent = if i == 0 {
        samples[1] - samples[0]
    } else if i + 1 == samples.len() {
        samples[i] - samples[i - 1]
    } else {
        samples[i + 1] - samples[i - 1]
    };
    let normalized = tangent.normalize_or_zero();
    if normalized.length_squared() > 0.0 {
        normalized
    } else {
        Vec2::Y
    }
}

fn joint_strength(point: Vec2, joints: &[Vec2], radius: f32) -> f32 {
    joints
        .iter()
        .map(|&joint| {
            let proximity = 1.0 - (point.distance(joint) / (radius * 5.0)).clamp(0.0, 1.0);
            proximity * proximity * (3.0 - 2.0 * proximity)
        })
        .fold(0.0_f32, f32::max)
}

fn append_joint_bulb(
    mesh: &mut RubberBandMesh,
    center: Vec2,
    radius: f32,
    primary: Vec4,
    accent: Vec4,
) {
    const LAT_SEGMENTS: usize = 6;
    const LON_SEGMENTS: usize = 16;

    let base = mesh.vertices.len() as u32;
    for lat in 0..=LAT_SEGMENTS {
        let phi = lat as f32 / LAT_SEGMENTS as f32 * std::f32::consts::FRAC_PI_2;
        let z = phi.cos();
        let ring_radius = phi.sin();
        for lon in 0..LON_SEGMENTS {
            let theta = lon as f32 / LON_SEGMENTS as f32 * std::f32::consts::TAU;
            let (sin, cos) = theta.sin_cos();
            let normal = Vec3::new(cos * ring_radius, sin * ring_radius, z).normalize();
            let color = primary.lerp(accent, 0.72 + 0.18 * z);
            mesh.vertices.push(RubberVertex {
                pos: [
                    center.x + normal.x * radius,
                    center.y + normal.y * radius,
                    normal.z * radius,
                ],
                normal: normal.to_array(),
                color: [color.x, color.y, color.z, 0.96],
                rubber: [0.0, 1.0, radius, 1.0],
            });
        }
    }

    for lat in 0..LAT_SEGMENTS {
        for lon in 0..LON_SEGMENTS {
            let next = (lon + 1) % LON_SEGMENTS;
            let a = base + (lat * LON_SEGMENTS + lon) as u32;
            let b = base + (lat * LON_SEGMENTS + next) as u32;
            let c = base + ((lat + 1) * LON_SEGMENTS + lon) as u32;
            let d = base + ((lat + 1) * LON_SEGMENTS + next) as u32;
            mesh.indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
}

fn pick_device(
    instance: &ash::Instance,
    surface_loader: &ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
) -> Result<(vk::PhysicalDevice, u32)> {
    let devices = unsafe { instance.enumerate_physical_devices()? };
    for pd in devices {
        let props = unsafe { instance.get_physical_device_queue_family_properties(pd) };
        for (i, q) in props.iter().enumerate() {
            let graphics = q.queue_flags.contains(vk::QueueFlags::GRAPHICS);
            let present = unsafe {
                surface_loader
                    .get_physical_device_surface_support(pd, i as u32, surface)
                    .unwrap_or(false)
            };
            if graphics && present {
                return Ok((pd, i as u32));
            }
        }
    }
    Err(anyhow!("no Vulkan device with graphics + present support"))
}

fn choose_surface_format(
    surface_loader: &ash::khr::surface::Instance,
    pd: vk::PhysicalDevice,
    surface: vk::SurfaceKHR,
) -> Result<vk::SurfaceFormatKHR> {
    let formats = unsafe { surface_loader.get_physical_device_surface_formats(pd, surface)? };
    let chosen = formats
        .iter()
        .find(|f| {
            f.format == vk::Format::B8G8R8A8_UNORM
                && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
        })
        .or_else(|| formats.first())
        .copied()
        .ok_or_else(|| anyhow!("surface reported no formats"))?;
    Ok(chosen)
}

fn create_render_pass(device: &ash::Device, format: vk::Format) -> Result<vk::RenderPass> {
    let color = vk::AttachmentDescription::default()
        .format(format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);
    let color_ref = vk::AttachmentReference::default()
        .attachment(0)
        .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
    let color_refs = [color_ref];
    let subpass = vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(&color_refs);
    let dependency = vk::SubpassDependency::default()
        .src_subpass(vk::SUBPASS_EXTERNAL)
        .dst_subpass(0)
        .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE);
    let attachments = [color];
    let subpasses = [subpass];
    let dependencies = [dependency];
    let info = vk::RenderPassCreateInfo::default()
        .attachments(&attachments)
        .subpasses(&subpasses)
        .dependencies(&dependencies);
    Ok(unsafe { device.create_render_pass(&info, None)? })
}

#[allow(clippy::too_many_arguments)]
fn create_swapchain(
    swapchain_loader: &ash::khr::swapchain::Device,
    surface_loader: &ash::khr::surface::Instance,
    device: &ash::Device,
    pd: vk::PhysicalDevice,
    surface: vk::SurfaceKHR,
    surface_format: vk::SurfaceFormatKHR,
    render_pass: vk::RenderPass,
    window_size: (u32, u32),
    old: vk::SwapchainKHR,
) -> Result<SwapchainBundle> {
    let caps = unsafe { surface_loader.get_physical_device_surface_capabilities(pd, surface)? };
    let present_modes =
        unsafe { surface_loader.get_physical_device_surface_present_modes(pd, surface)? };

    let present_mode = present_modes
        .iter()
        .copied()
        .find(|&m| m == vk::PresentModeKHR::MAILBOX)
        .unwrap_or(vk::PresentModeKHR::FIFO);

    let extent = if caps.current_extent.width != u32::MAX {
        caps.current_extent
    } else {
        vk::Extent2D {
            width: window_size
                .0
                .clamp(caps.min_image_extent.width, caps.max_image_extent.width),
            height: window_size
                .1
                .clamp(caps.min_image_extent.height, caps.max_image_extent.height),
        }
    };

    let mut image_count = caps.min_image_count + 1;
    if caps.max_image_count > 0 && image_count > caps.max_image_count {
        image_count = caps.max_image_count;
    }

    let composite_alpha = choose_composite_alpha(caps.supported_composite_alpha);
    log::info!(
        "swapchain composite alpha: {:?} (supported: {:?})",
        composite_alpha,
        caps.supported_composite_alpha
    );

    let info = vk::SwapchainCreateInfoKHR::default()
        .surface(surface)
        .min_image_count(image_count)
        .image_format(surface_format.format)
        .image_color_space(surface_format.color_space)
        .image_extent(extent)
        .image_array_layers(1)
        .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
        .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
        .pre_transform(caps.current_transform)
        .composite_alpha(composite_alpha)
        .present_mode(present_mode)
        .clipped(true)
        .old_swapchain(old);

    let swapchain = unsafe { swapchain_loader.create_swapchain(&info, None)? };
    let images = unsafe { swapchain_loader.get_swapchain_images(swapchain)? };

    let mut views = Vec::with_capacity(images.len());
    let mut framebuffers = Vec::with_capacity(images.len());
    for &image in &images {
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(surface_format.format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        let view = unsafe { device.create_image_view(&view_info, None)? };
        views.push(view);

        let attachments = [view];
        let fb_info = vk::FramebufferCreateInfo::default()
            .render_pass(render_pass)
            .attachments(&attachments)
            .width(extent.width)
            .height(extent.height)
            .layers(1);
        framebuffers.push(unsafe { device.create_framebuffer(&fb_info, None)? });
    }

    Ok(SwapchainBundle {
        swapchain,
        extent,
        views,
        framebuffers,
    })
}

fn choose_composite_alpha(supported: vk::CompositeAlphaFlagsKHR) -> vk::CompositeAlphaFlagsKHR {
    [
        vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
        vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
        vk::CompositeAlphaFlagsKHR::INHERIT,
        vk::CompositeAlphaFlagsKHR::OPAQUE,
    ]
    .into_iter()
    .find(|flag| supported.contains(*flag))
    .unwrap_or(vk::CompositeAlphaFlagsKHR::OPAQUE)
}

fn create_descriptor_set_layout(device: &ash::Device) -> Result<vk::DescriptorSetLayout> {
    let binding = vk::DescriptorSetLayoutBinding::default()
        .binding(0)
        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .descriptor_count(1)
        .stage_flags(vk::ShaderStageFlags::FRAGMENT);
    let bindings = [binding];
    let info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
    Ok(unsafe { device.create_descriptor_set_layout(&info, None)? })
}

fn create_pipeline(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    descriptor_set_layout: vk::DescriptorSetLayout,
) -> Result<(vk::PipelineLayout, vk::Pipeline)> {
    let vert = create_shader_module(device, VERT_SPV)?;
    let frag = create_shader_module(device, FRAG_SPV)?;

    let stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vert)
            .name(c"main"),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(frag)
            .name(c"main"),
    ];

    let bindings = [
        // Binding 0: per-vertex quad corner.
        vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(std::mem::size_of::<[f32; 2]>() as u32)
            .input_rate(vk::VertexInputRate::VERTEX),
        // Binding 1: per-instance blob data.
        vk::VertexInputBindingDescription::default()
            .binding(1)
            .stride(std::mem::size_of::<Instance>() as u32)
            .input_rate(vk::VertexInputRate::INSTANCE),
    ];
    let attributes = [
        vk::VertexInputAttributeDescription::default()
            .location(0)
            .binding(0)
            .format(vk::Format::R32G32_SFLOAT)
            .offset(0),
        vk::VertexInputAttributeDescription::default()
            .location(1)
            .binding(1)
            .format(vk::Format::R32G32_SFLOAT)
            .offset(0),
        vk::VertexInputAttributeDescription::default()
            .location(2)
            .binding(1)
            .format(vk::Format::R32G32_SFLOAT)
            .offset(8),
        vk::VertexInputAttributeDescription::default()
            .location(3)
            .binding(1)
            .format(vk::Format::R32G32B32A32_SFLOAT)
            .offset(16),
        vk::VertexInputAttributeDescription::default()
            .location(4)
            .binding(1)
            .format(vk::Format::R32_SFLOAT)
            .offset(32),
        vk::VertexInputAttributeDescription::default()
            .location(5)
            .binding(1)
            .format(vk::Format::R32_SFLOAT)
            .offset(36),
        vk::VertexInputAttributeDescription::default()
            .location(6)
            .binding(1)
            .format(vk::Format::R32G32B32A32_SFLOAT)
            .offset(40),
    ];
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&bindings)
        .vertex_attribute_descriptions(&attributes);

    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);

    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);

    let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0);

    let multisample = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);

    // Premultiplied alpha keeps the transparent overlay clean while still
    // allowing glow instances to layer softly.
    let blend_attachment = vk::PipelineColorBlendAttachmentState::default()
        .blend_enable(true)
        .src_color_blend_factor(vk::BlendFactor::ONE)
        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .color_blend_op(vk::BlendOp::ADD)
        .src_alpha_blend_factor(vk::BlendFactor::ONE)
        .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .alpha_blend_op(vk::BlendOp::ADD)
        .color_write_mask(vk::ColorComponentFlags::RGBA);
    let blend_attachments = [blend_attachment];
    let color_blend =
        vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);

    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic_state =
        vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

    let push_range = vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::VERTEX)
        .offset(0)
        .size(std::mem::size_of::<[f32; 2]>() as u32);
    let push_ranges = [push_range];
    let set_layouts = [descriptor_set_layout];
    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(&set_layouts)
        .push_constant_ranges(&push_ranges);
    let pipeline_layout = unsafe { device.create_pipeline_layout(&layout_info, None)? };

    let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&rasterizer)
        .multisample_state(&multisample)
        .color_blend_state(&color_blend)
        .dynamic_state(&dynamic_state)
        .layout(pipeline_layout)
        .render_pass(render_pass)
        .subpass(0);

    let pipeline = unsafe {
        device
            .create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
            .map_err(|(_, e)| anyhow!("failed to create graphics pipeline: {e}"))?[0]
    };

    unsafe {
        device.destroy_shader_module(vert, None);
        device.destroy_shader_module(frag, None);
    }

    Ok((pipeline_layout, pipeline))
}

fn create_mesh_pipeline(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    descriptor_set_layout: vk::DescriptorSetLayout,
) -> Result<(vk::PipelineLayout, vk::Pipeline)> {
    let vert = create_shader_module(device, MESH_VERT_SPV)?;
    let frag = create_shader_module(device, MESH_FRAG_SPV)?;

    let stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vert)
            .name(c"main"),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(frag)
            .name(c"main"),
    ];

    let bindings = [vk::VertexInputBindingDescription::default()
        .binding(0)
        .stride(std::mem::size_of::<MeshVertex>() as u32)
        .input_rate(vk::VertexInputRate::VERTEX)];
    let attributes = [
        vk::VertexInputAttributeDescription::default()
            .location(0)
            .binding(0)
            .format(vk::Format::R32G32B32_SFLOAT)
            .offset(0),
        vk::VertexInputAttributeDescription::default()
            .location(1)
            .binding(0)
            .format(vk::Format::R32G32B32_SFLOAT)
            .offset(12),
        vk::VertexInputAttributeDescription::default()
            .location(2)
            .binding(0)
            .format(vk::Format::R32G32_SFLOAT)
            .offset(24),
    ];
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&bindings)
        .vertex_attribute_descriptions(&attributes);

    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);
    let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0);
    let multisample = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);
    let blend_attachment = vk::PipelineColorBlendAttachmentState::default()
        .blend_enable(true)
        .src_color_blend_factor(vk::BlendFactor::ONE)
        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .color_blend_op(vk::BlendOp::ADD)
        .src_alpha_blend_factor(vk::BlendFactor::ONE)
        .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .alpha_blend_op(vk::BlendOp::ADD)
        .color_write_mask(vk::ColorComponentFlags::RGBA);
    let blend_attachments = [blend_attachment];
    let color_blend =
        vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);
    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic_state =
        vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

    let push_range = vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::VERTEX)
        .offset(0)
        .size(std::mem::size_of::<[[f32; 4]; 3]>() as u32);
    let push_ranges = [push_range];
    let set_layouts = [descriptor_set_layout];
    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(&set_layouts)
        .push_constant_ranges(&push_ranges);
    let pipeline_layout = unsafe { device.create_pipeline_layout(&layout_info, None)? };

    let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&rasterizer)
        .multisample_state(&multisample)
        .color_blend_state(&color_blend)
        .dynamic_state(&dynamic_state)
        .layout(pipeline_layout)
        .render_pass(render_pass)
        .subpass(0);

    let pipeline = unsafe {
        device
            .create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
            .map_err(|(_, e)| anyhow!("failed to create mesh graphics pipeline: {e}"))?[0]
    };

    unsafe {
        device.destroy_shader_module(vert, None);
        device.destroy_shader_module(frag, None);
    }

    Ok((pipeline_layout, pipeline))
}

fn create_rubber_pipeline(
    device: &ash::Device,
    render_pass: vk::RenderPass,
) -> Result<(vk::PipelineLayout, vk::Pipeline)> {
    let vert = create_shader_module(device, RUBBER_VERT_SPV)?;
    let frag = create_shader_module(device, RUBBER_FRAG_SPV)?;

    let stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vert)
            .name(c"main"),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(frag)
            .name(c"main"),
    ];

    let bindings = [vk::VertexInputBindingDescription::default()
        .binding(0)
        .stride(std::mem::size_of::<RubberVertex>() as u32)
        .input_rate(vk::VertexInputRate::VERTEX)];
    let attributes = [
        vk::VertexInputAttributeDescription::default()
            .location(0)
            .binding(0)
            .format(vk::Format::R32G32B32_SFLOAT)
            .offset(0),
        vk::VertexInputAttributeDescription::default()
            .location(1)
            .binding(0)
            .format(vk::Format::R32G32B32_SFLOAT)
            .offset(12),
        vk::VertexInputAttributeDescription::default()
            .location(2)
            .binding(0)
            .format(vk::Format::R32G32B32A32_SFLOAT)
            .offset(24),
        vk::VertexInputAttributeDescription::default()
            .location(3)
            .binding(0)
            .format(vk::Format::R32G32B32A32_SFLOAT)
            .offset(40),
    ];
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&bindings)
        .vertex_attribute_descriptions(&attributes);

    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);
    let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0);
    let multisample = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);
    let blend_attachment = vk::PipelineColorBlendAttachmentState::default()
        .blend_enable(true)
        .src_color_blend_factor(vk::BlendFactor::ONE)
        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .color_blend_op(vk::BlendOp::ADD)
        .src_alpha_blend_factor(vk::BlendFactor::ONE)
        .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .alpha_blend_op(vk::BlendOp::ADD)
        .color_write_mask(vk::ColorComponentFlags::RGBA);
    let blend_attachments = [blend_attachment];
    let color_blend =
        vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);
    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic_state =
        vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

    let push_range = vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
        .offset(0)
        .size(std::mem::size_of::<[f32; 4]>() as u32);
    let push_ranges = [push_range];
    let layout_info = vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&push_ranges);
    let pipeline_layout = unsafe { device.create_pipeline_layout(&layout_info, None)? };

    let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&rasterizer)
        .multisample_state(&multisample)
        .color_blend_state(&color_blend)
        .dynamic_state(&dynamic_state)
        .layout(pipeline_layout)
        .render_pass(render_pass)
        .subpass(0);

    let pipeline = unsafe {
        device
            .create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
            .map_err(|(_, e)| anyhow!("failed to create rubber graphics pipeline: {e}"))?[0]
    };

    unsafe {
        device.destroy_shader_module(vert, None);
        device.destroy_shader_module(frag, None);
    }

    Ok((pipeline_layout, pipeline))
}

fn create_shader_module(device: &ash::Device, spv: &[u8]) -> Result<vk::ShaderModule> {
    let mut code = Vec::with_capacity(spv.len() / 4);
    for chunk in spv.chunks_exact(4) {
        code.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    let info = vk::ShaderModuleCreateInfo::default().code(&code);
    Ok(unsafe { device.create_shader_module(&info, None)? })
}

struct LoadedBallMesh {
    vertices: Vec<MeshVertex>,
    indices: Vec<u32>,
    base_color: Vec<u8>,
}

fn create_ball_mesh(
    device: &ash::Device,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    command_pool: vk::CommandPool,
    queue: vk::Queue,
    glb: &[u8],
) -> Result<BallMesh> {
    let mesh = load_ball_mesh(glb)?;

    let vertex_size = (mesh.vertices.len() * std::mem::size_of::<MeshVertex>()) as vk::DeviceSize;
    let vertex_buffer = create_host_buffer(
        device,
        mem_props,
        vertex_size,
        vk::BufferUsageFlags::VERTEX_BUFFER,
    )?;
    unsafe {
        std::ptr::copy_nonoverlapping(
            mesh.vertices.as_ptr() as *const u8,
            vertex_buffer.mapped,
            vertex_size as usize,
        );
    }

    let index_size = (mesh.indices.len() * std::mem::size_of::<u32>()) as vk::DeviceSize;
    let index_buffer = create_host_buffer(
        device,
        mem_props,
        index_size,
        vk::BufferUsageFlags::INDEX_BUFFER,
    )?;
    unsafe {
        std::ptr::copy_nonoverlapping(
            mesh.indices.as_ptr() as *const u8,
            index_buffer.mapped,
            index_size as usize,
        );
    }

    let texture = create_texture(device, mem_props, command_pool, queue, &mesh.base_color)?;
    log::info!(
        "loaded GLB ball mesh: {} vertices, {} indices",
        mesh.vertices.len(),
        mesh.indices.len()
    );

    Ok(BallMesh {
        vertex_buffer,
        index_buffer,
        index_count: mesh.indices.len() as u32,
        texture,
    })
}

fn load_ball_mesh(glb: &[u8]) -> Result<LoadedBallMesh> {
    if glb.len() < 20 || &glb[0..4] != b"glTF" {
        return Err(anyhow!("ball asset is not a GLB file"));
    }

    let mut offset = 12usize;
    let mut json_chunk = None;
    let mut bin_chunk = None;
    while offset + 8 <= glb.len() {
        let len = u32::from_le_bytes(glb[offset..offset + 4].try_into().unwrap()) as usize;
        let kind = u32::from_le_bytes(glb[offset + 4..offset + 8].try_into().unwrap());
        offset += 8;
        if offset + len > glb.len() {
            return Err(anyhow!("GLB chunk exceeds file length"));
        }
        match kind {
            0x4E4F_534A => json_chunk = Some(&glb[offset..offset + len]),
            0x004E_4942 => bin_chunk = Some(&glb[offset..offset + len]),
            _ => {}
        }
        offset += len;
    }

    let json = serde_json::from_slice::<serde_json::Value>(
        json_chunk.ok_or_else(|| anyhow!("GLB missing JSON chunk"))?,
    )
    .context("failed to parse GLB JSON")?;
    let bin = bin_chunk.ok_or_else(|| anyhow!("GLB missing BIN chunk"))?;

    let primitive = &json["meshes"][0]["primitives"][0];
    let position_accessor = value_usize(&primitive["attributes"]["POSITION"])?;
    let normal_accessor = value_usize(&primitive["attributes"]["NORMAL"])?;
    let uv_accessor = value_usize(&primitive["attributes"]["TEXCOORD_0"])?;
    let index_accessor = value_usize(&primitive["indices"])?;

    let positions = read_vec3_accessor(&json, bin, position_accessor)?;
    let normals = read_vec3_accessor(&json, bin, normal_accessor)?;
    let uvs = read_vec2_accessor(&json, bin, uv_accessor)?;
    let indices = read_index_accessor(&json, bin, index_accessor)?;
    if positions.len() != normals.len() || positions.len() != uvs.len() {
        return Err(anyhow!("GLB mesh attribute counts do not match"));
    }

    let max_extent = positions
        .iter()
        .flat_map(|p| p.iter())
        .map(|v| v.abs())
        .fold(0.0_f32, f32::max)
        .max(0.001);
    let vertices = positions
        .into_iter()
        .zip(normals)
        .zip(uvs)
        .map(|((pos, normal), uv)| MeshVertex {
            pos: [
                pos[0] / max_extent,
                pos[1] / max_extent,
                pos[2] / max_extent,
            ],
            normal,
            uv,
        })
        .collect();

    let material_index = value_usize(&primitive["material"])?;
    let texture_index = value_usize(
        &json["materials"][material_index]["pbrMetallicRoughness"]["baseColorTexture"]["index"],
    )?;
    let image_index = value_usize(&json["textures"][texture_index]["source"])?;
    let base_color =
        read_image_bytes(&json, bin, image_index).unwrap_or_else(|_| SOCCER_TEXTURE_PNG.to_vec());

    Ok(LoadedBallMesh {
        vertices,
        indices,
        base_color,
    })
}

fn value_usize(value: &serde_json::Value) -> Result<usize> {
    value
        .as_u64()
        .map(|v| v as usize)
        .ok_or_else(|| anyhow!("expected unsigned GLB index, got {value}"))
}

fn read_vec3_accessor(
    json: &serde_json::Value,
    bin: &[u8],
    accessor_index: usize,
) -> Result<Vec<[f32; 3]>> {
    let values = read_f32_accessor(json, bin, accessor_index, 3)?;
    Ok(values.chunks_exact(3).map(|v| [v[0], v[1], v[2]]).collect())
}

fn read_vec2_accessor(
    json: &serde_json::Value,
    bin: &[u8],
    accessor_index: usize,
) -> Result<Vec<[f32; 2]>> {
    let values = read_f32_accessor(json, bin, accessor_index, 2)?;
    Ok(values.chunks_exact(2).map(|v| [v[0], v[1]]).collect())
}

fn read_f32_accessor(
    json: &serde_json::Value,
    bin: &[u8],
    accessor_index: usize,
    components: usize,
) -> Result<Vec<f32>> {
    let accessor = &json["accessors"][accessor_index];
    if value_usize(&accessor["componentType"])? != 5126 {
        return Err(anyhow!("GLB accessor {accessor_index} is not f32"));
    }
    let count = value_usize(&accessor["count"])?;
    let view_index = value_usize(&accessor["bufferView"])?;
    let (start, stride) = accessor_view(json, accessor, view_index, components * 4)?;
    let mut out = Vec::with_capacity(count * components);
    for i in 0..count {
        let base = start + i * stride;
        for c in 0..components {
            let off = base + c * 4;
            out.push(read_f32(bin, off)?);
        }
    }
    Ok(out)
}

fn read_index_accessor(
    json: &serde_json::Value,
    bin: &[u8],
    accessor_index: usize,
) -> Result<Vec<u32>> {
    let accessor = &json["accessors"][accessor_index];
    let count = value_usize(&accessor["count"])?;
    let component_type = value_usize(&accessor["componentType"])?;
    let component_size = match component_type {
        5121 => 1,
        5123 => 2,
        5125 => 4,
        _ => {
            return Err(anyhow!(
                "unsupported GLB index component type {component_type}"
            ))
        }
    };
    let view_index = value_usize(&accessor["bufferView"])?;
    let (start, stride) = accessor_view(json, accessor, view_index, component_size)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let off = start + i * stride;
        let index = match component_type {
            5121 => *bin
                .get(off)
                .ok_or_else(|| anyhow!("GLB u8 index out of bounds"))? as u32,
            5123 => {
                let bytes = bin
                    .get(off..off + 2)
                    .ok_or_else(|| anyhow!("GLB u16 index out of bounds"))?;
                u16::from_le_bytes(bytes.try_into().unwrap()) as u32
            }
            5125 => {
                let bytes = bin
                    .get(off..off + 4)
                    .ok_or_else(|| anyhow!("GLB u32 index out of bounds"))?;
                u32::from_le_bytes(bytes.try_into().unwrap())
            }
            _ => unreachable!(),
        };
        out.push(index);
    }
    Ok(out)
}

fn read_image_bytes(json: &serde_json::Value, bin: &[u8], image_index: usize) -> Result<Vec<u8>> {
    let view_index = value_usize(&json["images"][image_index]["bufferView"])?;
    let view = &json["bufferViews"][view_index];
    let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
    let len = value_usize(&view["byteLength"])?;
    Ok(bin
        .get(start..start + len)
        .ok_or_else(|| anyhow!("GLB image buffer view out of bounds"))?
        .to_vec())
}

fn accessor_view(
    json: &serde_json::Value,
    accessor: &serde_json::Value,
    view_index: usize,
    tight_stride: usize,
) -> Result<(usize, usize)> {
    let view = &json["bufferViews"][view_index];
    let view_offset = view["byteOffset"].as_u64().unwrap_or(0) as usize;
    let accessor_offset = accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
    let stride = view["byteStride"]
        .as_u64()
        .map(|v| v as usize)
        .unwrap_or(tight_stride);
    Ok((view_offset + accessor_offset, stride))
}

fn read_f32(bytes: &[u8], offset: usize) -> Result<f32> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| anyhow!("GLB f32 accessor out of bounds"))?;
    Ok(f32::from_le_bytes(slice.try_into().unwrap()))
}

fn create_texture(
    device: &ash::Device,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    command_pool: vk::CommandPool,
    queue: vk::Queue,
    bytes: &[u8],
) -> Result<Texture> {
    let image = image::load_from_memory(bytes)
        .context("failed to decode ball texture")?
        .to_rgba8();
    let (width, height) = image.dimensions();
    let pixels = image.into_raw();
    let size = pixels.len() as vk::DeviceSize;

    let staging = create_host_buffer(device, mem_props, size, vk::BufferUsageFlags::TRANSFER_SRC)?;
    unsafe {
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), staging.mapped, pixels.len());
    }

    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_SRGB)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);
    let image = unsafe { device.create_image(&image_info, None)? };
    let reqs = unsafe { device.get_image_memory_requirements(image) };
    let mem_type = find_memory_type(
        mem_props,
        reqs.memory_type_bits,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )?;
    let alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(reqs.size)
        .memory_type_index(mem_type);
    let memory = unsafe { device.allocate_memory(&alloc, None)? };
    unsafe { device.bind_image_memory(image, memory, 0)? };

    copy_buffer_to_image(
        device,
        command_pool,
        queue,
        staging.buffer,
        image,
        width,
        height,
    )?;
    destroy_buffer(device, &staging);

    let view_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_SRGB)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });
    let view = unsafe { device.create_image_view(&view_info, None)? };
    let sampler_info = vk::SamplerCreateInfo::default()
        .mag_filter(vk::Filter::LINEAR)
        .min_filter(vk::Filter::LINEAR)
        .address_mode_u(vk::SamplerAddressMode::REPEAT)
        .address_mode_v(vk::SamplerAddressMode::REPEAT)
        .address_mode_w(vk::SamplerAddressMode::REPEAT)
        .anisotropy_enable(false)
        .max_anisotropy(1.0)
        .border_color(vk::BorderColor::INT_OPAQUE_BLACK)
        .unnormalized_coordinates(false)
        .compare_enable(false)
        .mipmap_mode(vk::SamplerMipmapMode::LINEAR);
    let sampler = unsafe { device.create_sampler(&sampler_info, None)? };

    Ok(Texture {
        image,
        memory,
        view,
        sampler,
    })
}

fn create_texture_descriptor(
    device: &ash::Device,
    layout: vk::DescriptorSetLayout,
    texture: &Texture,
) -> Result<(vk::DescriptorPool, vk::DescriptorSet)> {
    let pool_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .descriptor_count(1);
    let pool_sizes = [pool_size];
    let pool_info = vk::DescriptorPoolCreateInfo::default()
        .pool_sizes(&pool_sizes)
        .max_sets(1);
    let pool = unsafe { device.create_descriptor_pool(&pool_info, None)? };

    let layouts = [layout];
    let alloc_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(pool)
        .set_layouts(&layouts);
    let set = unsafe { device.allocate_descriptor_sets(&alloc_info)?[0] };

    let image_info = vk::DescriptorImageInfo::default()
        .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .image_view(texture.view)
        .sampler(texture.sampler);
    let image_infos = [image_info];
    let write = vk::WriteDescriptorSet::default()
        .dst_set(set)
        .dst_binding(0)
        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .image_info(&image_infos);
    unsafe { device.update_descriptor_sets(&[write], &[]) };

    Ok((pool, set))
}

fn create_host_buffer(
    device: &ash::Device,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    size: vk::DeviceSize,
    usage: vk::BufferUsageFlags,
) -> Result<Buffer> {
    let info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let buffer = unsafe { device.create_buffer(&info, None)? };
    let reqs = unsafe { device.get_buffer_memory_requirements(buffer) };

    let flags = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
    let mem_type = find_memory_type(mem_props, reqs.memory_type_bits, flags)?;

    let alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(reqs.size)
        .memory_type_index(mem_type);
    let memory = unsafe { device.allocate_memory(&alloc, None)? };
    unsafe { device.bind_buffer_memory(buffer, memory, 0)? };
    let mapped =
        unsafe { device.map_memory(memory, 0, reqs.size, vk::MemoryMapFlags::empty())? as *mut u8 };

    Ok(Buffer {
        buffer,
        memory,
        mapped,
    })
}

fn destroy_buffer(device: &ash::Device, buffer: &Buffer) {
    unsafe {
        device.unmap_memory(buffer.memory);
        device.destroy_buffer(buffer.buffer, None);
        device.free_memory(buffer.memory, None);
    }
}

fn destroy_texture(device: &ash::Device, texture: &Texture) {
    unsafe {
        device.destroy_sampler(texture.sampler, None);
        device.destroy_image_view(texture.view, None);
        device.destroy_image(texture.image, None);
        device.free_memory(texture.memory, None);
    }
}

fn find_memory_type(
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    bits: u32,
    flags: vk::MemoryPropertyFlags,
) -> Result<u32> {
    (0..mem_props.memory_type_count)
        .find(|&i| {
            (bits & (1 << i)) != 0
                && mem_props.memory_types[i as usize]
                    .property_flags
                    .contains(flags)
        })
        .ok_or_else(|| anyhow!("no suitable memory type for {flags:?}"))
}

fn copy_buffer_to_image(
    device: &ash::Device,
    command_pool: vk::CommandPool,
    queue: vk::Queue,
    buffer: vk::Buffer,
    image: vk::Image,
    width: u32,
    height: u32,
) -> Result<()> {
    let cmd = begin_one_time_commands(device, command_pool)?;

    transition_image_layout(
        device,
        cmd,
        image,
        vk::ImageLayout::UNDEFINED,
        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
    );

    let region = vk::BufferImageCopy::default()
        .image_subresource(vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        })
        .image_extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        });
    unsafe {
        device.cmd_copy_buffer_to_image(
            cmd,
            buffer,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &[region],
        );
    }

    transition_image_layout(
        device,
        cmd,
        image,
        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
    );

    end_one_time_commands(device, command_pool, queue, cmd)
}

fn transition_image_layout(
    device: &ash::Device,
    cmd: vk::CommandBuffer,
    image: vk::Image,
    old: vk::ImageLayout,
    new: vk::ImageLayout,
) {
    let (src_access, dst_access, src_stage, dst_stage) = match (old, new) {
        (vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL) => (
            vk::AccessFlags::empty(),
            vk::AccessFlags::TRANSFER_WRITE,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
        ),
        (vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL) => (
            vk::AccessFlags::TRANSFER_WRITE,
            vk::AccessFlags::SHADER_READ,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
        ),
        _ => (
            vk::AccessFlags::empty(),
            vk::AccessFlags::empty(),
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
        ),
    };

    let barrier = vk::ImageMemoryBarrier::default()
        .old_layout(old)
        .new_layout(new)
        .src_access_mask(src_access)
        .dst_access_mask(dst_access)
        .image(image)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });
    unsafe {
        device.cmd_pipeline_barrier(
            cmd,
            src_stage,
            dst_stage,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier],
        );
    }
}

fn begin_one_time_commands(
    device: &ash::Device,
    command_pool: vk::CommandPool,
) -> Result<vk::CommandBuffer> {
    let alloc = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let cmd = unsafe { device.allocate_command_buffers(&alloc)?[0] };
    let begin =
        vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    unsafe { device.begin_command_buffer(cmd, &begin)? };
    Ok(cmd)
}

fn end_one_time_commands(
    device: &ash::Device,
    command_pool: vk::CommandPool,
    queue: vk::Queue,
    cmd: vk::CommandBuffer,
) -> Result<()> {
    unsafe {
        device.end_command_buffer(cmd)?;
        let cmd_buffers = [cmd];
        let submit = vk::SubmitInfo::default().command_buffers(&cmd_buffers);
        device.queue_submit(queue, &[submit], vk::Fence::null())?;
        device.queue_wait_idle(queue)?;
        device.free_command_buffers(command_pool, &[cmd]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_embedded_glb_ball_mesh() {
        let mesh = load_ball_mesh(SOCCER_GLB).expect("embedded GLB mesh should load");

        assert_eq!(mesh.vertices.len(), 6_944);
        assert_eq!(mesh.indices.len(), 24_924);
        assert!(
            mesh.base_color.starts_with(&[0xFF, 0xD8, 0xFF]),
            "base color should be the embedded JPEG texture"
        );
    }

    #[test]
    fn builds_rubber_band_tube_with_joint_bulbs() {
        let mut mesh = RubberBandMesh::default();
        let path = [
            Vec2::new(120.0, 40.0),
            Vec2::new(160.0, 140.0),
            Vec2::new(220.0, 260.0),
        ];
        let joints = [path[0], path[1], path[2]];

        mesh.rebuild(
            &path,
            &joints,
            Vec4::new(0.1, 0.45, 1.0, 1.0),
            Vec4::new(0.75, 0.92, 1.0, 1.0),
            7.0,
        );

        assert!(!mesh.vertices.is_empty());
        assert!(!mesh.indices.is_empty());
        assert!(mesh
            .indices
            .iter()
            .all(|&index| index < mesh.vertices.len() as u32));
        assert!(mesh.vertices.iter().any(|vertex| vertex.rubber[1] > 0.95));
    }
}
