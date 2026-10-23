//! Classic Minesweeper UI, rendered with egui/eframe.
//!
//! The visual layout mirrors `winmine.exe`: a grey client area with a raised
//! border, a sunken display panel holding the mine counter, the smiley button and
//! the timer, and a sunken 3D board. The game logic lives in [`minesweeper::engine`].

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use minesweeper::engine::{cell, Difficulty, Game, GameState, HighScore, Preset, MAX_HEIGHT, MAX_WIDTH};

/// Geometry constants taken from the original binary.
const CELL: f32 = 16.0;
const BORDER: f32 = 12.0;
const BOARD_TOP: f32 = 55.0;
const MENU_H: f32 = 22.0;

const COL_FACE: Color32 = Color32::from_rgb(0xC0, 0xC0, 0xC0);
const COL_LIGHT: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const COL_DARK: Color32 = Color32::from_rgb(0x80, 0x80, 0x80);
const COL_BLACK: Color32 = Color32::from_rgb(0x00, 0x00, 0x00);
const COL_RED: Color32 = Color32::from_rgb(0xFF, 0x00, 0x00);
const COL_LED_ON: Color32 = Color32::from_rgb(0xFF, 0x00, 0x00);

/// Number colours, indexed by adjacent-mine count (1..=8).
fn number_color(n: u8) -> Color32 {
    match n {
        1 => Color32::from_rgb(0x00, 0x00, 0xFF),
        2 => Color32::from_rgb(0x00, 0x80, 0x00),
        3 => Color32::from_rgb(0xFF, 0x00, 0x00),
        4 => Color32::from_rgb(0x00, 0x00, 0x80),
        5 => Color32::from_rgb(0x80, 0x00, 0x00),
        6 => Color32::from_rgb(0x00, 0x80, 0x80),
        7 => Color32::from_rgb(0x00, 0x00, 0x00),
        8 => Color32::from_rgb(0x80, 0x80, 0x80),
        _ => COL_BLACK,
    }
}

struct Settings {
    difficulty: usize,
    width: usize,
    height: usize,
    mines: usize,
    marks: bool,
    color: bool,
    sound: bool,
    best: [HighScore; 3],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            difficulty: 0,
            width: 9,
            height: 9,
            mines: 10,
            marks: true,
            color: true,
            sound: false,
            best: [HighScore::default(), HighScore::default(), HighScore::default()],
        }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        let base = if cfg!(windows) {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        };
        base.map(|b| b.join("winmine_rs").join("settings.ini"))
    }

    fn load() -> Self {
        let mut s = Settings::default();
        let Some(path) = Self::path() else { return s };
        let Ok(text) = std::fs::read_to_string(path) else { return s };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "Difficulty" => s.difficulty = v.parse().unwrap_or(0),
                "Width" => s.width = v.parse().unwrap_or(9),
                "Height" => s.height = v.parse().unwrap_or(9),
                "Mines" => s.mines = v.parse().unwrap_or(10),
                "Marks" => s.marks = v != "0",
                "Color" => s.color = v != "0",
                "Sound" => s.sound = v != "0",
                "Time1" => s.best[0].time = v.parse().unwrap_or(999),
                "Time2" => s.best[1].time = v.parse().unwrap_or(999),
                "Time3" => s.best[2].time = v.parse().unwrap_or(999),
                "Name1" => s.best[0].name = v.to_string(),
                "Name2" => s.best[1].name = v.to_string(),
                "Name3" => s.best[2].name = v.to_string(),
                _ => {}
            }
        }
        s
    }

    fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let text = format!(
            "Difficulty={}\nWidth={}\nHeight={}\nMines={}\nMarks={}\nColor={}\nSound={}\n\
             Time1={}\nTime2={}\nTime3={}\nName1={}\nName2={}\nName3={}\n",
            self.difficulty,
            self.width,
            self.height,
            self.mines,
            self.marks as u8,
            self.color as u8,
            self.sound as u8,
            self.best[0].time,
            self.best[1].time,
            self.best[2].time,
            sanitize(&self.best[0].name),
            sanitize(&self.best[1].name),
            sanitize(&self.best[2].name),
        );
        let _ = std::fs::write(path, text);
    }
}

fn sanitize(s: &str) -> String {
    s.replace(['\n', '\r', '='], " ")
}

