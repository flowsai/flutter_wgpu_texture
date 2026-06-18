//! Viewport management: the offscreen render target, camera, per-frame pump and
//! the cross-copy of the rendered image into the package's DMA-BUF texture.

pub mod camera;

use bevy::app::{AppLabel, SubApps};
use bevy::asset::{AssetId, RenderAssetUsages};
use bevy::camera::RenderTarget;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{PollType, TextureFormat};
use bevy::render::texture::GpuImage;
use bevy::render::RenderApp;

use crate::engine::device::shared_gpu;
use crate::level;
use camera::OrbitCamera;

/// Format of the offscreen render target. MUST match the wgpu format of the
/// package's DMA-BUF `shared_texture` (Bgra8Unorm) for copy_texture_to_texture.
pub(crate) const TARGET_FORMAT: TextureFormat = TextureFormat::Bgra8Unorm;

/// Per-viewport bookkeeping kept on the render thread.
pub(crate) struct Viewport {
    pub(crate) image: AssetId<Image>,
    pub(crate) camera: Entity,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

fn make_target_image(width: u32, height: u32) -> Image {
    let mut img = Image::new_target_texture(width.max(1), height.max(1), TARGET_FORMAT, None);
    // new_target_texture omits COPY_SRC; required for the cross-copy into shared_texture.
    img.texture_descriptor.usage |= wgpu::TextureUsages::COPY_SRC;
    img.asset_usage = RenderAssetUsages::RENDER_WORLD;
    img
}

pub(crate) fn spawn_viewport(
    sub_apps: &mut SubApps,
    width: u32,
    height: u32,
) -> (AssetId<Image>, Entity) {
    let world = sub_apps.main.world_mut();
    let img = make_target_image(width, height);
    let handle = world.resource_mut::<Assets<Image>>().add(img);
    let image_id = handle.id();
    // In this Bevy, RenderTarget is its own component (not Camera.target).
    world.init_resource::<OrbitCamera>();
    let cam_xf = world.resource::<OrbitCamera>().transform();
    let camera = world
        .spawn((Camera3d::default(), RenderTarget::Image(handle.into()), cam_xf))
        .id();
    (image_id, camera)
}

pub(crate) fn resize_viewport_image(
    sub_apps: &mut SubApps,
    image: AssetId<Image>,
    width: u32,
    height: u32,
) {
    let world = sub_apps.main.world_mut();
    let new_image = make_target_image(width, height);
    let mut images = world.resource_mut::<Assets<Image>>();
    let _ = images.insert(image, new_image);
}

/// Pump one frame and cross-copy the rendered image into the DMA-BUF `dst`.
pub(crate) fn render_one_frame(
    sub_apps: &mut SubApps,
    image: AssetId<Image>,
    dst: Option<&wgpu::Texture>,
    width: u32,
    height: u32,
) -> Result<bool, String> {
    level::ensure_default_scene(sub_apps);

    // Pump one full frame (Extract -> render-world Render schedule -> submit).
    sub_apps.update();

    let gpu = shared_gpu().ok_or_else(|| "shared gpu missing".to_string())?;

    // Wait for the GPU to finish this frame's submissions before copying.
    gpu.device
        .poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(|e| format!("device.poll failed: {e:?}"))?;

    let Some(dst) = dst else {
        return Ok(true);
    };

    // Reach into the render sub-app world for the rendered GpuImage texture.
    let render_app = sub_apps
        .sub_apps
        .get_mut(&RenderApp.intern())
        .ok_or_else(|| "RenderApp sub-app missing".to_string())?;
    let gpu_images = render_app
        .world()
        .get_resource::<RenderAssets<GpuImage>>()
        .ok_or_else(|| "RenderAssets<GpuImage> missing".to_string())?;
    let Some(gpu_image) = gpu_images.get(image) else {
        // Image not prepared yet (first frame). Not an error; just no copy.
        return Ok(false);
    };

    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("bevy->dmabuf cross copy"),
        });
    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &gpu_image.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyTextureInfo {
            texture: dst,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    gpu.device
        .poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(|e| format!("device.poll (copy) failed: {e:?}"))?;

    Ok(true)
}
