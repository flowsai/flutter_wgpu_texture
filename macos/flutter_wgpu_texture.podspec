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

  s.script_phase = {
    :name => 'Build Rust library',
    # First argument is relative path to the `rust` folder, second is name of rust library
    :script => 'sh "$PODS_TARGET_SRCROOT/../cargokit/build_pod.sh" ../rust flutter_wgpu_texture',
    :execution_position => :before_compile,
    :input_files => ['${BUILT_PRODUCTS_DIR}/cargokit_phony'],
    # Let XCode know that the static library referenced in -force_load below is
    # created by this build step.
    :output_files => ["${BUILT_PRODUCTS_DIR}/libflutter_wgpu_texture.a"],
  }
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    # Flutter.framework does not contain a i386 slice.
    'EXCLUDED_ARCHS[sdk=iphonesimulator*]' => 'i386',
    'OTHER_LDFLAGS' => '-force_load ${BUILT_PRODUCTS_DIR}/libflutter_wgpu_texture.a',
  }
end