pub struct MinesweeperApp {
    game: Game,
    difficulty: Difficulty,
    settings: Settings,
    seed_counter: u32,

    // dialogs
    show_custom: bool,
    custom_w: i32,
    custom_h: i32,
    custom_m: i32,
    show_best: bool,
    show_name: bool,
    show_about: bool,
    name_input: String,
    pending_best: Option<usize>,
    pending_key_new: bool,

    // input state for chord detection
    press_cell: Option<(usize, usize)>,
    press_chord: bool,
    press_primary: bool,

    // clock
    timer_start: Option<Instant>,

    // xyzzy cheat
    xyzzy_pos: usize,

    last_window: Option<(usize, usize)>,
}

impl MinesweeperApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = Settings::load();
        let difficulty = match settings.difficulty {
            1 => Difficulty::Intermediate,
            2 => Difficulty::Expert,
            3 => Difficulty::Custom,
            _ => Difficulty::Beginner,
        };
        let preset = difficulty.preset(Preset {
            width: settings.width,
            height: settings.height,
            mines: settings.mines,
        });
        let seed = seed();
        let game = Game::new(preset.width, preset.height, preset.mines, seed, settings.marks);

        cc.egui_ctx.set_visuals(egui::Visuals::light());
        cc.egui_ctx.all_styles_mut(|s| {
            s.spacing.item_spacing = Vec2::ZERO;
            s.spacing.window_margin = egui::Margin::same(8);
        });

        Self {
            game,
            difficulty,
            custom_w: settings.width as i32,
            custom_h: settings.height as i32,
            custom_m: settings.mines as i32,
            settings,
            seed_counter: seed.wrapping_add(1),
            show_custom: false,
            show_best: false,
            show_name: false,
            show_about: false,
            name_input: String::new(),
            pending_best: None,
            pending_key_new: false,
            press_cell: None,
            press_chord: false,
            press_primary: false,
            timer_start: None,
            xyzzy_pos: 0,
            last_window: None,
        }
    }

    fn base_size(&self) -> (f32, f32) {
        let w = self.game.width as f32 * CELL + 2.0 * BORDER;
        let h = self.game.height as f32 * CELL + 0x43 as f32;
        (w, h)
    }

    fn screen_size(&self) -> (f32, f32) {
        self.base_size()
    }

    fn new_game(&mut self) {
        let preset = self.difficulty.preset(Preset {
            width: self.settings.width,
            height: self.settings.height,
            mines: self.settings.mines,
        });
        self.settings.width = preset.width;
        self.settings.height = preset.height;
        self.settings.mines = preset.mines;
        let seed = self.seed_counter;
        self.seed_counter = self.seed_counter.wrapping_add(1);
        self.game = Game::new(preset.width, preset.height, preset.mines, seed, self.settings.marks);
        self.timer_start = None;
        self.press_cell = None;
        self.xyzzy_pos = 0;
    }

    fn set_difficulty(&mut self, d: Difficulty) {
        self.difficulty = d;
        self.settings.difficulty = d.index();
        if d != Difficulty::Custom {
            let p = d.preset(Preset { width: 0, height: 0, mines: 0 });
            self.settings.width = p.width;
            self.settings.height = p.height;
            self.settings.mines = p.mines;
        }
        self.settings.save();
        self.new_game();
    }

    fn update_timer(&mut self) {
        if self.game.timer_running {
            if self.timer_start.is_none() {
                self.timer_start = Some(Instant::now());
            }
            if let Some(t0) = self.timer_start {
                let secs = t0.elapsed().as_secs() as i32;
                self.game.time = (1 + secs).min(999);
            }
        } else {
            self.timer_start = None;
        }
    }

    fn cell_at(&self, area: Rect, pos: Pos2, scale: f32) -> Option<(usize, usize)> {
        let local = pos - area.min;
        let gx = ((local.x - BORDER * scale) / (CELL * scale)).floor();
        let gy = ((local.y - BOARD_TOP * scale) / (CELL * scale)).floor();
        if gx < 0.0 || gy < 0.0 {
            return None;
        }
        let (gx, gy) = (gx as usize, gy as usize);
        if gx < self.game.width && gy < self.game.height {
            Some((gx + 1, gy + 1))
        } else {
            None
        }
    }

    fn cell_rect(&self, area: Rect, x: usize, y: usize, scale: f32) -> Rect {
        let px = area.min.x + BORDER * scale + ((x as f32 - 1.0) * CELL * scale);
        let py = area.min.y + BOARD_TOP * scale + ((y as f32 - 1.0) * CELL * scale);
        Rect::from_min_size(Pos2::new(px, py), Vec2::splat(CELL * scale))
    }

    fn handle_input(&mut self, ui: &egui::Ui, area: Rect, scale: f32) {
        let (
            primary_down,
            secondary_down,
            primary_pressed,
            secondary_pressed,
            primary_released,
            secondary_released,
            shift,
            pointer,
        ) = ui.input(|i| {
            (
                i.pointer.primary_down(),
                i.pointer.secondary_down(),
                i.pointer.button_pressed(egui::PointerButton::Primary),
                i.pointer.button_pressed(egui::PointerButton::Secondary),
                i.pointer.button_released(egui::PointerButton::Primary),
                i.pointer.button_released(egui::PointerButton::Secondary),
                i.modifiers.shift,
                i.pointer.interact_pos(),
            )
        });

        // Face button: click restarts the game.
        let (w_base, _) = self.base_size();
        let w = w_base * scale;
        let face_rect = Rect::from_min_size(
            Pos2::new(area.min.x + ((w - 24.0 * scale) * 0.5).floor(), area.min.y + 15.0 * scale),
            Vec2::new(24.0 * scale, 24.0 * scale),
        );
        if primary_pressed {
            if let Some(p) = pointer {
                if face_rect.contains(p) {
                    self.new_game();
                    return;
                }
            }
        }

        let hover = pointer.and_then(|p| self.cell_at(area, p, scale));

        if primary_pressed {
            self.press_primary = true;
            self.press_cell = hover;
            self.press_chord = secondary_down || shift;
        }
        if secondary_pressed {
            self.press_cell = hover;
            self.press_chord = primary_down || shift;
        }
        if primary_down && secondary_down && self.press_cell == hover {
            self.press_chord = true;
        }

        if primary_released {
            if let (Some(c), Some(h)) = (self.press_cell, hover) {
                if c == h {
                    if self.press_chord {
                        self.game.click(c.0, c.1, true);
                    } else if self.press_primary {
                        self.game.click(c.0, c.1, false);
                    }
                    self.after_action();
                }
            }
            self.press_primary = false;
            self.press_cell = None;
            self.press_chord = false;
        }
        if secondary_released {
            if !primary_down {
                if let (Some(c), Some(h)) = (self.press_cell, hover) {
                    if c == h {
                        if self.press_chord {
                            self.game.click(c.0, c.1, true);
                        } else {
                            self.game.mark(c.0, c.1);
                        }
                        self.after_action();
                    }
                }
                self.press_cell = None;
                self.press_chord = false;
            }
        }
        if !primary_down && !secondary_down {
            self.press_cell = None;
            self.press_chord = false;
            self.press_primary = false;
        }
    }

    /// Runs after a player action: handles the win/lose transitions and sounds.
    fn after_action(&mut self) {
        if self.game.state == GameState::Playing {
            return;
        }
        // Record a best time for standard levels.
        if self.game.state == GameState::Won && self.difficulty != Difficulty::Custom {
            let idx = self.difficulty.index();
            if self.game.time < self.settings.best[idx].time {
                self.settings.best[idx].time = self.game.time;
                self.name_input = self.settings.best[idx].name.clone();
                self.pending_best = Some(idx);
                self.show_name = true;
            }
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            // F2 -> new game.
            if i.key_pressed(egui::Key::F2) {
                self.pending_key_new = true;
            }
            // Track the "xyzzy" cheat sequence.
            for event in &i.events {
                if let egui::Event::Text(t) = event {
                    for ch in t.chars() {
                        let secret = ['X', 'Y', 'Z', 'Z', 'Y'];
                        if self.xyzzy_pos < secret.len()
                            && ch.to_ascii_uppercase() == secret[self.xyzzy_pos]
                        {
                            self.xyzzy_pos += 1;
                        } else {
                            self.xyzzy_pos = 0;
                        }
                    }
                }
            }
        });
        if self.pending_key_new {
            self.pending_key_new = false;
            self.new_game();
            self.settings.save();
        }
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Game", |ui| {
                if ui.button("New\tF2").clicked() {
                    self.new_game();
                    self.settings.save();
                    ui.close();
                }
                ui.separator();
                let cur = self.difficulty;
                if ui.radio(cur == Difficulty::Beginner, "Beginner").clicked() {
                    self.set_difficulty(Difficulty::Beginner);
                    ui.close();
                }
                if ui.radio(cur == Difficulty::Intermediate, "Intermediate").clicked() {
                    self.set_difficulty(Difficulty::Intermediate);
                    ui.close();
                }
                if ui.radio(cur == Difficulty::Expert, "Expert").clicked() {
                    self.set_difficulty(Difficulty::Expert);
                    ui.close();
                }
                if ui.radio(cur == Difficulty::Custom, "Custom...").clicked() {
                    self.custom_w = self.settings.width as i32;
                    self.custom_h = self.settings.height as i32;
                    self.custom_m = self.settings.mines as i32;
                    self.show_custom = true;
                    ui.close();
                }
                ui.separator();
                if ui.checkbox(&mut self.settings.marks, "Marks (?)").changed() {
                    self.game.marks_enabled = self.settings.marks;
                    self.settings.save();
                }
                ui.checkbox(&mut self.settings.color, "Color");
                if ui.checkbox(&mut self.settings.sound, "Sound").changed() {
                    self.settings.save();
                }
                ui.separator();
                if ui.button("Best Times...").clicked() {
                    self.show_best = true;
                    ui.close();
                }
                ui.separator();
                if ui.button("Exit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Contents\tF1").clicked() {
                    self.show_about = true;
                    ui.close();
                }
                if ui.button("Search for Help on...").clicked() {
                    self.show_about = true;
                    ui.close();
                }
                if ui.button("Using Help").clicked() {
                    self.show_about = true;
                    ui.close();
                }
                ui.separator();
                if ui.button("About Minesweeper...").clicked() {
                    self.show_about = true;
                    ui.close();
                }
            });
        });
    }
}

