#![cfg(target_os = "linux")]
#![allow(dead_code)]
#[allow(unused_imports)]
#[path = "backend.rs"]
mod backend;
#[path = "support/p10_e2e.rs"]
mod gate;
fn main() {
    gate::run();
}
