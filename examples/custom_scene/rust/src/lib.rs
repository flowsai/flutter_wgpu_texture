// Combined workspace bridge: engine + gradient scene in one cdylib.
//
// `pub use *` re-exports every public item from the plugin engine at this
// crate's root.  That includes:
//   - engine_create / engine_dispose / engine_request_frame / … (hand-written C exports)
//   - frb_get_rust_content_hash / frb_pde_ffi_dispatcher_* (FRB boilerplate)
//
// Declaring these as *public items of this crate* is what prevents Rust's
// dead-code eliminator from stripping them before the cdylib link step.
// Without this the symbols exist in the engine rlib but never reach the
// combined dylib's export table.
//
// The gradient_scene dep is listed in Cargo.toml; its #[ctor::ctor] fn runs
// at dylib load time and registers "gradient" in the scene registry.
pub use flutter_wgpu_texture_engine::*;
