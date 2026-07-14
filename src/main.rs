#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod monitor;
mod system;
mod theme;
mod worker;

use std::sync::Arc;

use eframe::egui;
use tracing_subscriber::EnvFilter;

fn main() -> eframe::Result {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .try_init();

    if let Some(exit_code) = monitor::run_helper_command() {
        std::process::exit(exit_code);
    }

    let Some(_single_instance) = system::acquire_single_instance() else {
        return Ok(());
    };

    let icon_size = 64;
    let icon = egui::IconData {
        rgba: system::brand_icon_rgba(icon_size),
        width: icon_size,
        height: icon_size,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("LG Monitor Control")
            .with_inner_size([980.0, 700.0])
            .with_min_inner_size([760.0, 540.0])
            .with_icon(Arc::new(icon)),
        renderer: eframe::Renderer::Glow,
        centered: true,
        persist_window: false,
        ..Default::default()
    };

    eframe::run_native(
        "LG Monitor Control",
        options,
        Box::new(|creation_context| Ok(Box::new(app::MonitorControlApp::new(creation_context)))),
    )
}
