#include "include/flutter_wgpu_texture/flutter_wgpu_texture_plugin.h"

#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GL/gl.h>
#include <flutter_linux/flutter_linux.h>
#include <gtk/gtk.h>

#include <algorithm>
#include <cstring>
#include <cstdint>
#include <mutex>
#include <memory>
#include <string>
#include <unordered_map>
#include <unistd.h>
#include <vector>

// No extern "C" Rust declarations — Rust is compiled as a separate dylib
// loaded by flutter_rust_bridge.  All Rust operations happen in Dart via FRB;
// DMA-BUF file descriptor and metadata are forwarded here via the method
// channel so this file can create the EGL image / GL texture.

namespace {

constexpr char kChannelName[] = "flutter_wgpu_texture";

typedef void (*PFNGLEGLIMAGETARGETTEXTURE2DOESPROC)(GLenum target, EGLImageKHR image);

static int g_dmabuf_import_supported = -1;
static PFNEGLCREATEIMAGEKHRPROC g_egl_create_image_khr = nullptr;
static PFNEGLDESTROYIMAGEKHRPROC g_egl_destroy_image_khr = nullptr;
static PFNGLEGLIMAGETARGETTEXTURE2DOESPROC g_gl_egl_image_target_texture_2d_oes = nullptr;

struct DmaBufParams {
  int32_t fd = -1;
  uint32_t width = 0;
  uint32_t height = 0;
  int32_t stride = 0;
  int32_t offset = 0;
  int32_t fourcc = 0;
  uint32_t modifier_low = 0;
  uint32_t modifier_high = 0;
};

struct SurfaceState {
  std::string surface_id;
  int64_t texture_id = -1;
  FlTexture* texture = nullptr;
  GLuint gl_texture_name = 0;
  uint32_t width = 1;
  uint32_t height = 1;
  uint32_t imported_width = 0;
  uint32_t imported_height = 0;
  std::mutex mutex;
};

struct PluginState {
  explicit PluginState(FlTextureRegistrar* registrar)
      : texture_registrar(FL_TEXTURE_REGISTRAR(g_object_ref(registrar))) {}

  ~PluginState() {
    if (texture_registrar != nullptr) {
      g_object_unref(texture_registrar);
      texture_registrar = nullptr;
    }
  }