impl eframe::App for MinesweeperApp {
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.settings.save();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.update_timer();

        // Resize the window when the board dimensions change.
        let (w, h) = self.screen_size();
        if self.last_window != Some((self.game.width, self.game.height)) {
            self.last_window = Some((self.game.width, self.game.height));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                w,
                h + MENU_H,
            )));
        }

        self.handle_keys(ui.ctx());

        ui.painter().rect_filled(
            Rect::from_min_size(Pos2::ZERO, Vec2::new(w, h + MENU_H)),
            0.0,
            COL_FACE,
        );

        self.menu(ui);

        let (area, _resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());

        self.handle_input(ui, area);
        self.update_timer();

        // ---- draw ----
        let p = ui.painter_at(area);
        let ox = area.min.x;
        let oy = area.min.y;
        let r = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(Pos2::new(ox + x, oy + y), Vec2::new(w, h));

        bevel(&p, r(0.0, 0.0, w, h), true, 3.0); // outer raised border

        let face_x = ((w - 24.0) * 0.5).floor();
        // Top display panel (sunken).
        bevel(&p, r(9.0, 9.0, w - 19.0, 36.0), false, 2.0);
        // Counter well.
        bevel(&p, r(16.0, 15.0, 40.0, 24.0), false, 1.0);
        // Timer well.
        bevel(&p, r(w - 57.0, 15.0, 40.0, 24.0), false, 1.0);
        // Board well (sunken).
        bevel(&p, r(9.0, 52.0, w - 19.0, h - 61.0), false, 3.0);
        // Face button (raised).
        bevel(&p, r(face_x - 1.0, 15.0, 26.0, 26.0), true, 2.0);

        // Counter and timer LED displays.
        draw_led_group(&p, r(16.0, 15.0, 40.0, 24.0), self.game.counter);
        draw_led_group(&p, r(w - 57.0, 15.0, 40.0, 24.0), self.game.time);

        // Face.
        draw_face(&p, r(face_x, 16.0, 24.0, 24.0), self.game.face);

        // Cells.
        for y in 1..=self.game.height {
            for x in 1..=self.game.width {
                let rect = self.cell_rect(area, x, y);
                draw_cell(&p, rect, self.game.sprite(x, y));
            }
        }

        // Press preview (highlight the cell currently held down on).
        if let (Some(c), true) = (self.press_cell, true) {
            if self.game.is_playing() {
                let rect = self.cell_rect(area, c.0, c.1);
                let sprite = self.game.sprite(c.0, c.1);
                if sprite == cell::V_COVERED {
                    p.rect_filled(rect, 0.0, COL_FACE);
                    bevel(&p, rect, false, 2.0);
                } else if sprite == cell::V_QUESTION {
                    p.rect_filled(rect, 0.0, COL_FACE);
                    bevel(&p, rect, false, 2.0);
                    p.text(rect.center(), Align2::CENTER_CENTER, "?", FontId::proportional(12.0), COL_BLACK);
                }
            }
        }

        // xyzzy cheat: highlight the mine under the cursor with a dot in the corner.
        if self.xyzzy_pos >= 5 {
            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                if let Some((x, y)) = self.cell_at(area, pos) {
                    if self.game.cell(x, y) & cell::MINE != 0 {
                        p.rect_filled(
                            Rect::from_min_size(area.min + Vec2::new(1.0, 1.0), Vec2::splat(4.0)),
                            0.0,
                            COL_RED,
                        );
                    }
                }
            }
        }

        // Keep repainting while the clock runs so the timer updates.
        if self.game.timer_running {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }

        // ---- dialogs ----
        self.custom_dialog(ui.ctx());
        self.best_times_dialog(ui.ctx());
        self.enter_name_dialog(ui.ctx());
        self.about_dialog(ui.ctx());
    }
}

