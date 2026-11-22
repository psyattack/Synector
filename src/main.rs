#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod arg_spellings;
pub mod autorun;
mod cli;
pub mod config;
mod daemon;
mod gui;
pub mod i18n;
mod ipc;
pub mod logging;
pub mod models;
pub mod sharing;
pub mod single_instance;
pub mod shortcut;
pub mod updater;
mod validation;

fn main() {
    app::run();
}
