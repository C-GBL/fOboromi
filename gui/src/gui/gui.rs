use eframe::egui::{
    self, Align2, CentralPanel, Color32, Pos2, Rect, RichText, ScrollArea, TextureHandle, Vec2,
};
use oboromi_core::sys::boot::{self, BootPhase, BootResult, FirmwareConfig};
use oboromi_core::tests::run::run_tests;
use std::time::{Duration, Instant};

pub struct GUI {
    pub logs: Vec<String>,
    pub test_thread: Option<std::thread::JoinHandle<Vec<String>>>,
    pub boot_thread: Option<std::thread::JoinHandle<BootResult>>,
    pub boot_phase: BootPhase,
    pub splash_start: Instant,
    pub logo: Option<TextureHandle>,
}

impl Default for GUI {
    fn default() -> Self {
        Self {
            logs: vec!["click 'Run CPU Tests' or 'Boot Firmware' to begin".to_string()],
            test_thread: None,
            boot_thread: None,
            boot_phase: BootPhase::Off,
            splash_start: Instant::now(),
            logo: None,
        }
    }
}

impl eframe::App for GUI {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.logo.is_none() {
            let bytes = include_bytes!("../../../assets/oboromi_logo.png");
            let image = image::load_from_memory(bytes)
                .expect("failed to load image")
                .to_rgba8();
            let size = [image.width() as usize, image.height() as usize];
            let tex = ctx.load_texture(
                "logo",
                egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
                egui::TextureOptions::LINEAR,
            );
            self.logo = Some(tex);
        }

        let now = Instant::now();
        let elapsed = now.duration_since(self.splash_start);
        let fade_in = Duration::from_millis(400);
        let hold = Duration::from_millis(20000);
        let fade_out = Duration::from_millis(400);
        let total = fade_in + hold + fade_out;

        let (phase, progress) = if elapsed < fade_in {
            ("fade_in", elapsed.as_secs_f32() / fade_in.as_secs_f32())
        } else if elapsed < fade_in + hold {
            ("hold", 1.0)
        } else if elapsed < total {
            (
                "fade_out",
                1.0 - (elapsed - fade_in - hold).as_secs_f32() / fade_out.as_secs_f32(),
            )
        } else {
            ("done", 0.0)
        };

        if phase != "done" {
            CentralPanel::default().show(ctx, |ui| {
                let rect = ui.max_rect();
                let painter = ui.painter();

                let bg_color = match phase {
                    "fade_in" => Color32::from_rgb(25, 25, 25),
                    "hold" => Color32::from_rgb(25, 25, 25),
                    "fade_out" => Color32::from_rgb(25, 25, 25),
                    _ => Color32::BLACK,
                };
                painter.rect_filled(rect, 0.0, bg_color);

                let Some(tex) = &self.logo else {
                    return;
                };
                let center = rect.center();
                let logo_size = tex.size_vec2() * 0.3;
                let logo_rect = Rect::from_center_size(center + Vec2::new(0.0, -60.0), logo_size);

                painter.image(
                    tex.id(),
                    logo_rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::from_rgba_premultiplied(255, 255, 255, (progress * 255.0) as u8),
                );

                let text_color =
                    Color32::from_rgba_premultiplied(120, 180, 255, (progress * 255.0) as u8);
                painter.text(
                    center + Vec2::new(0.0, logo_size.y / 2.0 - 16.0),
                    Align2::CENTER_TOP,
                    "fOboromi",
                    egui::TextStyle::Heading.resolve(ui.style()),
                    text_color,
                );

                // Pre-release warning text
                let warning_color =
                    Color32::from_rgba_premultiplied(255, 200, 100, (progress * 200.0) as u8);
                let info_color =
                    Color32::from_rgba_premultiplied(180, 180, 180, (progress * 180.0) as u8);

                painter.text(
                    center + Vec2::new(0.0, logo_size.y / 2.0 + 40.0),
                    Align2::CENTER_TOP,
                    "Experimental",
                    egui::FontId::proportional(14.0),
                    warning_color,
                );

                // Multi-line explanation
                let lines = vec![
                    "Forked by C-GBL, added storage emulation, firmware support;",
                    "kernel emulation (basic) and NCA decryption.",
                    " ",
                    "fOboromi (oboromi) 0.1.0",
                    "This is an experimental foundation for Switch 2 emulation.",
                    "Without a kernel exploit, running retail games is currently impossible.",
                    "This release focuses on CPU instruction emulation only.",
                ];

                for (i, line) in lines.iter().enumerate() {
                    painter.text(
                        center + Vec2::new(0.0, logo_size.y / 2.0 + 70.0 + (i as f32 * 18.0)),
                        Align2::CENTER_TOP,
                        *line,
                        egui::FontId::proportional(11.0),
                        info_color,
                    );
                }
            });

            ctx.request_repaint();
            return;
        }