/// Extra fields declared out-of-line to keep the struct readable.
impl MinesweeperApp {
    fn custom_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_custom {
            return;
        }
        let mut open = self.show_custom;
        let mut ok = false;
        egui::Window::new("Custom Field")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                egui::Grid::new("custom_grid").show(ui, |ui| {
                    ui.label("Height:");
                    ui.add(egui::DragValue::new(&mut self.custom_h).range(9..=MAX_HEIGHT as i32));
                    ui.end_row();
                    ui.label("Width:");
                    ui.add(egui::DragValue::new(&mut self.custom_w).range(9..=MAX_WIDTH as i32));
                    ui.end_row();
                    ui.label("Mines:");
                    let max_mines = ((self.custom_w - 1) * (self.custom_h - 1)).clamp(10, 999);
                    self.custom_m = self.custom_m.clamp(10, max_mines);
                    ui.add(egui::DragValue::new(&mut self.custom_m).range(10..=max_mines));
                    ui.end_row();
                });
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        ok = true;
                    }
                    if ui.button("Cancel").clicked() {
                        self.show_custom = false;
                    }
                });
            });
        self.show_custom &= open;
        if ok {
            self.custom_h = self.custom_h.clamp(9, MAX_HEIGHT as i32);
            self.custom_w = self.custom_w.clamp(9, MAX_WIDTH as i32);
            let max_mines = ((self.custom_w - 1) * (self.custom_h - 1)).clamp(10, 999);
            self.custom_m = self.custom_m.clamp(10, max_mines);
            self.settings.width = self.custom_w as usize;
            self.settings.height = self.custom_h as usize;
            self.settings.mines = self.custom_m as usize;
            self.show_custom = false;
            self.difficulty = Difficulty::Custom;
            self.settings.difficulty = Difficulty::Custom.index();
            self.settings.save();
            self.new_game();
        }
    }

    fn best_times_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_best {
            return;
        }
        let mut open = self.show_best;
        let mut reset = false;
        egui::Window::new("Fastest Mine Sweepers")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                egui::Grid::new("best_grid").show(ui, |ui| {
                    for (i, label) in ["Beginner:", "Intermediate:", "Expert:"].iter().enumerate() {
                        ui.label(*label);
                        ui.label(format!("{} seconds", self.settings.best[i].time));
                        ui.label(self.settings.best[i].name.clone());
                        ui.end_row();
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Reset Scores").clicked() {
                        reset = true;
                    }
                    if ui.button("OK").clicked() {
                        self.show_best = false;
                    }
                });
            });
        self.show_best &= open;
        if reset {
            for b in &mut self.settings.best {
                *b = HighScore::default();
            }
            self.settings.save();
        }
    }

    fn enter_name_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_name {
            return;
        }
        let idx = match self.pending_best {
            Some(i) => i,
            None => {
                self.show_name = false;
                return;
            }
        };
        let level = ["beginner", "intermediate", "expert"][idx];
        let mut open = self.show_name;
        let mut ok = false;
        egui::Window::new("Fastest Mine Sweepers")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!(
                    "You have the fastest time\rfor {level} level.\rPlease enter your name."
                ));
                let resp = ui.text_edit_singleline(&mut self.name_input);
                resp.request_focus();
                if ui.button("OK").clicked() {
                    ok = true;
                }
            });
        self.show_name &= open;
        if ok {
            let name = if self.name_input.trim().is_empty() {
                "Anonymous".to_string()
            } else {
                sanitize(&self.name_input)
            };
            self.settings.best[idx].name = name;
            self.settings.save();
            self.show_name = false;
            self.pending_best = None;
            self.show_best = true;
        }
    }

    fn about_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let mut open = self.show_about;
        egui::Window::new("About Minesweeper")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.heading("Minesweeper");
                ui.label("by Robert Donner and Curt Johnson");
                ui.separator();
                ui.label("A faithful Rust port of winmine.exe (5.1), rendered with egui.");
                if ui.button("OK").clicked() {
                    self.show_about = false;
                }
            });
        self.show_about &= open;
    }
}