  FlTextureRegistrar* texture_registrar = nullptr;
  std::mutex mutex;
  std::unordered_map<std::string, std::shared_ptr<SurfaceState>> surfaces;
};

int64_t GetIntValue(FlValue* value, int64_t fallback) {
  if (value == nullptr) {
    return fallback;
  }
  switch (fl_value_get_type(value)) {
    case FL_VALUE_TYPE_INT:
      return fl_value_get_int(value);
    case FL_VALUE_TYPE_FLOAT:
      return static_cast<int64_t>(fl_value_get_float(value));
    case FL_VALUE_TYPE_STRING: {
      const gchar* raw = fl_value_get_string(value);
      if (raw == nullptr) {
        return fallback;
      }
      return g_ascii_strtoll(raw, nullptr, 10);
    }
    default:
      return fallback;
  }
}

std::string GetStringValue(FlValue* args, const char* key, const char* fallback) {
  FlValue* value = args == nullptr ? nullptr : fl_value_lookup_string(args, key);
  if (value != nullptr && fl_value_get_type(value) == FL_VALUE_TYPE_STRING) {
    const gchar* raw = fl_value_get_string(value);
    if (raw != nullptr && raw[0] != '\0') {
      return raw;
    }
  }
  return fallback;
}

int GetClampedInt(FlValue* args,
                  const char* key,
                  int fallback,
                  int min_value,
                  int max_value) {
  FlValue* value = args == nullptr ? nullptr : fl_value_lookup_string(args, key);
  return std::clamp(static_cast<int>(GetIntValue(value, fallback)), min_value, max_value);
}

DmaBufParams GetDmaBufParams(FlValue* args) {
  DmaBufParams p;
  p.fd = static_cast<int32_t>(GetIntValue(
      args ? fl_value_lookup_string(args, "fd") : nullptr, -1));
  p.width = static_cast<uint32_t>(GetClampedInt(args, "width", 1, 1, 65535));
  p.height = static_cast<uint32_t>(GetClampedInt(args, "height", 1, 1, 65535));
  p.stride = static_cast<int32_t>(GetIntValue(
      args ? fl_value_lookup_string(args, "stride") : nullptr, 0));
  p.offset = static_cast<int32_t>(GetIntValue(
      args ? fl_value_lookup_string(args, "offset") : nullptr, 0));
  p.fourcc = static_cast<int32_t>(GetIntValue(
      args ? fl_value_lookup_string(args, "fourcc") : nullptr, 0));
  p.modifier_low = static_cast<uint32_t>(GetIntValue(
      args ? fl_value_lookup_string(args, "modifierLow") : nullptr, 0));
  p.modifier_high = static_cast<uint32_t>(GetIntValue(
      args ? fl_value_lookup_string(args, "modifierHigh") : nullptr, 0));
  return p;
}

void detect_dmabuf_import_support(EGLDisplay egl_display) {
  if (g_dmabuf_import_supported == 1) {
    return;
  }
  g_dmabuf_import_supported = 0;
  if (egl_display == EGL_NO_DISPLAY) {
    return;
  }
  const char* extensions = eglQueryString(egl_display, EGL_EXTENSIONS);
  if (extensions == nullptr) {
    g_warning("DMA-BUF import detection: eglQueryString(EGL_EXTENSIONS) returned null");
    return;
  }
  std::string ext_str(extensions);
  if (ext_str.find("EGL_EXT_image_dma_buf_import") == std::string::npos) {
    return;
  }
  g_egl_create_image_khr = reinterpret_cast<PFNEGLCREATEIMAGEKHRPROC>(
      eglGetProcAddress("eglCreateImageKHR"));
  g_egl_destroy_image_khr = reinterpret_cast<PFNEGLDESTROYIMAGEKHRPROC>(
      eglGetProcAddress("eglDestroyImageKHR"));
  g_gl_egl_image_target_texture_2d_oes =
      reinterpret_cast<PFNGLEGLIMAGETARGETTEXTURE2DOESPROC>(
          eglGetProcAddress("glEGLImageTargetTexture2DOES"));
  if (g_egl_create_image_khr == nullptr || g_egl_destroy_image_khr == nullptr ||
      g_gl_egl_image_target_texture_2d_oes == nullptr) {
    return;
  }
  g_dmabuf_import_supported = 1;
}

GLuint import_dmabuf_to_gl_texture(const DmaBufParams& p) {
  if (p.fd < 0) {
    return 0;
  }
  std::vector<EGLint> attribs;
  attribs.push_back(EGL_LINUX_DRM_FOURCC_EXT);
  attribs.push_back(p.fourcc);
  attribs.push_back(EGL_WIDTH);
  attribs.push_back(static_cast<EGLint>(p.width));
  attribs.push_back(EGL_HEIGHT);
  attribs.push_back(static_cast<EGLint>(p.height));
  attribs.push_back(EGL_DMA_BUF_PLANE0_FD_EXT);
  attribs.push_back(p.fd);
  attribs.push_back(EGL_DMA_BUF_PLANE0_OFFSET_EXT);
  attribs.push_back(p.offset);
  attribs.push_back(EGL_DMA_BUF_PLANE0_PITCH_EXT);
  attribs.push_back(p.stride);
  if (p.modifier_low != 0 || p.modifier_high != 0) {
    attribs.push_back(EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT);
    attribs.push_back(static_cast<EGLint>(p.modifier_low));
    attribs.push_back(EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT);
    attribs.push_back(static_cast<EGLint>(p.modifier_high));
  }
  attribs.push_back(EGL_NONE);

  EGLDisplay egl_display = eglGetCurrentDisplay();
  if (egl_display == EGL_NO_DISPLAY) {
    return 0;
  }
  EGLImageKHR egl_image = g_egl_create_image_khr(
      egl_display, EGL_NO_CONTEXT, EGL_LINUX_DMA_BUF_EXT, nullptr, attribs.data());
  if (egl_image == EGL_NO_IMAGE_KHR || egl_image == nullptr) {
    return 0;
  }

  GLuint texture_name = 0;
  glGenTextures(1, &texture_name);
  if (texture_name == 0) {
    g_egl_destroy_image_khr(egl_display, egl_image);
    return 0;
  }
  glBindTexture(GL_TEXTURE_2D, texture_name);
  g_gl_egl_image_target_texture_2d_oes(GL_TEXTURE_2D, egl_image);
  GLenum err = glGetError();
  if (err != GL_NO_ERROR) {
    g_warning("DMA-BUF import: glEGLImageTargetTexture2DOES failed with GL error 0x%x", err);
    glBindTexture(GL_TEXTURE_2D, 0);
    glDeleteTextures(1, &texture_name);
    g_egl_destroy_image_khr(egl_display, egl_image);
    return 0;
  }
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
  glBindTexture(GL_TEXTURE_2D, 0);
  g_egl_destroy_image_khr(egl_display, egl_image);
  return texture_name;
}

void release_surface_texture(const std::shared_ptr<SurfaceState>& surface) {
  if (surface == nullptr) {
    return;
  }
  if (surface->gl_texture_name != 0) {
    GLuint texture_name = surface->gl_texture_name;
    glDeleteTextures(1, &texture_name);
    surface->gl_texture_name = 0;
    surface->imported_width = 0;
    surface->imported_height = 0;
  }
}

typedef struct _FlutterWgpuLinuxTexture FlutterWgpuLinuxTexture;
typedef struct _FlutterWgpuLinuxTextureClass FlutterWgpuLinuxTextureClass;

struct _FlutterWgpuLinuxTexture {
  FlTextureGL parent_instance;
  std::shared_ptr<SurfaceState>* surface;
};

struct _FlutterWgpuLinuxTextureClass {
  FlTextureGLClass parent_class;
};

G_DEFINE_TYPE(FlutterWgpuLinuxTexture,
              flutter_wgpu_linux_texture,
              fl_texture_gl_get_type())

gboolean flutter_wgpu_linux_texture_populate(FlTextureGL* texture,
                                             uint32_t* target,
                                             uint32_t* name,
                                             uint32_t* width,
                                             uint32_t* height,
                                             GError** error) {
  (void)error;
  auto* self = reinterpret_cast<FlutterWgpuLinuxTexture*>(texture);
  if (self->surface == nullptr || !(*self->surface)) {
    return FALSE;
  }
  std::shared_ptr<SurfaceState> surface = *self->surface;
  std::lock_guard<std::mutex> lock(surface->mutex);
  const uint32_t pixel_width = std::max(surface->width, 1u);
  const uint32_t pixel_height = std::max(surface->height, 1u);

  if (g_dmabuf_import_supported != 1) {
    detect_dmabuf_import_support(eglGetCurrentDisplay());
  }
  if (g_dmabuf_import_supported != 1) {
    g_warning("DMA-BUF populate failed: EGL import support unavailable");
    return FALSE;
  }

  // The GL texture is re-imported whenever the surface size changes.
  // The Dart layer (via FRB + method channel) always calls resizeSurface before
  // the next frame when dimensions change, which clears gl_texture_name.
  if (surface->gl_texture_name == 0) {
    g_warning("DMA-BUF populate: no GL texture yet (waiting for DMA-BUF import)");
    return FALSE;
  }

  *target = GL_TEXTURE_2D;
  *name = surface->gl_texture_name;
  *width = pixel_width;
  *height = pixel_height;
  return TRUE;
}

void flutter_wgpu_linux_texture_dispose(GObject* object) {
  auto* self = reinterpret_cast<FlutterWgpuLinuxTexture*>(object);
  delete self->surface;
  self->surface = nullptr;
  G_OBJECT_CLASS(flutter_wgpu_linux_texture_parent_class)->dispose(object);
}

void flutter_wgpu_linux_texture_class_init(FlutterWgpuLinuxTextureClass* klass) {
  FL_TEXTURE_GL_CLASS(klass)->populate = flutter_wgpu_linux_texture_populate;
  G_OBJECT_CLASS(klass)->dispose = flutter_wgpu_linux_texture_dispose;
}

void flutter_wgpu_linux_texture_init(FlutterWgpuLinuxTexture* self) {
  self->surface = nullptr;
}

FlTexture* flutter_wgpu_linux_texture_new(const std::shared_ptr<SurfaceState>& surface) {
  auto* texture = reinterpret_cast<FlutterWgpuLinuxTexture*>(
      g_object_new(flutter_wgpu_linux_texture_get_type(), nullptr));
  texture->surface = new std::shared_ptr<SurfaceState>(surface);
  return FL_TEXTURE(texture);
}

FlMethodResponse* make_success_response(FlValue* value) {
  return FL_METHOD_RESPONSE(fl_method_success_response_new(value));
}

FlMethodResponse* make_error_response(const char* code, const char* message) {
  return FL_METHOD_RESPONSE(fl_method_error_response_new(code, message, nullptr));
}

}  // namespace

