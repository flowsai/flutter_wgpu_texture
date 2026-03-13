#ifndef FLUTTER_PLUGIN_FLUTTER_WGPU_TEXTURE_PLUGIN_H_
#define FLUTTER_PLUGIN_FLUTTER_WGPU_TEXTURE_PLUGIN_H_

#include <flutter_linux/flutter_linux.h>

G_BEGIN_DECLS

#ifdef FLUTTER_PLUGIN_IMPL
#define FLUTTER_PLUGIN_EXPORT __attribute__((visibility("default")))
#else
#define FLUTTER_PLUGIN_EXPORT
#endif

G_DECLARE_FINAL_TYPE(FlutterWgpuTexturePlugin,
                     flutter_wgpu_texture_plugin,
                     FLUTTER_WGPU_TEXTURE,
                     PLUGIN,
                     GObject)

FLUTTER_PLUGIN_EXPORT void flutter_wgpu_texture_plugin_register_with_registrar(
    FlPluginRegistrar* registrar);

G_END_DECLS

#endif  // FLUTTER_PLUGIN_FLUTTER_WGPU_TEXTURE_PLUGIN_H_