// ---------------------------------------------------------------------------
// Drawing helpers
// ---------------------------------------------------------------------------

fn seed() -> u32 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    // The original seeds with GetTickCount() & 0xffff.
    (now & 0xffff) as u32
}

/// Draw a layered 3D border. `raised` bevels light on top/left, dark on bottom/right.
fn bevel(p: &egui::Painter, r: Rect, raised: bool, width: f32) {
    let (tl, br) = if raised { (COL_LIGHT, COL_DARK) } else { (COL_DARK, COL_LIGHT) };
    let n = width.max(1.0) as i32;
    for i in 0..n {
        let rr = r.shrink(i as f32);
        let l = rr.left();
        let t = rr.top();
        let ri = rr.right() - 1.0;
        let b = rr.bottom() - 1.0;
        p.line_segment([Pos2::new(l, t), Pos2::new(ri, t)], Stroke::new(1.0, tl));
        p.line_segment([Pos2::new(l, t), Pos2::new(l, b)], Stroke::new(1.0, tl));
        p.line_segment([Pos2::new(l, b), Pos2::new(ri, b)], Stroke::new(1.0, br));
        p.line_segment([Pos2::new(ri, t), Pos2::new(ri, b)], Stroke::new(1.0, br));
    }
}

/// Draw a three-digit 7-segment LED group. `value` may be negative (shows `-`).
fn draw_led_group(p: &egui::Painter, group: Rect, value: i32) {
    p.rect_filled(group, 0.0, COL_BLACK);
    let (d1, d2, d3): (i32, i32, i32);
    if value < 0 {
        let v = (-value).min(99);
        d1 = -1; // minus sign
        d2 = (v % 100) / 10;
        d3 = (v % 100) % 10;
    } else {
        d1 = value / 100;
        d2 = (value % 100) / 10;
        d3 = (value % 100) % 10;
    }
    let dh = group.height() - 2.0;
    let dy = group.top() + 1.0;
    let dw = 13.0;
    let gap = (group.width() - 3.0 * dw) / 4.0;
    let x0 = group.left() + gap;
    draw_digit(p, Rect::from_min_size(Pos2::new(x0, dy), Vec2::new(dw, dh)), d1);
    draw_digit(p, Rect::from_min_size(Pos2::new(x0 + dw + gap, dy), Vec2::new(dw, dh)), d2);
    draw_digit(p, Rect::from_min_size(Pos2::new(x0 + 2.0 * (dw + gap), dy), Vec2::new(dw, dh)), d3);
}

