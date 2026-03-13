#include "include/flutter_wgpu_texture/flutter_wgpu_texture_plugin.h"

#include <flutter/method_channel.h>
#include <flutter/plugin_registrar_windows.h>
#include <flutter/standard_method_codec.h>
#include <flutter_texture_registrar.h>

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <Windows.h>

#include <algorithm>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <unordered_map>

extern "C" {
void* engine_create_present_dxgi_surface(uint64_t handle,
                                         uint32_t width,
                                         uint32_t height);
uint8_t engine_resize(uint64_t handle, uint32_t width, uint32_t height);
uint8_t engine_get_backend(uint64_t handle);
}

namespace {

constexpr char kChannelName[] = "flutter_wgpu_texture";

std::optional<int64_t> GetIntValue(const flutter::EncodableValue& value) {
  if (const auto* int32_value = std::get_if<int32_t>(&value)) {
    return *int32_value;
  }
  if (const auto* int64_value = std::get_if<int64_t>(&value)) {
    return *int64_value;
  }
  if (const auto* double_value = std::get_if<double>(&value)) {
    return static_cast<int64_t>(*double_value);
  }
  if (const auto* string_value = std::get_if<std::string>(&value)) {
    if (!string_value->empty()) {
      return std::stoll(*string_value);
    }
  }
  return std::nullopt;
}

int GetClampedInt(const flutter::EncodableMap& args,
                  const char* key,
                  int fallback,
                  int min_value,
                  int max_value) {
  const auto it = args.find(flutter::EncodableValue(key));
  if (it == args.end()) {
    return std::clamp(fallback, min_value, max_value);
  }
  const auto value = GetIntValue(it->second);
  if (!value.has_value()) {
    return std::clamp(fallback, min_value, max_value);
  }
  return std::clamp(static_cast<int>(*value), min_value, max_value);
}

std::string GetSurfaceId(const flutter::EncodableMap& args) {
  const auto it = args.find(flutter::EncodableValue("surfaceId"));
  if (it == args.end()) {
    return "default";
  }
  if (const auto* string_value = std::get_if<std::string>(&it->second)) {
    if (!string_value->empty()) {
      return *string_value;
    }
  }
  if (const auto value = GetIntValue(it->second); value.has_value()) {
    return std::to_string(*value);
  }
  return "default";
}

uint64_t GetHandle(const flutter::EncodableMap& args) {
  const auto it = args.find(flutter::EncodableValue("handle"));
  if (it == args.end()) {
    return 0;
  }
  return static_cast<uint64_t>(GetIntValue(it->second).value_or(0));
}

const char* BackendName(uint8_t backend) {
  switch (backend) {
    case 1:
      return "metal";
    case 2:
      return "dx12";
    case 3:
      return "vulkan";
    default:
      return "unknown";
  }
}

struct GpuSurfaceBinding {
  explicit GpuSurfaceBinding(void* handle, size_t width, size_t height)
      : shared_handle(handle) {
    descriptor.struct_size = sizeof(FlutterDesktopGpuSurfaceDescriptor);
    descriptor.handle = handle;
    descriptor.width = width;
    descriptor.height = height;
    descriptor.visible_width = width;
    descriptor.visible_height = height;
    descriptor.format = kFlutterDesktopPixelFormatBGRA8888;
    descriptor.release_callback = nullptr;
    descriptor.release_context = nullptr;
  }

  const FlutterDesktopGpuSurfaceDescriptor* GetDescriptor() const {
    return &descriptor;
  }

  static const FlutterDesktopGpuSurfaceDescriptor* Callback(size_t,
                                                            size_t,
                                                            void* user_data) {
    const auto* binding = static_cast<GpuSurfaceBinding*>(user_data);
    return binding ? binding->GetDescriptor() : nullptr;
  }

  void Update(void* handle, size_t width, size_t height) {
    if (shared_handle && shared_handle != handle) {
      CloseHandle(static_cast<HANDLE>(shared_handle));
    }
    shared_handle = handle;
    descriptor.handle = handle;
    descriptor.width = width;
    descriptor.height = height;
    descriptor.visible_width = width;
    descriptor.visible_height = height;
  }

  void* shared_handle = nullptr;
  FlutterDesktopGpuSurfaceDescriptor descriptor{};
};

void ReleaseBinding(void* user_data) {
  auto* keepalive = static_cast<std::shared_ptr<GpuSurfaceBinding>*>(user_data);
  if (keepalive && *keepalive) {
    if ((*keepalive)->shared_handle) {
      CloseHandle(static_cast<HANDLE>((*keepalive)->shared_handle));
      (*keepalive)->shared_handle = nullptr;
    }
  }
  delete keepalive;
}

struct SurfaceState {
  std::string surface_id;
  uint64_t handle = 0;
  int width = 1;
  int height = 1;
  int64_t texture_id = -1;
  std::shared_ptr<GpuSurfaceBinding> binding;
  std::mutex mutex;
};

class FlutterWgpuTexturePlugin : public flutter::Plugin {
 public:
  static void RegisterWithRegistrar(
      flutter::PluginRegistrarWindows* registrar,
      FlutterDesktopPluginRegistrarRef raw_registrar);