struct _FlutterWgpuTexturePlugin {
  GObject parent_instance;
  PluginState* state;
};

G_DEFINE_TYPE(FlutterWgpuTexturePlugin,
              flutter_wgpu_texture_plugin,
              g_object_get_type())

static FlMethodResponse* handle_create_surface(FlutterWgpuTexturePlugin* self, FlValue* args) {
  if (self->state == nullptr) {
    return make_error_response("unavailable", "Plugin state unavailable");
  }
  const std::string surface_id = GetStringValue(args, "surfaceId", "default");
  const DmaBufParams params = GetDmaBufParams(args);

  if (params.fd < 0) {
    return make_error_response("invalid-fd", "DMA-BUF fd is required");
  }

  // Import the DMA-BUF into an OpenGL texture.
  if (g_dmabuf_import_supported != 1) {
    detect_dmabuf_import_support(eglGetCurrentDisplay());
  }
  if (g_dmabuf_import_supported != 1) {
    return make_error_response("dmabuf-unsupported", "EGL DMA-BUF import not supported");
  }

  GLuint gl_texture = import_dmabuf_to_gl_texture(params);
  // Close the fd — the EGL image holds a reference now.
  close(params.fd);
  if (gl_texture == 0) {
    return make_error_response("import-failed", "Failed to import DMA-BUF into GL texture");
  }

  std::shared_ptr<SurfaceState> surface;
  {
    std::lock_guard<std::mutex> lock(self->state->mutex);
    auto it = self->state->surfaces.find(surface_id);
    if (it == self->state->surfaces.end()) {
      surface = std::make_shared<SurfaceState>();
      surface->surface_id = surface_id;
      self->state->surfaces.emplace(surface_id, surface);
    } else {
      surface = it->second;
    }
  }
  {
    std::lock_guard<std::mutex> lock(surface->mutex);
    release_surface_texture(surface);
    surface->gl_texture_name = gl_texture;
    surface->width = params.width;
    surface->height = params.height;
    surface->imported_width = params.width;
    surface->imported_height = params.height;

    if (surface->texture == nullptr) {
      g_autoptr(FlTexture) texture = flutter_wgpu_linux_texture_new(surface);
      if (!fl_texture_registrar_register_texture(self->state->texture_registrar, texture)) {
        return make_error_response("register-failed", "Texture registration failed");
      }
      surface->texture = FL_TEXTURE(g_object_ref(texture));
      surface->texture_id = fl_texture_get_id(surface->texture);
    }
  }

  g_autoptr(FlValue) result = fl_value_new_map();
  fl_value_set_string_take(result, "textureId", fl_value_new_int(surface->texture_id));
  fl_value_set_string_take(result, "width", fl_value_new_int(params.width));
  fl_value_set_string_take(result, "height", fl_value_new_int(params.height));
  return make_success_response(result);
}