fn segment_on(ch: i32, seg: usize) -> bool {
    // segments: 0=a 1=b 2=c 3=d 4=e 5=f 6=g
    match ch {
        -1 => seg == 6,                       // minus
        0 => matches!(seg, 0 | 1 | 2 | 3 | 4 | 5),
        1 => matches!(seg, 1 | 2),
        2 => matches!(seg, 0 | 1 | 6 | 4 | 3),
        3 => matches!(seg, 0 | 1 | 6 | 2 | 3),
        4 => matches!(seg, 5 | 6 | 1 | 2),
        5 => matches!(seg, 0 | 5 | 6 | 2 | 3),
        6 => matches!(seg, 0 | 5 | 6 | 4 | 2 | 3),
        7 => matches!(seg, 0 | 1 | 2),
        8 => true,
        9 => matches!(seg, 0 | 1 | 2 | 3 | 5 | 6),
        _ => false,
    }
}

fn draw_digit(p: &egui::Painter, r: Rect, ch: i32) {
    let t = 2.0;
    let x0 = r.left() + 1.0;
    let x1 = r.right() - 1.0;
    let y0 = r.top() + 1.0;
    let y1 = r.bottom() - 1.0;
    let ym = r.center().y;
    let s = |a: Pos2, b: Pos2| {
        p.line_segment([a, b], Stroke::new(t, COL_LED_ON));
    };
    if segment_on(ch, 0) {
        s(Pos2::new(x0, y0), Pos2::new(x1, y0));
    }
    if segment_on(ch, 1) {
        s(Pos2::new(x1, y0), Pos2::new(x1, ym));
    }
    if segment_on(ch, 2) {
        s(Pos2::new(x1, ym), Pos2::new(x1, y1));
    }
    if segment_on(ch, 3) {
        s(Pos2::new(x0, y1), Pos2::new(x1, y1));
    }
    if segment_on(ch, 4) {
        s(Pos2::new(x0, ym), Pos2::new(x0, y1));
    }
    if segment_on(ch, 5) {
        s(Pos2::new(x0, y0), Pos2::new(x0, ym));
    }
    if segment_on(ch, 6) {
        s(Pos2::new(x0, ym), Pos2::new(x1, ym));
    }
}