  explicit FlutterWgpuTexturePlugin(
      FlutterDesktopTextureRegistrarRef texture_registrar)
      : texture_registrar_(texture_registrar) {}

  ~FlutterWgpuTexturePlugin() override { DisposeAll(); }

 private:
  void HandleMethodCall(
      const flutter::MethodCall<flutter::EncodableValue>& method_call,
      std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> result);

  void HandleCreateSurface(
      const flutter::EncodableMap& args,
      std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> result);

  void HandleResizeSurface(
      const flutter::EncodableMap& args,
      std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> result);

  void HandleDisposeSurface(
      const flutter::EncodableMap& args,
      std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> result);

  void HandleMarkFrameAvailable(
      const flutter::EncodableMap& args,
      std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> result);

  std::shared_ptr<SurfaceState> GetOrCreateSurface(const std::string& surface_id) {
    std::lock_guard<std::mutex> lock(surfaces_mutex_);
    auto it = surfaces_.find(surface_id);
    if (it != surfaces_.end()) {
      return it->second;
    }
    auto surface = std::make_shared<SurfaceState>();
    surface->surface_id = surface_id;
    surfaces_[surface_id] = surface;
    return surface;
  }

  std::shared_ptr<SurfaceState> FindSurface(const std::string& surface_id) {
    std::lock_guard<std::mutex> lock(surfaces_mutex_);
    auto it = surfaces_.find(surface_id);
    return it == surfaces_.end() ? nullptr : it->second;
  }

  void RemoveSurface(const std::string& surface_id) {
    std::lock_guard<std::mutex> lock(surfaces_mutex_);
    surfaces_.erase(surface_id);
  }

  void UnregisterTextureLocked(const std::shared_ptr<SurfaceState>& surface) {
    if (!surface || surface->texture_id < 0) {
      return;
    }
    const auto texture_id = surface->texture_id;
    surface->texture_id = -1;
    auto binding = surface->binding;
    surface->binding.reset();
    if (!binding) {
      return;
    }
    auto* keepalive = new std::shared_ptr<GpuSurfaceBinding>(binding);
    FlutterDesktopTextureRegistrarUnregisterExternalTexture(
        texture_registrar_, texture_id, ReleaseBinding, keepalive);
  }

  void DisposeAll() {
    std::unordered_map<std::string, std::shared_ptr<SurfaceState>> surfaces;
    {
      std::lock_guard<std::mutex> lock(surfaces_mutex_);
      surfaces.swap(surfaces_);
    }
    for (const auto& entry : surfaces) {
      std::lock_guard<std::mutex> surface_lock(entry.second->mutex);
      UnregisterTextureLocked(entry.second);
    }
  }

