const COMMANDS: &[&str] = &[
    "ping",
    "prepare_vpn",
    "start_vpn",
    "stop_vpn",
    "get_vpn_status",
    "consume_vpn_tile_action",
    "get_installed_apps",
    "get_app_icon",
    "get_app_routing_policy",
    "set_app_routing_policy",
    "set_app_routing_page_open",
    "registerListener",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .ios_path("ios")
        .build();
}
