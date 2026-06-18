//! Standalone smoke test for the Bevy -> DMA-BUF pipeline (Linux only).
//!
//! Run with: `cargo run --example smoke_dmabuf`
//!
//! Exercises the same path the Flutter plugin uses, without Flutter:
//!   create_renderer -> ensure_linux_present -> render frame -> export_dmabuf.

fn main() {
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("smoke_dmabuf only runs on Linux");
    }

    #[cfg(target_os = "linux")]
    {
        use flutter_wgpu_texture::api;

        env_logger_init();

        let w = 640;
        let h = 360;

        println!("[smoke] creating renderer {w}x{h} (starts Bevy render thread)...");
        let info = api::create_renderer(w, h, "cube".to_string()).expect("create_renderer");
        let handle = info.handle;
        println!(
            "[smoke] backend={} device={} driver={}",
            info.backend.backend, info.backend.device_name, info.backend.driver
        );

        let supported = api::linux_dmabuf_supported(handle);
        println!("[smoke] dmabuf supported = {supported}");
        if !supported {
            eprintln!("[smoke] FAIL: DMA-BUF export not supported on this device");
            std::process::exit(1);
        }

        println!("[smoke] ensure_linux_present...");
        api::ensure_linux_present(handle, w, h).expect("ensure_linux_present");

        // Exercise the set_scene path (deserialize + reconcile) headless.
        let scene_json = r#"{
            "entities": [
                {"id":"cube","name":"Cube","kind":"mesh:cube",
                 "transform":{"translation":[0.0,0.5,0.0],"rotation":[0.0,0.0,0.0,1.0],"scale":[1.0,1.0,1.0]},
                 "material":{"color":[0.4,0.6,0.9,1.0]}},
                {"id":"plane","name":"Plane","kind":"mesh:plane",
                 "transform":{"translation":[0.0,0.0,0.0],"rotation":[0.0,0.0,0.0,1.0],"scale":[1.0,1.0,1.0]},
                 "material":{"color":[0.3,0.3,0.3,1.0]}},
                {"id":"light","name":"DirectionalLight","kind":"light:directional",
                 "transform":{"translation":[3.0,8.0,5.0],"rotation":[0.0,0.0,0.0,1.0],"scale":[1.0,1.0,1.0]},
                 "light":{"illuminance":10000.0}}
            ]
        }"#;
        println!("[smoke] set_scene...");
        api::set_scene(handle, scene_json.to_string()).expect("set_scene");

        // Render a few frames so the offscreen GpuImage gets prepared.
        let mut last_ok = false;
        for i in 0..5 {
            let rendered = api::request_frame(handle).expect("request_frame");
            println!("[smoke] frame {i}: rendered={rendered}");
            last_ok = rendered;
        }
        if !last_ok {
            eprintln!("[smoke] WARN: last frame reported not-rendered (GpuImage not ready?)");
        }

        // Exercise pick + selection + gizmo mode (headless; just must not panic).
        println!("[smoke] pick at center...");
        let hit = api::pick(handle, (w / 2) as f32, (h / 2) as f32).expect("pick");
        println!("[smoke] pick hit = {hit:?}");
        api::select_entity(handle, hit.clone()).expect("select_entity");
        api::set_gizmo_mode(handle, "translate".to_string()).expect("set_gizmo_mode");
        // Exercise camera nav (must not panic).
        api::camera_orbit(handle, 20.0, 10.0).expect("camera_orbit");
        api::camera_pan(handle, 5.0, 5.0).expect("camera_pan");
        api::camera_zoom(handle, -50.0).expect("camera_zoom");
        api::camera_fly(handle, 1.0, 0.0, 0.0, 0.016).expect("camera_fly");
        // Exercise gizmo drag (must not panic; grabbing may or may not hit a handle).
        let cx = (w / 2) as f32;
        let cy = (h / 2) as f32;
        let grabbed = api::drag_begin(handle, cx, cy).expect("drag_begin");
        println!("[smoke] drag_begin grabbed = {grabbed}");
        let moved = api::drag_update(handle, cx + 30.0, cy).expect("drag_update");
        println!("[smoke] drag_update -> {:?}", moved.map(|t| t.translation));
        api::drag_end(handle).expect("drag_end");
        // Render a couple frames so gizmos draw.
        for _ in 0..2 {
            api::request_frame(handle).expect("request_frame");
        }

        println!("[smoke] export_dmabuf...");
        match api::export_dmabuf(handle).expect("export_dmabuf") {
            Some(export) => {
                println!(
                    "[smoke] OK fd={} {}x{} stride={} offset={} fourcc=0x{:x} modifier={:#x}{:08x}",
                    export.fd,
                    export.width,
                    export.height,
                    export.stride,
                    export.offset,
                    export.fourcc,
                    export.modifier_high,
                    export.modifier_low,
                );
                assert!(export.fd >= 0, "fd should be valid");
                // Close the fd so we don't leak.
                unsafe {
                    libc_close(export.fd);
                }
                println!("[smoke] SUCCESS");
            }
            None => {
                eprintln!("[smoke] FAIL: export_dmabuf returned None");
                std::process::exit(1);
            }
        }

        api::dispose_renderer(handle);
    }
}

#[cfg(target_os = "linux")]
fn env_logger_init() {
    // Bevy uses tracing/log; without a subscriber, logs are dropped, which is fine.
    // Keep eprintln diagnostics from linux_dma_buf visible (they use eprintln).
}

#[cfg(target_os = "linux")]
unsafe fn libc_close(fd: i32) {
    extern "C" {
        fn close(fd: i32) -> i32;
    }
    let _ = close(fd);
}
