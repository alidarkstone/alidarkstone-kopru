#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![allow(dead_code)]

mod features;
mod input;
mod msg;
mod net;
mod state;
mod ui;

fn main() {
    // Birleştirme koordinatör tarafından yapılacak (src/app.rs).
    // Geçici: `kopru --demo` arayüzü örnek veriyle açar.
}
