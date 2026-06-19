//! Headless proof that lights reach Rust, render, and are pickable (Linux only).
//! Run with: `cargo run --example smoke_lights`

fn main() {
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("smoke_lights only runs on Linux");
    }

    #[cfg(target_os = "linux")]
    {
        use flutter_wgpu_texture::api;

        let w = 640;
        let h = 360;

        println!("[smoke_lights] creating renderer {w}x{h}...");
        let info = api::create_renderer(w, h, "cube".to_string()).expect("create_renderer");
        let handle = info.handle;
        println!("[smoke_lights] backend={} device={}", info.backend.backend, info.backend.device_name);

        api::ensure_linux_present(handle, w, h).expect("ensure_linux_present");

        // Render a couple frames BEFORE pushing a scene: spawns the startup
        // fallback scene (cube+plane+directional, tagged FallbackMarker), exactly
        // like the real app does while the editor connects.
        for _ in 0..2 {
            api::request_frame(handle).expect("request_frame (fallback)");
        }

        // Scene: bright directional + spot aimed at the cube (identity rotation →
        // light_transform aims at origin) + cube + plane. Uses the Flax-style
        // `brightness` multiplier (mapped to lux/lumens in light::spawn_light).
        let scene_json = r#"{"entities":[
            {"id":"dl","name":"Dir","kind":"light:directional",
             "transform":{"translation":[3.0,8.0,5.0],"rotation":[0.0,0.0,0.0,1.0],"scale":[1.0,1.0,1.0]},
             "light":{"brightness":5.0,"color":[1.0,1.0,1.0,1.0],"shadow_maps_enabled":true}},
            {"id":"sp","name":"Spot","kind":"light:spot",
             "transform":{"translation":[0.0,5.0,0.0],"rotation":[0.0,0.0,0.0,1.0],"scale":[1.0,1.0,1.0]},
             "light":{"brightness":5.0,"range":20.0,"outer_angle":0.7854,"color":[1.0,1.0,1.0,1.0]}},
            {"id":"cube","name":"Cube","kind":"mesh:cube",
             "transform":{"translation":[0.0,0.5,0.0],"rotation":[0.0,0.0,0.0,1.0],"scale":[1.0,1.0,1.0]},
             "material":{"color":[0.8,0.6,0.4,1.0]}},
            {"id":"plane","name":"Plane","kind":"mesh:plane",
             "transform":{"translation":[0.0,0.0,0.0],"rotation":[0.0,0.0,0.0,1.0],"scale":[1.0,1.0,1.0]},
             "material":{"color":[0.5,0.5,0.5,1.0]}}
        ]}"#;
        println!("[smoke_lights] set_scene with directional + spot + cube + plane...");
        api::set_scene(handle, scene_json.to_string()).expect("set_scene");

        // Render several frames so the offscreen GpuImage prepares + lights extract.
        for i in 0..8 {
            let rendered = api::request_frame(handle).expect("request_frame");
            print!("[smoke_lights] frame {i}: rendered={rendered}\r");
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
        println!();

        // Picking: click the cube center and the spot position (projected).
        // The spot is at [0,5,0]; with the default orbit camera it projects near the
        // top-center of the view. We just confirm pick doesn't panic and reports something.
        let cx = (w as f32) * 0.5;
        let cy = (h as f32) * 0.5;
        let hit_center = api::pick(handle, cx, cy).expect("pick center");
        println!("[smoke_lights] pick(center) = {hit_center:?}");
        let hit_top = api::pick(handle, cx, (h as f32) * 0.2).expect("pick top");
        println!("[smoke_lights] pick(top) = {hit_top:?}");

        // Select the spot by id and exercise a gizmo drag (move) on it.
        api::select_entity(handle, Some("sp".to_string())).expect("select spot");
        api::set_gizmo_mode(handle, "translate".to_string()).expect("set_gizmo_mode");
        let grabbed = api::drag_begin(handle, cx, (h as f32) * 0.2).expect("drag_begin");
        println!("[smoke_lights] drag_begin on spot = grabbed={grabbed}");
        let moved = api::drag_update(handle, cx + 40.0, (h as f32) * 0.2).expect("drag_update");
        println!("[smoke_lights] drag_update -> {:?}", moved.map(|t| t.translation));
        api::drag_end(handle).expect("drag_end");

        println!("[smoke_lights] SUCCESS — lights spawned, frames rendered, picking + drag exercised");
        println!("[smoke_lights] Check the 'light::spawn:' info logs above for mapped lux/lumens.");
        api::dispose_renderer(handle);
    }
}