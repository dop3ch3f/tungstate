// The commands the window may call, so Tauri can write their permissions.
const COMMANDS: &[&str] = &["access", "ask", "summary"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .ios_path("ios")
        .build();
}
