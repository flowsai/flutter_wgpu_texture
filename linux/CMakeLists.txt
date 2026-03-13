# The Flutter tooling requires that developers have CMake 3.10 or later
# installed. You should not increase this version, as doing so will cause
# the plugin to fail to compile for some customers of the plugin.
cmake_minimum_required(VERSION 3.10)

set(PROJECT_NAME "flutter_wgpu_texture")
project(${PROJECT_NAME} LANGUAGES CXX)

set(PLUGIN_NAME "flutter_wgpu_texture_plugin")

set(OpenGL_GL_PREFERENCE LEGACY)
find_package(OpenGL REQUIRED)
find_package(PkgConfig REQUIRED)
pkg_check_modules(EGL REQUIRED egl)

add_library(${PLUGIN_NAME} SHARED
  "flutter_wgpu_texture_plugin.cc"
  "include/flutter_wgpu_texture/flutter_wgpu_texture_plugin.h"
)

apply_standard_settings(${PLUGIN_NAME})

set_target_properties(${PLUGIN_NAME} PROPERTIES
  CXX_VISIBILITY_PRESET hidden)
target_compile_definitions(${PLUGIN_NAME} PRIVATE FLUTTER_PLUGIN_IMPL)
target_include_directories(${PLUGIN_NAME} INTERFACE
  "${CMAKE_CURRENT_SOURCE_DIR}/include"
)
target_include_directories(${PLUGIN_NAME} PRIVATE ${EGL_INCLUDE_DIRS})
target_link_libraries(${PLUGIN_NAME} PRIVATE flutter)
target_link_libraries(${PLUGIN_NAME} PRIVATE PkgConfig::GTK)
target_link_libraries(${PLUGIN_NAME} PRIVATE OpenGL::GL)
target_link_libraries(${PLUGIN_NAME} PRIVATE ${EGL_LIBRARIES})

include("../cargokit/cmake/cargokit.cmake")
apply_cargokit(${PLUGIN_NAME} ../rust flutter_wgpu_texture "engine_create")

# List of absolute paths to libraries that should be bundled with the plugin.
set(flutter_wgpu_texture_bundled_libraries
  "${${PLUGIN_NAME}_cargokit_lib}"
  PARENT_SCOPE
)