static FlMethodResponse* handle_resize_surface(FlutterWgpuTexturePlugin* self, FlValue* args) {
  if (self->state == nullptr) {
    return make_error_response("unavailable", "Plugin state unavailable");
  }
  const std::string surface_id = GetStringValue(args, "surfaceId", "default");
  const DmaBufParams params = GetDmaBufParams(args);

  std::shared_ptr<SurfaceState> surface;
  {
    std::lock_guard<std::mutex> lock(self->state->mutex);
    auto it = self->state->surfaces.find(surface_id);
    if (it == self->state->surfaces.end()) {
      return make_error_response("missing-surface", "Surface not found");
    }
    surface = it->second;
  }

  if (params.fd < 0) {
    return make_error_response("invalid-fd", "DMA-BUF fd is required");
  }

  GLuint gl_texture = import_dmabuf_to_gl_texture(params);
  close(params.fd);
  if (gl_texture == 0) {
    return make_error_response("import-failed", "Failed to import DMA-BUF into GL texture");
  }

  {
    std::lock_guard<std::mutex> lock(surface->mutex);
    release_surface_texture(surface);
    surface->gl_texture_name = gl_texture;
    surface->width = params.width;
    surface->height = params.height;
    surface->imported_width = params.width;
    surface->imported_height = params.height;
  }
  return make_success_response(nullptr);
}

