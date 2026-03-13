import 'dart:io';

import 'package:path/path.dart' as p;

class RustDynamicLibrary {
  RustDynamicLibrary._();

  static String open() => _cached;

  static final String _cached = _openImpl();

  static String _openImpl() {
    if (Platform.isWindows) {
      return 'flutter_wgpu_texture.dll';
    }
    if (Platform.isMacOS) {
      final exeDir = p.dirname(Platform.resolvedExecutable);
      final candidates = <String>[
        p.normalize(
          p.join(
            exeDir,
            '..',
            'Frameworks',
            'flutter_wgpu_texture.framework',
            'flutter_wgpu_texture',
          ),
        ),
        p.normalize(
          p.join(exeDir, '..', 'Frameworks', 'libflutter_wgpu_texture.dylib'),
        ),
        p.normalize(
          p.join(
            Directory.current.path,
            'rust',
            'target',
            'release',
            'libflutter_wgpu_texture.dylib',
          ),
        ),
      ];
      for (final candidate in candidates) {
        if (candidate.contains('/') && !File(candidate).existsSync()) {
          continue;
        }
        return candidate;
      }
      return 'flutter_wgpu_texture.framework/flutter_wgpu_texture';
    }
    final candidates = <String>[
      p.normalize(
        p.join(
          Directory.current.path,
          'rust',
          'target',
          'release',
          'libflutter_wgpu_texture.so',
        ),
      ),
      'libflutter_wgpu_texture.so',
      'flutter_wgpu_texture.so',
    ];
    for (final candidate in candidates) {
      if (candidate.contains('/') && !File(candidate).existsSync()) {
        continue;
      }
      return candidate;
    }
    return 'libflutter_wgpu_texture.so';
  }
}
