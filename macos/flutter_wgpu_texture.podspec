#
# To learn more about a Podspec see http://guides.cocoapods.org/syntax/podspec.html.
# Run `pod lib lint flutter_wgpu_texture.podspec` to validate before publishing.
#
Pod::Spec.new do |s|
  s.name             = 'flutter_wgpu_texture'
  s.version          = '0.0.1'
  s.summary          = 'Desktop Flutter texture plugin backed by Rust wgpu.'
  s.description      = <<-DESC
Desktop Flutter texture plugin backed by Rust wgpu.
                       DESC
  s.homepage         = 'https://github.com/example/flutter_wgpu_texture'
  s.license          = { :file => '../LICENSE' }
  s.author           = { 'Open Source Contributors' => 'devnull@example.com' }

  s.source           = { :path => '.' }
  s.source_files     = 'Classes/**/*'
  s.dependency 'FlutterMacOS'

  s.platform = :osx, '10.14'
  s.swift_version = '5.0'

  # Rust is compiled by the build hook (hook/build.dart) using native_toolchain_rust
  # and loaded at runtime by flutter_rust_bridge.  No static linking needed here.
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    # Flutter.framework does not contain a i386 slice.
    'EXCLUDED_ARCHS[sdk=iphonesimulator*]' => 'i386',
  }
end