static FlMethodResponse* handle_dispose_surface(FlutterWgpuTexturePlugin* self, FlValue* args) {
  if (self->state == nullptr) {
    return make_error_response("unavailable", "Plugin state unavailable");
  }
  const std::string surface_id = GetStringValue(args, "surfaceId", "default");
  std::shared_ptr<SurfaceState> surface;
  {
    std::lock_guard<std::mutex> lock(self->state->mutex);
    auto it = self->state->surfaces.find(surface_id);
    if (it == self->state->surfaces.end()) {
      return make_success_response(nullptr);
    }
    surface = it->second;
    self->state->surfaces.erase(it);
  }
  {
    std::lock_guard<std::mutex> lock(surface->mutex);
    release_surface_texture(surface);
    if (surface->texture != nullptr) {
      fl_texture_registrar_unregister_texture(self->state->texture_registrar, surface->texture);
      g_object_unref(surface->texture);
      surface->texture = nullptr;
    }
  }
  return make_success_response(nullptr);
}

static FlMethodResponse* handle_mark_frame_available(FlutterWgpuTexturePlugin* self, FlValue* args) {
  if (self->state == nullptr) {
    return make_error_response("unavailable", "Plugin state unavailable");
  }
  const std::string surface_id = GetStringValue(args, "surfaceId", "default");
  std::shared_ptr<SurfaceState> surface;
  {
    std::lock_guard<std::mutex> lock(self->state->mutex);
    auto it = self->state->surfaces.find(surface_id);
    if (it == self->state->surfaces.end()) {
      return make_success_response(nullptr);
    }
    surface = it->second;
  }
  if (surface->texture != nullptr) {
    fl_texture_registrar_mark_texture_frame_available(self->state->texture_registrar, surface->texture);
  }
  return make_success_response(nullptr);
}

static void flutter_wgpu_texture_plugin_handle_method_call(
    FlutterWgpuTexturePlugin* self,
    FlMethodCall* method_call) {
  const gchar* method = fl_method_call_get_name(method_call);
  FlValue* args = fl_method_call_get_args(method_call);
  g_autoptr(FlMethodResponse) response = nullptr;

  if (strcmp(method, "createSurface") == 0) {
    response = handle_create_surface(self, args);
  } else if (strcmp(method, "resizeSurface") == 0) {
    response = handle_resize_surface(self, args);
  } else if (strcmp(method, "disposeSurface") == 0) {
    response = handle_dispose_surface(self, args);
  } else if (strcmp(method, "markFrameAvailable") == 0) {
    response = handle_mark_frame_available(self, args);
  } else {
    response = FL_METHOD_RESPONSE(fl_method_not_implemented_response_new());
  }

  fl_method_call_respond(method_call, response, nullptr);
}

static void method_call_cb(FlMethodChannel* channel, FlMethodCall* method_call, gpointer user_data) {
  (void)channel;
  flutter_wgpu_texture_plugin_handle_method_call(
      FLUTTER_WGPU_TEXTURE_PLUGIN(user_data), method_call);
}

static void flutter_wgpu_texture_plugin_dispose(GObject* object) {
  auto* self = FLUTTER_WGPU_TEXTURE_PLUGIN(object);
  delete self->state;
  self->state = nullptr;
  G_OBJECT_CLASS(flutter_wgpu_texture_plugin_parent_class)->dispose(object);
}

static void flutter_wgpu_texture_plugin_class_init(FlutterWgpuTexturePluginClass* klass) {
  G_OBJECT_CLASS(klass)->dispose = flutter_wgpu_texture_plugin_dispose;
}

static void flutter_wgpu_texture_plugin_init(FlutterWgpuTexturePlugin* self) {
  self->state = nullptr;
}

FLUTTER_PLUGIN_EXPORT void flutter_wgpu_texture_plugin_register_with_registrar(
    FlPluginRegistrar* registrar) {
  auto* plugin = FLUTTER_WGPU_TEXTURE_PLUGIN(
      g_object_new(flutter_wgpu_texture_plugin_get_type(), nullptr));
  plugin->state = new PluginState(fl_plugin_registrar_get_texture_registrar(registrar));

  g_autoptr(FlStandardMethodCodec) codec = fl_standard_method_codec_new();
  g_autoptr(FlMethodChannel) channel = fl_method_channel_new(
      fl_plugin_registrar_get_messenger(registrar),
      kChannelName,
      FL_METHOD_CODEC(codec));
  fl_method_channel_set_method_call_handler(channel, method_call_cb, g_object_ref(plugin), g_object_unref);
  g_object_unref(plugin);
}
