//! Spike S1 of docs/RUST_MIGRATION.md: a frame that proves text, layout and capture.

use gz_app::{FrameReport, Shell};

pub struct SpikeShell {
    clicks: u32,
    text: String,
}

impl SpikeShell {
    pub fn new() -> Self {
        Self { clicks: 0, text: "Ночная доставка".to_owned() }
    }
}

fn load_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let dir = std::path::Path::new(&std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_owned())).join("Fonts");
    for (name, file, family) in [
        ("segoeui", "segoeui.ttf", egui::FontFamily::Proportional),
        ("cascadia", "CascadiaMono.ttf", egui::FontFamily::Monospace),
    ] {
        if let Ok(bytes) = std::fs::read(dir.join(file)) {
            fonts.font_data.insert(name.to_owned(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
            fonts.families.entry(family).or_default().insert(0, name.to_owned());
        }
    }
    ctx.set_fonts(fonts);
}

impl Shell for SpikeShell {
    fn init(&mut self, ctx: &egui::Context, _: &egui_wgpu::RenderState) {
        load_fonts(ctx);
    }

    fn frame(&mut self, ui: &mut egui::Ui) -> FrameReport {
        egui::Panel::top("caption").exact_size(36.0).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.strong("Grizzly Editor");
                ui.separator();
                for menu in ["File", "Edit", "Scene", "Mod", "Libraries", "Help"] {
                    let _ = ui.button(menu);
                }
            });
        });
        egui::Panel::bottom("status").exact_size(24.0).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label("Ready");
                ui.separator();
                ui.monospace("0 errors · 0 warnings");
            });
        });
        egui::Panel::left("graph").default_size(280.0).resizable(true).show(ui, |ui| {
            ui.heading("Graph");
            ui.label("Tasks");
            for i in 1..=5 {
                ui.label(format!("{i}. Найти тайник у моста"));
            }
        });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Spike S1");
            ui.label("The quick brown fox jumps over the lazy dog. 0123456789");
            ui.label("Съешь же ещё этих мягких французских булок, да выпей чаю.");
            ui.monospace("return { quest = 1, title = \"Ночная доставка\" }");
            if ui.button(format!("Clicked {} times", self.clicks)).clicked() {
                self.clicks += 1;
            }
            ui.text_edit_singleline(&mut self.text);
        });
        FrameReport { active_tab: "spike".into() }
    }

    fn close_requested(&mut self) -> bool {
        true
    }
}
