#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![allow(dead_code)]

mod app;
mod config;
mod features;
mod msg;
mod net;
mod state;
mod ui;

fn main() {
    if std::env::args().any(|a| a == "--demo") {
        let _ = ui::run_demo();
        return;
    }
    if let Err(e) = app::run() {
        show_error(&e);
        std::process::exit(1);
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn show_error(msg: &str) {
    rfd::MessageDialog::new().set_title("Köprü").set_description(msg).set_level(rfd::MessageLevel::Error).show();
}

#[cfg(not(any(windows, target_os = "macos")))]
fn show_error(msg: &str) {
    eprintln!("Köprü: {msg}");
}
