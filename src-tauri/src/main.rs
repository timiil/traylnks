// Prevent the accompanying console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    traylnks_lib::run();
}
