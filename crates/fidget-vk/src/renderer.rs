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
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// Per-instance vertex data. Layout must match `shaders/blob.vert`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Instance {
    pub center: [f32; 2],
    pub half: [f32; 2],
    pub color: [f32; 4],
    pub softness: f32,
    pub _pad: [f32; 3],
}

const MAX_INSTANCES: usize = 8192;
const FRAMES_IN_FLIGHT: usize = 2;

// Embedded compiled shaders (produced by build.rs).
const VERT_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/blob.vert.spv"));
const FRAG_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/blob.frag.spv"));

/// A GPU buffer plus its backing memory and (optional) persistent mapping.
struct Buffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    mapped: *mut u8,
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
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,

    command_pool: vk::CommandPool,
    command_buffers: Vec<vk::CommandBuffer>,
    quad_buffer: Buffer,
    instance_buffers: Vec<Buffer>,

    image_available: Vec<vk::Semaphore>,
    render_finished: Vec<vk::Semaphore>,
    in_flight: Vec<vk::Fence>,
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
        let (physical_device, queue_family) =
            pick_device(&instance, &surface_loader, surface)?;
        let mem_props =
            unsafe { instance.get_physical_device_memory_properties(physical_device) };

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

        // --- Pipeline ------------------------------------------------------
        let (pipeline_layout, pipeline) = create_pipeline(&device, render_pass)?;

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

        // --- Sync primitives ----------------------------------------------
        let mut image_available = Vec::new();
        let mut render_finished = Vec::new();
        let mut in_flight = Vec::new();
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
            pipeline_layout,
            pipeline,
            command_pool,
            command_buffers,
            quad_buffer,
            instance_buffers,
            image_available,
            render_finished,
            in_flight,
            current_frame: 0,
            clear_color: [0.02, 0.03, 0.06, 1.0],
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
        let surface_format = choose_surface_format(
            &self.surface_loader,
            self.physical_device,
            self.surface,
        )?;
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
    pub fn render(&mut self, instances: &[Instance]) -> Result<bool> {
        let frame = self.current_frame;
        let fence = self.in_flight[frame];
        unsafe {
            self.device.wait_for_fences(&[fence], true, u64::MAX)?;
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

        // Upload instances for this frame.
        let count = instances.len().min(MAX_INSTANCES);
        unsafe {
            std::ptr::copy_nonoverlapping(
                instances.as_ptr() as *const u8,
                self.instance_buffers[frame].mapped,
                count * std::mem::size_of::<Instance>(),
            );
        }

        self.record_command_buffer(frame, image_index as usize, count as u32)?;

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
        let present_result =
            unsafe { self.swapchain_loader.queue_present(self.queue, &present) };

        self.current_frame = (frame + 1) % FRAMES_IN_FLIGHT;

        match present_result {
            Ok(false) => Ok(true),
            Ok(true) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => Ok(false),
            Err(e) => Err(anyhow!("queue_present failed: {e}")),
        }
    }

    fn record_command_buffer(&self, frame: usize, image_index: usize, count: u32) -> Result<()> {
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

            let resolution = [self.swap.extent.width as f32, self.swap.extent.height as f32];
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
            destroy_buffer(&self.device, &self.quad_buffer);
            self.device.destroy_command_pool(self.command_pool, None);
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
    let formats =
        unsafe { surface_loader.get_physical_device_surface_formats(pd, surface)? };
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
    let caps =
        unsafe { surface_loader.get_physical_device_surface_capabilities(pd, surface)? };
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
            width: window_size.0.clamp(
                caps.min_image_extent.width,
                caps.max_image_extent.width,
            ),
            height: window_size.1.clamp(
                caps.min_image_extent.height,
                caps.max_image_extent.height,
            ),
        }
    };

    let mut image_count = caps.min_image_count + 1;
    if caps.max_image_count > 0 && image_count > caps.max_image_count {
        image_count = caps.max_image_count;
    }

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
        .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
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

fn create_pipeline(
    device: &ash::Device,
    render_pass: vk::RenderPass,
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

    // Additive blending (premultiplied source) for the glow look.
    let blend_attachment = vk::PipelineColorBlendAttachmentState::default()
        .blend_enable(true)
        .src_color_blend_factor(vk::BlendFactor::ONE)
        .dst_color_blend_factor(vk::BlendFactor::ONE)
        .color_blend_op(vk::BlendOp::ADD)
        .src_alpha_blend_factor(vk::BlendFactor::ONE)
        .dst_alpha_blend_factor(vk::BlendFactor::ONE)
        .alpha_blend_op(vk::BlendOp::ADD)
        .color_write_mask(vk::ColorComponentFlags::RGBA);
    let blend_attachments = [blend_attachment];
    let color_blend = vk::PipelineColorBlendStateCreateInfo::default()
        .attachments(&blend_attachments);

    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic_state =
        vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

    let push_range = vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::VERTEX)
        .offset(0)
        .size(std::mem::size_of::<[f32; 2]>() as u32);
    let push_ranges = [push_range];
    let layout_info =
        vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&push_ranges);
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

fn create_shader_module(device: &ash::Device, spv: &[u8]) -> Result<vk::ShaderModule> {
    let mut code = Vec::with_capacity(spv.len() / 4);
    for chunk in spv.chunks_exact(4) {
        code.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    let info = vk::ShaderModuleCreateInfo::default().code(&code);
    Ok(unsafe { device.create_shader_module(&info, None)? })
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
    let mem_type = (0..mem_props.memory_type_count)
        .find(|&i| {
            (reqs.memory_type_bits & (1 << i)) != 0
                && mem_props.memory_types[i as usize]
                    .property_flags
                    .contains(flags)
        })
        .ok_or_else(|| anyhow!("no host-visible memory type"))?;

    let alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(reqs.size)
        .memory_type_index(mem_type);
    let memory = unsafe { device.allocate_memory(&alloc, None)? };
    unsafe { device.bind_buffer_memory(buffer, memory, 0)? };
    let mapped = unsafe {
        device.map_memory(memory, 0, reqs.size, vk::MemoryMapFlags::empty())? as *mut u8
    };

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
