use std::sync::Arc;

#[cfg(any(target_os = "macos", target_os = "ios"))]
use std::ffi::c_void;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use metal::foreign_types::ForeignType;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use metal::MTLTextureType;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use wgpu_hal::{api::Metal, CopyExtent};
#[cfg(target_os = "windows")]
use wgpu_hal::api::Dx12;
#[cfg(target_os = "windows")]
use wgpu_hal::dx12;
#[cfg(target_os = "windows")]
use winapi::shared::{dxgiformat, dxgitype};
#[cfg(target_os = "windows")]
use winapi::Interface as _;
#[cfg(target_os = "windows")]
use winapi::um::{d3d12 as d3d12_ty, handleapi::CloseHandle, winnt};

#[cfg(target_os = "linux")]
use crate::linux_dma_buf::{create_shared_texture, OwnedDmaBufImage};

pub(crate) struct PresentTextureTarget {
    render_texture: wgpu::Texture,
    pub(crate) render_view: wgpu::TextureView,
    shared_texture: Option<wgpu::Texture>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    #[cfg(target_os = "linux")]
    dma_buf_image: Option<Arc<OwnedDmaBufImage>>,
    #[cfg(target_os = "windows")]
    dxgi_handle: Option<winnt::HANDLE>,
}

impl PresentTextureTarget {
    pub(crate) fn render_texture(&self) -> &wgpu::Texture {
        &self.render_texture
    }

    pub(crate) fn shared_texture(&self) -> Option<&wgpu::Texture> {
        self.shared_texture.as_ref()
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn dma_buf_image(&self) -> Option<&Arc<OwnedDmaBufImage>> {
        self.dma_buf_image.as_ref()
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn take_dxgi_handle(&mut self) -> Option<usize> {
        self.dxgi_handle.take().map(|handle| handle as usize)
    }
}

#[cfg(target_os = "windows")]
impl Drop for PresentTextureTarget {
    fn drop(&mut self) {
        if let Some(handle) = self.dxgi_handle.take() {
            unsafe {
                CloseHandle(handle);
            }
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) fn attach_present_texture(
    device: &wgpu::Device,
    mtl_texture_ptr: *mut c_void,
    width: u32,
    height: u32,
    _bytes_per_row: u32,
) -> Option<PresentTextureTarget> {
    if mtl_texture_ptr.is_null() || width == 0 || height == 0 {
        return None;
    }

    let raw_ptr = mtl_texture_ptr as *mut metal::MTLTexture;
    let raw_texture = unsafe { metal::Texture::from_ptr(raw_ptr) };
    let hal_texture = unsafe {
        wgpu_hal::metal::Device::texture_from_raw(
            raw_texture,
            wgpu::TextureFormat::Bgra8Unorm,
            MTLTextureType::D2,
            1,
            1,
            CopyExtent {
                width,
                height,
                depth: 1,
            },
        )
    };

    let desc = wgpu::TextureDescriptor {
        label: Some("flutter_wgpu_texture present texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    };

    let texture = unsafe { device.create_texture_from_hal::<Metal>(hal_texture, &desc) };
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Some(PresentTextureTarget {
        render_texture: texture,
        render_view: view,
        shared_texture: None,
        width,
        height,
        #[cfg(target_os = "linux")]
        dma_buf_image: None,
        #[cfg(target_os = "windows")]
        dxgi_handle: None,
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn create_linux_present_target(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> Result<PresentTextureTarget, String> {
    let (shared_texture, dma_buf_image) = create_shared_texture(device, width, height)?;
    let render_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("flutter_wgpu_texture present render"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let render_view = render_texture.create_view(&wgpu::TextureViewDescriptor::default());
    Ok(PresentTextureTarget {
        render_texture,
        render_view,
        shared_texture: Some(shared_texture),
        width,
        height,
        dma_buf_image: Some(dma_buf_image),
    })
}

#[cfg(target_os = "windows")]
pub(crate) fn create_dxgi_shared_present_target(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> Result<PresentTextureTarget, String> {
    if width == 0 || height == 0 {
        return Err("present target size must be > 0".to_string());
    }

    let (resource, shared_handle) = unsafe {
        device
            .as_hal::<Dx12, _, _>(|hal_device| {
                let Some(hal_device) = hal_device else {
                    return Err("wgpu: dx12 backend unavailable".to_string());
                };

                let raw_device = hal_device.raw_device();
                let mut resource = d3d12::ComPtr::<d3d12_ty::ID3D12Resource>::null();

                let heap_props = d3d12_ty::D3D12_HEAP_PROPERTIES {
                    Type: d3d12_ty::D3D12_HEAP_TYPE_DEFAULT,
                    CPUPageProperty: d3d12_ty::D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                    MemoryPoolPreference: d3d12_ty::D3D12_MEMORY_POOL_UNKNOWN,
                    CreationNodeMask: 0,
                    VisibleNodeMask: 0,
                };

                let desc = d3d12_ty::D3D12_RESOURCE_DESC {
                    Dimension: d3d12_ty::D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                    Alignment: 0,
                    Width: width as u64,
                    Height: height,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    Format: dxgiformat::DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: dxgitype::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Layout: d3d12_ty::D3D12_TEXTURE_LAYOUT_UNKNOWN,
                    Flags: d3d12_ty::D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET
                        | d3d12_ty::D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS,
                };

                let hr = raw_device.CreateCommittedResource(
                    &heap_props,
                    d3d12_ty::D3D12_HEAP_FLAG_SHARED,
                    &desc,
                    d3d12_ty::D3D12_RESOURCE_STATE_COMMON,
                    std::ptr::null(),
                    &d3d12_ty::ID3D12Resource::uuidof(),
                    resource.mut_void(),
                );
                if hr < 0 || resource.is_null() {
                    return Err(format!(
                        "dx12 CreateCommittedResource failed: 0x{hr:08X}"
                    ));
                }

                let mut handle: winnt::HANDLE = std::ptr::null_mut();
                let hr = raw_device.CreateSharedHandle(
                    resource.as_mut_ptr() as *mut _,
                    std::ptr::null(),
                    winnt::GENERIC_ALL,
                    std::ptr::null(),
                    &mut handle,
                );
                if hr < 0 || handle.is_null() {
                    return Err(format!("dx12 CreateSharedHandle failed: 0x{hr:08X}"));
                }

                Ok((resource, handle))
            })
            .ok_or_else(|| "wgpu: dx12 backend unavailable".to_string())?
    }?;

    let render_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("flutter_wgpu_texture present render"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let render_view = render_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let hal_texture = unsafe {
        dx12::Device::texture_from_raw(
            resource,
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureDimension::D2,
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            1,
            1,
        )
    };

    let desc = wgpu::TextureDescriptor {
        label: Some("flutter_wgpu_texture present shared dxgi"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    };
    let shared_texture = unsafe { device.create_texture_from_hal::<Dx12>(hal_texture, &desc) };

    Ok(PresentTextureTarget {
        render_texture,
        render_view,
        shared_texture: Some(shared_texture),
        width,
        height,
        #[cfg(target_os = "linux")]
        dma_buf_image: None,
        dxgi_handle: Some(shared_handle),
    })
}