fn draw_cell(p: &egui::Painter, r: Rect, sprite: u8) {
    let covered = matches!(sprite, 0x0d | 0x0e | 0x0f);
    if covered {
        p.rect_filled(r, 0.0, COL_FACE);
        bevel(p, r, true, 2.0);
        match sprite {
            cell::V_FLAG => draw_flag(p, r),
            cell::V_QUESTION => {
                p.text(r.center(), Align2::CENTER_CENTER, "?", FontId::proportional(12.0), COL_BLACK);
            }
            _ => {}
        }
        return;
    }

    // Revealed / flat tiles.
    let is_exploded = sprite == cell::V_EXPLODE;
    if is_exploded {
        p.rect_filled(r, 0.0, COL_RED);
    } else {
        p.rect_filled(r, 0.0, COL_FACE);
        // subtle grid lines on the top/left, like the original
        p.line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, COL_DARK));
        p.line_segment([r.left_top(), r.left_bottom()], Stroke::new(1.0, COL_DARK));
    }

    match sprite {
        1..=8 => {
            p.text(
                r.center(),
                Align2::CENTER_CENTER,
                sprite.to_string(),
                FontId::proportional(12.0),
                number_color(sprite),
            );
        }
        cell::V_MINE | cell::V_EXPLODE => draw_mine(p, r),
        cell::V_WRONG => {
            draw_mine(p, r);
            let a = r.min + Vec2::new(2.0, 2.0);
            let b = r.max - Vec2::new(2.0, 2.0);
            p.line_segment([a, b], Stroke::new(2.0, COL_RED));
            p.line_segment([Pos2::new(a.x, b.y), Pos2::new(b.x, a.y)], Stroke::new(2.0, COL_RED));
        }
        _ => {}
    }
}

fn draw_flag(p: &egui::Painter, r: Rect) {
    let c = r.center();
    let pole_x = c.x + 2.0;
    p.line_segment(
        [Pos2::new(pole_x, r.top() + 3.0), Pos2::new(pole_x, r.bottom() - 3.0)],
        Stroke::new(1.5, COL_BLACK),
    );
    // Triangle flag.
    let tri = vec![
        Pos2::new(pole_x, r.top() + 2.5),
        Pos2::new(r.left() + 3.0, r.top() + 6.0),
        Pos2::new(pole_x, r.top() + 9.0),
    ];
    p.add(egui::Shape::convex_polygon(tri, COL_RED, Stroke::new(1.0, COL_BLACK)));
    // Base.
    p.line_segment(
        [Pos2::new(r.left() + 3.0, r.bottom() - 3.0), Pos2::new(r.right() - 3.0, r.bottom() - 3.0)],
        Stroke::new(2.0, COL_BLACK),
    );
}

