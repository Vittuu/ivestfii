//! Builds a fresh executable without replacing an instance already open on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    ivestfii_app::run();
}