  FlutterDesktopTextureRegistrarRef texture_registrar_;
  std::mutex surfaces_mutex_;
  std::unordered_map<std::string, std::shared_ptr<SurfaceState>> surfaces_;
};

void FlutterWgpuTexturePlugin::RegisterWithRegistrar(
    flutter::PluginRegistrarWindows* registrar,
    FlutterDesktopPluginRegistrarRef raw_registrar) {
  auto channel =
      std::make_unique<flutter::MethodChannel<flutter::EncodableValue>>(
          registrar->messenger(), kChannelName,
          &flutter::StandardMethodCodec::GetInstance());

  auto plugin = std::make_unique<FlutterWgpuTexturePlugin>(
      FlutterDesktopRegistrarGetTextureRegistrar(raw_registrar));

  channel->SetMethodCallHandler(
      [plugin_pointer = plugin.get()](const auto& call, auto result) {
        plugin_pointer->HandleMethodCall(call, std::move(result));
      });

  registrar->AddPlugin(std::move(plugin));
}

void FlutterWgpuTexturePlugin::HandleMethodCall(
    const flutter::MethodCall<flutter::EncodableValue>& method_call,
    std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> result) {
  const auto* args = std::get_if<flutter::EncodableMap>(method_call.arguments());
  const flutter::EncodableMap empty_args;
  const auto& resolved_args = args ? *args : empty_args;

  if (method_call.method_name() == "createSurface") {
    HandleCreateSurface(resolved_args, std::move(result));
  } else if (method_call.method_name() == "resizeSurface") {
    HandleResizeSurface(resolved_args, std::move(result));
  } else if (method_call.method_name() == "disposeSurface") {
    HandleDisposeSurface(resolved_args, std::move(result));
  } else if (method_call.method_name() == "markFrameAvailable") {
    HandleMarkFrameAvailable(resolved_args, std::move(result));
  } else {
    result->NotImplemented();
  }
}

void FlutterWgpuTexturePlugin::HandleCreateSurface(
    const flutter::EncodableMap& args,
    std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> result) {
  const auto surface_id = GetSurfaceId(args);
  const auto handle = GetHandle(args);
  const auto width = GetClampedInt(args, "width", 512, 1, 16384);
  const auto height = GetClampedInt(args, "height", 512, 1, 16384);
  if (handle == 0) {
    result->Error("invalid_handle", "Renderer handle is required");
    return;
  }

  auto surface = GetOrCreateSurface(surface_id);
  std::lock_guard<std::mutex> lock(surface->mutex);
  surface->handle = handle;
  surface->width = width;
  surface->height = height;

  if (surface->binding == nullptr) {
    void* shared_handle = engine_create_present_dxgi_surface(
        handle, static_cast<uint32_t>(width), static_cast<uint32_t>(height));
    if (!shared_handle) {
      result->Error(
          "create_present_failed",
          "engine_create_present_dxgi_surface returned null");
      return;
    }

    auto binding = std::make_shared<GpuSurfaceBinding>(
        shared_handle, static_cast<size_t>(width), static_cast<size_t>(height));
    FlutterDesktopTextureInfo texture_info{};
    texture_info.type = kFlutterDesktopGpuSurfaceTexture;
    texture_info.gpu_surface_config.struct_size =
        sizeof(FlutterDesktopGpuSurfaceTextureConfig);
    texture_info.gpu_surface_config.type =
        kFlutterDesktopGpuSurfaceTypeDxgiSharedHandle;
    texture_info.gpu_surface_config.callback = GpuSurfaceBinding::Callback;
    texture_info.gpu_surface_config.user_data = binding.get();

    const int64_t texture_id =
        FlutterDesktopTextureRegistrarRegisterExternalTexture(
            texture_registrar_, &texture_info);
    if (texture_id < 0) {
      CloseHandle(static_cast<HANDLE>(shared_handle));
      result->Error(
          "register_texture_failed",
          "RegisterExternalTexture returned < 0");
      return;
    }
    surface->binding = std::move(binding);
    surface->texture_id = texture_id;
  }

  flutter::EncodableMap response;
  response[flutter::EncodableValue("textureId")] =
      flutter::EncodableValue(surface->texture_id);
  response[flutter::EncodableValue("backend")] =
      flutter::EncodableValue(std::string(BackendName(engine_get_backend(handle))));
  response[flutter::EncodableValue("width")] = flutter::EncodableValue(width);
  response[flutter::EncodableValue("height")] = flutter::EncodableValue(height);
  result->Success(flutter::EncodableValue(response));
}

void FlutterWgpuTexturePlugin::HandleResizeSurface(
    const flutter::EncodableMap& args,
    std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> result) {
  const auto surface_id = GetSurfaceId(args);
  const auto handle = GetHandle(args);
  const auto width = GetClampedInt(args, "width", 512, 1, 16384);
  const auto height = GetClampedInt(args, "height", 512, 1, 16384);
  auto surface = FindSurface(surface_id);
  if (!surface) {
    result->Success();
    return;
  }

  std::lock_guard<std::mutex> lock(surface->mutex);
  if (engine_resize(handle, static_cast<uint32_t>(width), static_cast<uint32_t>(height)) == 0) {
    result->Error("resize_failed", "engine_resize returned 0");
    return;
  }
  void* shared_handle = engine_create_present_dxgi_surface(
      handle, static_cast<uint32_t>(width), static_cast<uint32_t>(height));
  if (!shared_handle) {
    result->Error(
        "create_present_failed",
        "engine_create_present_dxgi_surface returned null");
    return;
  }
  if (surface->binding) {
    surface->binding->Update(
        shared_handle, static_cast<size_t>(width), static_cast<size_t>(height));
  }
  surface->handle = handle;
  surface->width = width;
  surface->height = height;
  result->Success();
}

void FlutterWgpuTexturePlugin::HandleDisposeSurface(
    const flutter::EncodableMap& args,
    std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> result) {
  const auto surface_id = GetSurfaceId(args);
  auto surface = FindSurface(surface_id);
  if (!surface) {
    result->Success();
    return;
  }
  {
    std::lock_guard<std::mutex> lock(surface->mutex);
    UnregisterTextureLocked(surface);
  }
  RemoveSurface(surface_id);
  result->Success();
}

void FlutterWgpuTexturePlugin::HandleMarkFrameAvailable(
    const flutter::EncodableMap& args,
    std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> result) {
  const auto surface_id = GetSurfaceId(args);
  auto surface = FindSurface(surface_id);
  if (!surface) {
    result->Success();
    return;
  }
  std::lock_guard<std::mutex> lock(surface->mutex);
  if (surface->texture_id >= 0) {
    FlutterDesktopTextureRegistrarMarkExternalTextureFrameAvailable(
        texture_registrar_, surface->texture_id);
  }
  result->Success();
}

}  // namespace

void FlutterWgpuTexturePluginRegisterWithRegistrar(
    FlutterDesktopPluginRegistrarRef registrar) {
  FlutterWgpuTexturePlugin::RegisterWithRegistrar(
      flutter::PluginRegistrarManager::GetInstance()
          ->GetRegistrar<flutter::PluginRegistrarWindows>(registrar),
      registrar);
}

void FlutterWgpuTexturePluginCApiRegisterWithRegistrar(
    FlutterDesktopPluginRegistrarRef registrar) {
  FlutterWgpuTexturePluginRegisterWithRegistrar(registrar);
}