fn draw_mine(p: &egui::Painter, r: Rect) {
    let c = r.center();
    let rad = 4.0;
    for ang in [0.0f32, std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_4, -std::f32::consts::FRAC_PI_4] {
        let d = Vec2::new(ang.cos(), ang.sin()) * rad;
        p.line_segment([c - d, c + d], Stroke::new(1.0, COL_BLACK));
    }
    p.circle_filled(c, rad - 0.5, COL_BLACK);
    p.rect_filled(
        Rect::from_min_size(c + Vec2::new(-2.0, -2.0), Vec2::splat(2.0)),
        0.0,
        COL_LIGHT,
    );
}

fn draw_face(p: &egui::Painter, r: Rect, state: u8) {
    let c = r.center();
    let rad = r.width() * 0.5 - 1.0;
    p.circle_filled(c, rad, Color32::from_rgb(0xFF, 0xFF, 0x00));
    p.circle_stroke(c, rad, Stroke::new(1.0, COL_BLACK));

    let eye_dx = 4.0;
    let eye_y = c.y - 3.0;
    match state {
        2 => {
            // dead: X eyes, frown
            for sx in [-1.0f32, 1.0] {
                let ex = c.x + sx * eye_dx;
                p.line_segment(
                    [Pos2::new(ex - 2.0, eye_y - 2.0), Pos2::new(ex + 2.0, eye_y + 2.0)],
                    Stroke::new(1.5, COL_BLACK),
                );
                p.line_segment(
                    [Pos2::new(ex + 2.0, eye_y - 2.0), Pos2::new(ex - 2.0, eye_y + 2.0)],
                    Stroke::new(1.5, COL_BLACK),
                );
            }
            draw_arc(p, c + Vec2::new(0.0, 7.0), 5.0, -1.0, COL_BLACK);
        }
        3 => {
            // win: sunglasses + smile
            sunglasses(p, c, eye_dx, eye_y);
            draw_arc(p, c + Vec2::new(0.0, 2.0), 5.0, 1.0, COL_BLACK);
        }
        1 => {
            // pressed: worried, small o-mouth
            for sx in [-1.0f32, 1.0] {
                p.circle_filled(Pos2::new(c.x + sx * eye_dx, eye_y), 1.3, COL_BLACK);
            }
            p.circle_stroke(c + Vec2::new(0.0, 5.0), 2.2, Stroke::new(1.2, COL_BLACK));
        }
        _ => {
            // normal: dot eyes + smile
            for sx in [-1.0f32, 1.0] {
                p.circle_filled(Pos2::new(c.x + sx * eye_dx, eye_y), 1.3, COL_BLACK);
            }
            draw_arc(p, c + Vec2::new(0.0, 2.0), 5.0, 1.0, COL_BLACK);
        }
    }
}

fn draw_arc(p: &egui::Painter, center: Pos2, radius: f32, dir: f32, color: Color32) {
    // dir > 0: smile (dips down); dir < 0: frown (bulges up).
    let n = 10;
    let mut pts = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let x = center.x - radius + 2.0 * radius * t;
        let y = center.y + dir * (radius * 0.8) * (std::f32::consts::PI * t).sin();
        pts.push(Pos2::new(x, y));
    }
    p.add(egui::Shape::line(pts, Stroke::new(1.4, color)));
}

fn sunglasses(p: &egui::Painter, c: Pos2, eye_dx: f32, eye_y: f32) {
    let w = 6.0;
    for sx in [-1.0f32, 1.0] {
        let ex = c.x + sx * eye_dx;
        p.rect_filled(
            Rect::from_center_size(Pos2::new(ex, eye_y), Vec2::new(w, 3.5)),
            0.0,
            COL_BLACK,
        );
    }
    p.line_segment(
        [Pos2::new(c.x - eye_dx + w * 0.5, eye_y), Pos2::new(c.x + eye_dx - w * 0.5, eye_y)],
        Stroke::new(1.2, COL_BLACK),
    );
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([9.0 * CELL + 2.0 * BORDER, 9.0 * CELL + 0x43 as f32 + MENU_H])
            .with_resizable(true)
            .with_title("Minesweeper"),
        ..Default::default()
    };
    eframe::run_native(
        "Minesweeper",
        options,
        Box::new(|cc| Ok(Box::new(MinesweeperApp::new(cc)))),
    )
}
