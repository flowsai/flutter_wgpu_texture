#![cfg(target_os = "linux")]

use std::ffi::CStr;
use std::sync::{Arc, OnceLock};

use ash::ext::{external_memory_dma_buf, image_drm_format_modifier};
use ash::khr::external_memory_fd;
use ash::vk;
use wgpu::hal::api::Vulkan;

const DRM_FORMAT_ARGB8888: i32 = 0x34325241;

static DMA_BUF_SUPPORTED: OnceLock<bool> = OnceLock::new();

fn diag(args: std::fmt::Arguments) {
    eprintln!("[flutter_wgpu_texture][dma-buf] {args}");
}

fn has_extension(extensions: &[&'static CStr], needle: &'static CStr) -> bool {
    extensions.contains(&needle)
}

fn required_dma_buf_extensions_present(extensions: &[&'static CStr]) -> bool {
    has_extension(extensions, external_memory_fd::NAME)
        && has_extension(extensions, external_memory_dma_buf::NAME)
        && has_extension(extensions, image_drm_format_modifier::NAME)
}

fn extension_name_to_string(name: &[std::os::raw::c_char]) -> String {
    unsafe { CStr::from_ptr(name.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

fn pick_memory_type_index(
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    memory_type_bits: u32,
) -> Option<u32> {
    let count = memory_properties.memory_type_count as usize;
    let mut preferred = None;
    for index in 0..count {
        let bit = 1u32 << index;
        if memory_type_bits & bit == 0 {
            continue;
        }
        let flags = memory_properties.memory_types[index].property_flags;
        if flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL) {
            return Some(index as u32);
        }
        if preferred.is_none() {
            preferred = Some(index as u32);
        }
    }
    preferred
}

#[derive(Clone, Debug)]
pub(crate) struct DmaBufInfo {
    pub(crate) fd: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) stride: i32,
    pub(crate) offset: i32,
    pub(crate) fourcc: i32,
    pub(crate) modifier: u64,
}

pub(crate) struct OwnedDmaBufImage {
    raw_device: ash::Device,
    memory: vk::DeviceMemory,
    image: vk::Image,
    width: u32,
    height: u32,
    stride: i32,
    offset: i32,
    fourcc: i32,
    modifier: u64,
}

fn query_drm_format_modifiers(
    raw_instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
) -> Result<Vec<vk::DrmFormatModifierPropertiesEXT>, String> {
    let mut modifier_list = vk::DrmFormatModifierPropertiesListEXT::default();
    let mut format_properties = vk::FormatProperties2::default().push_next(&mut modifier_list);
    unsafe {
        raw_instance.get_physical_device_format_properties2(
            physical_device,
            vk::Format::B8G8R8A8_UNORM,
            &mut format_properties,
        );
    }

    let count = modifier_list.drm_format_modifier_count as usize;
    if count == 0 {
        return Err("No DRM format modifiers reported for B8G8R8A8_UNORM".to_string());
    }

    let mut modifiers = vec![vk::DrmFormatModifierPropertiesEXT::default(); count];
    let mut modifier_list = vk::DrmFormatModifierPropertiesListEXT::default()
        .drm_format_modifier_properties(&mut modifiers);
    let mut format_properties = vk::FormatProperties2::default().push_next(&mut modifier_list);
    unsafe {
        raw_instance.get_physical_device_format_properties2(
            physical_device,
            vk::Format::B8G8R8A8_UNORM,
            &mut format_properties,
        );
    }

    modifiers.retain(|modifier| {
        let features = modifier.drm_format_modifier_tiling_features;
        features.contains(vk::FormatFeatureFlags::TRANSFER_DST)
            && features.contains(vk::FormatFeatureFlags::SAMPLED_IMAGE)
    });

    if modifiers.is_empty() {
        return Err(
            "No DRM format modifier supports TRANSFER_DST|SAMPLED_IMAGE for B8G8R8A8_UNORM"
                .to_string(),
        );
    }

    modifiers.sort_by_key(|modifier| {
        if modifier.drm_format_modifier == 0 {
            1
        } else {
            0
        }
    });
    Ok(modifiers)
}

impl OwnedDmaBufImage {
    fn export(&self, raw_instance: &ash::Instance) -> Result<DmaBufInfo, String> {
        let external_memory_fd = external_memory_fd::Device::new(raw_instance, &self.raw_device);
        let get_fd_info = vk::MemoryGetFdInfoKHR::default()
            .memory(self.memory)
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let fd = unsafe { external_memory_fd.get_memory_fd(&get_fd_info) }
            .map_err(|err| format!("vkGetMemoryFdKHR failed: {err:?}"))?;
        Ok(DmaBufInfo {
            fd,
            width: self.width,
            height: self.height,
            stride: self.stride,
            offset: self.offset,
            fourcc: self.fourcc,
            modifier: self.modifier,
        })
    }
}

impl Drop for OwnedDmaBufImage {
    fn drop(&mut self) {
        unsafe {
            self.raw_device.destroy_image(self.image, None);
            self.raw_device.free_memory(self.memory, None);
        }
    }
}

pub(crate) fn dma_buf_supported(device: &wgpu::Device) -> bool {
    *DMA_BUF_SUPPORTED.get_or_init(|| unsafe {
        let Some(hal_device) = device.as_hal::<Vulkan>() else {
            diag(format_args!("wgpu HAL Vulkan device unavailable"));
            return false;
        };

        let enabled_extensions = hal_device.enabled_device_extensions();
        let has_enabled_external_memory_fd =
            has_extension(enabled_extensions, external_memory_fd::NAME);
        let has_enabled_dmabuf = has_extension(enabled_extensions, external_memory_dma_buf::NAME);
        let has_enabled_drm_modifier =
            has_extension(enabled_extensions, image_drm_format_modifier::NAME);

        let raw_instance = hal_device.shared_instance().raw_instance();
        let physical_device = hal_device.raw_physical_device();
        let available_extensions =
            match raw_instance.enumerate_device_extension_properties(physical_device) {
                Ok(props) => props,
                Err(err) => {
                    diag(format_args!(
                        "enumerate_device_extension_properties failed: {err:?}"
                    ));
                    return false;
                }
            };

        let has_available_external_memory_fd = available_extensions.iter().any(|ext| {
            extension_name_to_string(&ext.extension_name)
                == external_memory_fd::NAME.to_string_lossy()
        });
        let has_available_dmabuf = available_extensions.iter().any(|ext| {
            extension_name_to_string(&ext.extension_name)
                == external_memory_dma_buf::NAME.to_string_lossy()
        });
        let has_available_drm_modifier = available_extensions.iter().any(|ext| {
            extension_name_to_string(&ext.extension_name)
                == image_drm_format_modifier::NAME.to_string_lossy()
        });

        diag(format_args!(
            "device ext VK_KHR_external_memory_fd: available={} enabled={}",
            has_available_external_memory_fd, has_enabled_external_memory_fd
        ));
        diag(format_args!(
            "device ext VK_EXT_external_memory_dma_buf: available={} enabled={}",
            has_available_dmabuf, has_enabled_dmabuf
        ));
        diag(format_args!(
            "device ext VK_EXT_image_drm_format_modifier: available={} enabled={}",
            has_available_drm_modifier, has_enabled_drm_modifier
        ));

        if !(has_enabled_external_memory_fd && has_enabled_dmabuf && has_enabled_drm_modifier) {
            diag(format_args!(
                "export unsupported: required Vulkan device extensions are not enabled"
            ));
            return false;
        }

        let modifiers = match query_drm_format_modifiers(raw_instance, physical_device) {
            Ok(modifiers) => modifiers,
            Err(err) => {
                diag(format_args!("export unsupported: {err}"));
                return false;
            }
        };
        let modifier = modifiers[0].drm_format_modifier;
        let mut modifier_info = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default()
            .drm_format_modifier(modifier)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let mut external_image_info = vk::PhysicalDeviceExternalImageFormatInfo::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let mut external_image_format_properties = vk::ExternalImageFormatProperties::default();
        let mut image_format_properties =
            vk::ImageFormatProperties2::default().push_next(&mut external_image_format_properties);
        let image_format_info = vk::PhysicalDeviceImageFormatInfo2::default()
            .format(vk::Format::B8G8R8A8_UNORM)
            .ty(vk::ImageType::TYPE_2D)
            .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
            .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
            .flags(vk::ImageCreateFlags::empty())
            .push_next(&mut external_image_info)
            .push_next(&mut modifier_info);

        match raw_instance.get_physical_device_image_format_properties2(
            physical_device,
            &image_format_info,
            &mut image_format_properties,
        ) {
            Ok(()) => {
                let props = external_image_format_properties.external_memory_properties;
                let exportable = props
                    .external_memory_features
                    .contains(vk::ExternalMemoryFeatureFlags::EXPORTABLE);
                diag(format_args!(
                    "format caps modifier=0x{:x} external_features={:?} compatible={:?} export_from_imported={:?}",
                    modifier,
                    props.external_memory_features,
                    props.compatible_handle_types,
                    props.export_from_imported_handle_types
                ));
                if !exportable {
                    diag(format_args!(
                        "export unsupported: Vulkan DRM modifier image is not exportable as DMA-BUF"
                    ));
                }
                exportable
            }
            Err(err) => {
                diag(format_args!(
                    "get_physical_device_image_format_properties2 failed: {err:?}"
                ));
                false
            }
        }
    })
}

pub(crate) fn create_shared_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> Result<(wgpu::Texture, Arc<OwnedDmaBufImage>), String> {
    if width == 0 || height == 0 {
        return Err("linux dma-buf present target requires non-zero size".to_string());
    }

    unsafe {
        let Some(hal_device) = device.as_hal::<Vulkan>() else {
            return Err("wgpu Vulkan backend unavailable".to_string());
        };
        if !required_dma_buf_extensions_present(hal_device.enabled_device_extensions()) {
            return Err(
                "Vulkan device missing VK_KHR_external_memory_fd or VK_EXT_external_memory_dma_buf"
                    .to_string(),
            );
        }

        let raw_device = hal_device.raw_device().clone();
        let raw_instance = hal_device.shared_instance().raw_instance().clone();
        let physical_device = hal_device.raw_physical_device();
        let modifiers = query_drm_format_modifiers(&raw_instance, physical_device)?;
        let modifier_values = modifiers
            .iter()
            .map(|modifier| modifier.drm_format_modifier)
            .collect::<Vec<_>>();
        let mut drm_modifier_list =
            vk::ImageDrmFormatModifierListCreateInfoEXT::default().drm_format_modifiers(&modifier_values);

        let mut external_image_info = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::B8G8R8A8_UNORM)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
            .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .push_next(&mut drm_modifier_list)
            .push_next(&mut external_image_info);

        let image = raw_device
            .create_image(&image_info, None)
            .map_err(|err| format!("vkCreateImage failed: {err:?}"))?;

        let memory_requirements = raw_device.get_image_memory_requirements(image);
        let memory_properties =
            raw_instance.get_physical_device_memory_properties(physical_device);
        let memory_type_index =
            pick_memory_type_index(&memory_properties, memory_requirements.memory_type_bits)
                .ok_or_else(|| "No compatible Vulkan memory type for DMA-BUF image".to_string())?;

        let mut export_alloc_info = vk::ExportMemoryAllocateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let mut dedicated_alloc_info = vk::MemoryDedicatedAllocateInfo::default().image(image);
        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(memory_requirements.size)
            .memory_type_index(memory_type_index)
            .push_next(&mut export_alloc_info)
            .push_next(&mut dedicated_alloc_info);

        let memory = match raw_device.allocate_memory(&alloc_info, None) {
            Ok(memory) => memory,
            Err(err) => {
                raw_device.destroy_image(image, None);
                return Err(format!("vkAllocateMemory failed: {err:?}"));
            }
        };

        if let Err(err) = raw_device.bind_image_memory(image, memory, 0) {
            raw_device.free_memory(memory, None);
            raw_device.destroy_image(image, None);
            return Err(format!("vkBindImageMemory failed: {err:?}"));
        }

        let drm_modifier_ext =
            image_drm_format_modifier::Device::new(&raw_instance, &raw_device);
        let mut modifier_properties = vk::ImageDrmFormatModifierPropertiesEXT::default();
        drm_modifier_ext
            .get_image_drm_format_modifier_properties(image, &mut modifier_properties)
            .map_err(|err| format!("vkGetImageDrmFormatModifierPropertiesEXT failed: {err:?}"))?;

        let subresource = vk::ImageSubresource {
            aspect_mask: vk::ImageAspectFlags::MEMORY_PLANE_0_EXT,
            mip_level: 0,
            array_layer: 0,
        };
        // NOTE: original used COLOR; for DRM_FORMAT_MODIFIER images the layout must be
        // queried per memory plane. MEMORY_PLANE_0_EXT is correct for single-plane.
        let layout = raw_device.get_image_subresource_layout(image, subresource);

        let owned = Arc::new(OwnedDmaBufImage {
            raw_device: raw_device.clone(),
            memory,
            image,
            width,
            height,
            stride: layout.row_pitch as i32,
            offset: layout.offset as i32,
            fourcc: DRM_FORMAT_ARGB8888,
            modifier: modifier_properties.drm_format_modifier,
        });

        let hal_desc = wgpu::hal::TextureDescriptor {
            label: Some("flutter_wgpu_texture present texture (linux dma-buf shared)"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUses::COPY_DST | wgpu::TextureUses::RESOURCE,
            memory_flags: wgpu::hal::MemoryFlags::empty(),
            view_formats: Vec::new(),
        };

        // Keep the OwnedDmaBufImage alive for as long as the wgpu texture exists.
        let keep_alive = owned.clone();
        let hal_texture = hal_device.texture_from_raw(
            image,
            &hal_desc,
            Some(Box::new(move || {
                drop(keep_alive);
            })),
            wgpu::hal::vulkan::TextureMemory::External,
        );

        let wgpu_desc = wgpu::TextureDescriptor {
            label: Some("flutter_wgpu_texture present texture (linux dma-buf shared)"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        let texture = device.create_texture_from_hal::<Vulkan>(hal_texture, &wgpu_desc);

        diag(format_args!(
            "Linux DMA-BUF shared image created: {}x{} stride={} offset={} modifier=0x{:x}",
            width, height, owned.stride, owned.offset, owned.modifier,
        ));

        Ok((texture, owned))
    }
}

pub(crate) fn export_dmabuf(
    device: &wgpu::Device,
    image: &Arc<OwnedDmaBufImage>,
) -> Result<DmaBufInfo, String> {
    unsafe {
        let Some(hal_device) = device.as_hal::<Vulkan>() else {
            return Err("wgpu Vulkan backend unavailable".to_string());
        };
        image.export(hal_device.shared_instance().raw_instance())
    }
}
