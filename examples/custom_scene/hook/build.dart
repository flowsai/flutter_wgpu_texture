// This hook compiles a combined Rust workspace that includes both the plugin's
// engine and the custom gradient scene. The output replaces the plugin's own
// libflutter_wgpu_texture dylib — the app-level hook takes precedence over
// the plugin's hook in the Flutter native-assets build system.

import 'package:hooks/hooks.dart';
import 'package:native_toolchain_rust/native_toolchain_rust.dart';

void main(List<String> args) async {
  await build(args, (input, output) async {
    await RustBuilder(
      assetName: 'flutter_wgpu_texture',
      cratePath: 'rust',
    ).run(input: input, output: output);
  });
}
