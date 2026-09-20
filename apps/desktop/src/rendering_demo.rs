//! Opt-in offline window/surface diagnostic; retains one sample and writes no logs.
use std::sync::{Arc, Mutex};

use eframe::{egui, egui_wgpu, wgpu};

#[derive(Clone, Default)]
pub struct RenderingDemo(Arc<Mutex<Option<([u32; 2], f32)>>>);

impl RenderingDemo {
	pub fn show(&self, ctx: &egui::Context, window: &winit::window::Window) {
		let sample = *self.0.lock().unwrap();
		egui::Window::new("Native rendering diagnostic").show(ctx, |ui| {
			let physical = window.inner_size();
			let native = ctx.input(|input| input.viewport().native_pixels_per_point);
			ui.label(format!("Client: {} × {} physical px", physical.width, physical.height));
			ui.label(format!("Window scale: {} / egui native: {native:?}", window.scale_factor()));
			ui.label(format!("egui pixels/point: {} / zoom: {}", ctx.pixels_per_point(), ctx.zoom_factor()));
			ui.label(format!("Logical viewport: {:?}", ctx.input(|input| input.viewport().inner_rect.map(|rect| rect.size()))));
			ui.label(format!("Maximized: {} / fullscreen: {}", window.is_maximized(), window.fullscreen().is_some()));
			if let Some((size, scale)) = sample {
				ui.label(format!("WGPU surface: {} × {} px / pixels/point: {scale}", size[0], size[1]));
				if size != [physical.width, physical.height] {
					ui.colored_label(egui::Color32::YELLOW, "Surface/client mismatch (sample is from the previous paint)");
				}
			}
			ui.label("Resize, maximize/restore, and move between monitors.\nAt rest, surface and client dimensions must match.");
			// A one-physical-pixel pattern reveals global presentation filtering.
			let (rect, _) = ui.allocate_exact_size(egui::vec2(256.0, 24.0), egui::Sense::hover());
			let scale = ctx.pixels_per_point();
			let origin = (rect.left() * scale).round();
			for x in 0..256 {
				let stripe = egui::Rect::from_min_max(
					egui::pos2((origin + x as f32) / scale, rect.top()),
					egui::pos2((origin + x as f32 + 1.0) / scale, rect.bottom()),
				);
				ui.painter().rect_filled(stripe, 0, if x % 2 == 0 { egui::Color32::WHITE } else { egui::Color32::BLACK });
			}
			ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, self.clone()));
		});
		ctx.request_repaint_after(std::time::Duration::from_millis(250));
	}
}

impl egui_wgpu::CallbackTrait for RenderingDemo {
	fn prepare(
		&self,
		_: &wgpu::Device,
		_: &wgpu::Queue,
		screen: &egui_wgpu::ScreenDescriptor,
		_: &mut wgpu::CommandEncoder,
		_: &mut egui_wgpu::CallbackResources,
	) -> Vec<wgpu::CommandBuffer> {
		// The pinned painter builds this from the configured surface, not logical UI size.
		*self.0.lock().unwrap() = Some((screen.size_in_pixels, screen.pixels_per_point));
		Vec::new()
	}

	fn paint(
		&self,
		_: egui::PaintCallbackInfo,
		_: &mut wgpu::RenderPass<'static>,
		_: &egui_wgpu::CallbackResources,
	) {
	}
}