        CentralPanel::default().show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.style_mut().visuals.button_frame = true;

                ui.menu_button("File", |ui| {
                    if ui.button("Quit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("About", |ui| {
                    ui.hyperlink_to("See the code", "https://git.eden-emu.dev/Nikilite/oboromi/");
                });
            });

            ui.heading("fOboromi");
            ui.separator();

            // Collect finished background threads.
            if let Some(handle) = self.test_thread.take() {
                if handle.is_finished() {
                    self.logs = handle.join().unwrap();
                } else {
                    self.test_thread = Some(handle);
                }
            }
            if let Some(handle) = self.boot_thread.take() {
                if handle.is_finished() {
                    let result = handle.join().unwrap();
                    self.boot_phase = result.phase;
                    self.logs = result.log;
                } else {
                    self.boot_thread = Some(handle);
                }
            }

            let busy = self.test_thread.is_some() || self.boot_thread.is_some();

            ui.horizontal(|ui| {
                // -- CPU Tests button --
                if ui.add_enabled(!busy, egui::Button::new("Run CPU Tests")).clicked() {
                    let ctx = ctx.clone();
                    self.test_thread = Some(std::thread::spawn(move || {
                        ctx.request_repaint();
                        run_tests()
                    }));
                    self.logs = vec![
                        "Warming up JIT compiler...".into(),
                        "Running ARM64 tests...".into(),
                    ];
                }

                // -- Boot Firmware button --
                if ui.add_enabled(!busy, egui::Button::new("Boot Firmware")).clicked() {
                    let ctx = ctx.clone();
                    self.boot_phase = BootPhase::HardwareInit;
                    self.boot_thread = Some(std::thread::spawn(move || {
                        let config = FirmwareConfig::default();
                        let result = boot::boot(&config);
                        ctx.request_repaint();
                        result
                    }));
                    self.logs = vec!["Booting firmware...".into()];
                }

                // -- Status indicator --
                if busy {
                    ui.spinner();
                    if self.boot_thread.is_some() {
                        ui.label(format!("Boot phase: {:?}", self.boot_phase));
                    } else {
                        ui.label("Running tests...");
                    }
                }
            });

            // -- Boot phase status bar --
            if self.boot_phase != BootPhase::Off {
                ui.separator();
                let (phase_text, phase_color) = match self.boot_phase {
                    BootPhase::Off          => ("Off",          Color32::GRAY),
                    BootPhase::HardwareInit => ("Hardware Init", Color32::from_rgb(100, 150, 255)),
                    BootPhase::KernelLoad   => ("Kernel Load",  Color32::from_rgb(100, 150, 255)),
                    BootPhase::ServiceInit  => ("Service Init", Color32::from_rgb(100, 150, 255)),
                    BootPhase::HomeMenu     => ("Home Menu",    Color32::from_rgb(100, 200, 255)),
                    BootPhase::Running      => ("Running",      Color32::from_rgb(50, 255, 50)),
                    BootPhase::Failed       => ("FAILED",       Color32::from_rgb(255, 50, 50)),
                };
                ui.horizontal(|ui| {
                    ui.label("Boot status:");
                    ui.colored_label(phase_color, phase_text);
                });
            }

            ui.separator();
            ui.label(RichText::new("Output:").color(Color32::from_rgb(200, 200, 200)));
            ScrollArea::vertical().show(ui, |ui| {
                for line in &self.logs {
                    let color = if line.starts_with("Phase") {
                        Color32::from_rgb(120, 180, 255)
                    } else if line.contains("PASS") || line.contains("complete") || line.contains("OK") {
                        Color32::from_rgb(50, 255, 50)
                    } else if line.contains("FAIL") || line.contains("failed") || line.contains("error") {
                        Color32::from_rgb(255, 50, 50)
                    } else if line.starts_with("  ") {
                        Color32::from_rgb(180, 180, 180)
                    } else {
                        Color32::from_rgb(200, 200, 200)
                    };
                    ui.colored_label(color, line);
                }
            });
        });
    }
}
