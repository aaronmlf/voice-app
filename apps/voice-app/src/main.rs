#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use anyhow::{Context, Result, ensure};
use eframe::egui::{self, Color32, RichText, Vec2};
use serde_json::Value;
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use voice_core::{
    AudioConfig, DeviceInfo, DeviceKind, DspSettings, EFFECT_COUNT, EffectKind, EffectOrder,
    RecordingSource, VoiceBlend, VoiceLayer,
};
use voice_engine::{Paths, ServiceHandle, client, protocol::Command};
use voice_library::{Board, Clip, MAX_PRESET_IMAGE_BYTES, PRESET_IMAGE_SIDE, Preset, PresetImage};
pub mod art;
use art::Icon;
use std::collections::{BTreeMap, BTreeSet};

const BG: Color32 = Color32::from_rgb(11, 14, 15);
const SIDEBAR: Color32 = Color32::from_rgb(21, 25, 26);
const CARD: Color32 = Color32::from_rgb(26, 31, 32);
const LINE: Color32 = Color32::from_rgb(44, 51, 53);
const ACCENT: Color32 = Color32::from_rgb(0, 222, 231);
const TEXT: Color32 = Color32::from_rgb(237, 242, 242);
const MUTED: Color32 = Color32::from_rgb(139, 151, 153);
const PRESET_IMAGE_CACHE_SIZE: usize = 64;
const MAX_PENDING_PRESET_IMAGES: usize = 4;

struct CachedPresetImage {
    id: String,
    texture: Option<egui::TextureHandle>,
}

struct PresetImageReply {
    id: String,
    serial: u64,
    image: Result<Option<egui::ColorImage>, String>,
}

/// Only managed thumbnails are decoded here, on the command worker rather than a UI frame.
fn read_preset_thumbnail(value: &Value) -> Result<Option<egui::ColorImage>> {
    let Some(image) = serde_json::from_value::<Option<PresetImage>>(value.clone())? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    File::open(&image.path)?
        .take(MAX_PRESET_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_PRESET_IMAGE_BYTES as usize,
        "miniatura excede 300 KiB"
    );
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(PRESET_IMAGE_SIDE);
    limits.max_image_height = Some(PRESET_IMAGE_SIDE);
    limits.max_alloc = Some(1024 * 1024);
    reader.limits(limits);
    let image = reader.decode()?.into_rgba8();
    ensure!(
        image.dimensions() == (PRESET_IMAGE_SIDE, PRESET_IMAGE_SIDE),
        "miniatura deve ter 256×256 pixels"
    );
    Ok(Some(egui::ColorImage::from_rgba_unmultiplied(
        [PRESET_IMAGE_SIDE as usize, PRESET_IMAGE_SIDE as usize],
        image.as_raw(),
    )))
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Voices,
    Soundboard,
    Lab,
    Recorder,
    Settings,
}
impl Page {
    fn title(self) -> &'static str {
        match self {
            Self::Voices => "Vozes",
            Self::Soundboard => "Soundboard",
            Self::Lab => "Laboratório de voz",
            Self::Recorder => "Gravador",
            Self::Settings => "Configurações",
        }
    }
}
#[derive(Clone, Copy)]
enum Tag {
    Status,
    Devices,
    Presets,
    Boards,
    Clips,
    Action,
    AudioStart,
    MonitorGain,
    Configure,
    DesktopCapture,
    PresetImage(u64),
}
struct Reply {
    tag: Tag,
    value: Result<Value, String>,
    refresh_library: bool,
    preset_image: Option<PresetImageReply>,
    changed_preset_image: Option<String>,
}

struct App {
    _service: Option<ServiceHandle>,
    paths: Paths,
    page: Page,
    tx: mpsc::SyncSender<(Tag, Command)>,
    rx: mpsc::Receiver<Reply>,
    last_poll: Instant,
    poll_pending: bool,
    last_devices_poll: Instant,
    devices_pending: bool,
    status: Value,
    presets: Vec<Preset>,
    boards: Vec<Board>,
    clips: Vec<Clip>,
    devices: Vec<DeviceInfo>,
    audio_config: AudioConfig,
    audio_config_dirty: bool,
    audio_start_pending: Option<AudioConfig>,
    recorder_source: RecordingSource,
    desktop_output: Option<String>,
    desktop_capture_pending: bool,
    monitor_gain: f32,
    monitor_gain_dirty: bool,
    monitor_gain_pending: bool,
    monitor_gain_dragging: bool,
    monitor_gain_last_edit: Instant,
    settings: DspSettings,
    settings_dirty: bool,
    settings_pending: bool,
    last_edit: Instant,
    board: String,
    new_board: String,
    search: String,
    preset_name: String,
    favorites: BTreeSet<String>,
    voice_filter: u8,
    preset_images: Vec<CachedPresetImage>,
    pending_preset_images: BTreeMap<String, u64>,
    preset_image_serial: u64,
    preset_image_revision: Option<u64>,
    looped: bool,
    overlap: bool,
    notice: Option<(String, bool)>,
    trim_id: Option<String>,
    trim_start: f64,
    trim_end: f64,
    trim_name: String,
    started: Instant,
    smoke_seconds: Option<f32>,
    screenshot: Option<PathBuf>,
    screenshot_requested: bool,
}
impl App {
    fn new(
        cc: &eframe::CreationContext<'_>,
        paths: Paths,
        service: Option<ServiceHandle>,
        smoke_seconds: Option<f32>,
        screenshot: Option<PathBuf>,
        page: Page,
        recorder_source: RecordingSource,
    ) -> Self {
        let mut style = (*cc.egui_ctx.style_of(egui::Theme::Dark)).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = BG;
        style.visuals.window_fill = CARD;
        style.visuals.override_text_color = Some(TEXT);
        style.visuals.selection.bg_fill = Color32::from_rgb(0, 83, 87);
        style.visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(37, 44, 46);
        style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(37, 44, 46);
        style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, LINE);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(45, 56, 59);
        style.spacing.item_spacing = Vec2::new(10., 10.);
        style.spacing.button_padding = Vec2::new(12., 8.);
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(15.));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(28.));
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        cc.egui_ctx.set_style_of(egui::Theme::Dark, style);
        let (tx, requests) = mpsc::sync_channel::<(Tag, Command)>(32);
        let (replies, rx) = mpsc::sync_channel(8);
        let socket = paths.socket.clone();
        let ctx = cc.egui_ctx.clone();
        std::thread::spawn(move || {
            while let Ok((tag, command)) = requests.recv() {
                let requested_image = match &command {
                    Command::PresetImage { id } => Some(id.clone()),
                    _ => None,
                };
                let changed_preset_image = match &command {
                    Command::SetPresetImage { id, .. } | Command::ClearPresetImage { id } => {
                        Some(id.clone())
                    }
                    _ => None,
                };
                let refresh_library = matches!(
                    &command,
                    Command::SavePreset { .. }
                        | Command::ImportPreset { .. }
                        | Command::CreateBoard { .. }
                        | Command::ImportClip { .. }
                        | Command::TrimClip { .. }
                );
                let value = client::send(&socket, command).map_err(|e| format!("{e:#}"));
                let preset_image = requested_image.and_then(|id| {
                    let Tag::PresetImage(serial) = tag else {
                        return None;
                    };
                    let image = value.as_ref().map_err(Clone::clone).and_then(|value| {
                        read_preset_thumbnail(value).map_err(|error| format!("{error:#}"))
                    });
                    Some(PresetImageReply { id, serial, image })
                });
                if replies
                    .send(Reply {
                        tag,
                        value,
                        refresh_library,
                        preset_image,
                        changed_preset_image,
                    })
                    .is_err()
                {
                    break;
                }
                ctx.request_repaint();
            }
        });
        let mut app = Self {
            _service: service,
            paths,
            page,
            tx,
            rx,
            last_poll: Instant::now(),
            poll_pending: false,
            last_devices_poll: Instant::now(),
            devices_pending: false,
            status: Value::Null,
            presets: Vec::new(),
            boards: Vec::new(),
            clips: Vec::new(),
            devices: Vec::new(),
            audio_config: AudioConfig::default(),
            audio_config_dirty: false,
            audio_start_pending: None,
            recorder_source,
            desktop_output: None,
            desktop_capture_pending: false,
            monitor_gain: 0.5,
            monitor_gain_dirty: false,
            monitor_gain_pending: false,
            monitor_gain_dragging: false,
            monitor_gain_last_edit: Instant::now(),
            settings: DspSettings::default(),
            settings_dirty: false,
            settings_pending: false,
            last_edit: Instant::now(),
            board: "default".into(),
            new_board: String::new(),
            search: String::new(),
            preset_name: String::new(),
            favorites: BTreeSet::new(),
            voice_filter: 0,
            preset_images: Vec::new(),
            pending_preset_images: BTreeMap::new(),
            preset_image_serial: 0,
            preset_image_revision: None,
            looped: false,
            overlap: true,
            notice: None,
            trim_id: None,
            trim_start: 0.,
            trim_end: 1.,
            trim_name: String::new(),
            started: Instant::now(),
            smoke_seconds,
            screenshot,
            screenshot_requested: false,
        };
        if let Ok(bytes) = std::fs::read(app.paths.data.join("ui-favorites.json"))
            && let Ok(ids) = serde_json::from_slice(&bytes)
        {
            app.favorites = ids;
        }
        app.request_status();
        app.send(Tag::Presets, Command::Presets);
        app.send(Tag::Boards, Command::Boards);
        app.send(
            Tag::Clips,
            Command::Clips {
                board_id: "default".into(),
            },
        );
        app.request_devices();
        app
    }
    fn send(&self, tag: Tag, command: Command) {
        let _ = self.tx.try_send((tag, command));
    }
    fn paint_preset_avatar(&mut self, ui: &egui::Ui, rect: egui::Rect, id: &str, selected: bool) {
        if !id.starts_with("builtin.") && self.presets.iter().any(|preset| preset.id == id) {
            if let Some(index) = self.preset_images.iter().position(|image| image.id == id) {
                let cached = self.preset_images.remove(index);
                let texture = cached.texture.as_ref().map(egui::TextureHandle::id);
                self.preset_images.push(cached);
                if let Some(texture) = texture {
                    art::paint_custom_avatar(ui.painter(), rect, texture, selected);
                    return;
                }
            } else if ui.is_rect_visible(rect)
                && !self.pending_preset_images.contains_key(id)
                && self.pending_preset_images.len() < MAX_PENDING_PRESET_IMAGES
            {
                self.preset_image_serial = self.preset_image_serial.wrapping_add(1);
                let serial = self.preset_image_serial;
                if self
                    .tx
                    .try_send((
                        Tag::PresetImage(serial),
                        Command::PresetImage { id: id.into() },
                    ))
                    .is_ok()
                {
                    self.pending_preset_images.insert(id.into(), serial);
                }
            }
        }
        art::paint_avatar(ui.painter(), rect, id, selected);
    }
    fn invalidate_preset_image(&mut self, id: &str) {
        self.preset_images.retain(|image| image.id != id);
        // An older response cannot overwrite a freshly changed image.
        self.pending_preset_images.remove(id);
    }
    fn preset_image_menu(&mut self, ui: &mut egui::Ui, id: &str) {
        let has_image = self
            .preset_images
            .iter()
            .any(|image| image.id == id && image.texture.is_some());
        if ui
            .button(if has_image {
                "Trocar imagem…"
            } else {
                "Escolher imagem…"
            })
            .clicked()
        {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Imagem do preset")
                .add_filter("Imagem", &["png", "jpg", "jpeg", "webp"])
                .pick_file()
            {
                self.action(Command::SetPresetImage {
                    id: id.into(),
                    path,
                });
            }
        }
        if ui
            .add_enabled(has_image, egui::Button::new("Remover imagem"))
            .clicked()
        {
            self.action(Command::ClearPresetImage { id: id.into() });
            ui.close();
        }
    }
    fn action(&mut self, command: Command) {
        let configure = matches!(&command, Command::Configure { .. });
        let desktop = matches!(
            &command,
            Command::StartDesktopCapture { .. } | Command::StopDesktopCapture
        );
        if desktop && self.desktop_capture_pending {
            return;
        }
        let config = match &command {
            Command::Start { config } => {
                if self.audio_start_pending.is_some() {
                    return;
                }
                Some(config.clone())
            }
            _ => None,
        };
        let tag = if configure {
            Tag::Configure
        } else if config.is_some() {
            Tag::AudioStart
        } else if desktop {
            Tag::DesktopCapture
        } else {
            Tag::Action
        };
        if let Err(e) = self.tx.try_send((tag, command)) {
            if configure {
                self.settings_dirty = true;
                self.last_edit = Instant::now();
            }
            self.notice = Some((format!("O serviço está ocupado: {e}"), true));
        } else if configure {
            self.settings_pending = true;
        } else if let Some(config) = config {
            self.audio_start_pending = Some(config);
        } else if desktop {
            self.desktop_capture_pending = true;
        }
    }
    fn request_status(&mut self) {
        if !self.poll_pending && self.tx.try_send((Tag::Status, Command::Status)).is_ok() {
            self.poll_pending = true;
            self.last_poll = Instant::now();
        }
    }
    fn request_devices(&mut self) {
        if !self.devices_pending && self.tx.try_send((Tag::Devices, Command::Devices)).is_ok() {
            self.devices_pending = true;
            self.last_devices_poll = Instant::now();
        }
    }
    fn audio_requested(&self) -> bool {
        self.status["audio_requested"]
            .as_bool()
            .unwrap_or(self.running())
            || self.audio_start_pending.is_some()
    }
    fn virtual_microphone(&self) -> bool {
        self.status["capabilities"]["virtual_microphone"]
            .as_bool()
            .unwrap_or(voice_engine::VIRTUAL_MICROPHONE)
    }
    fn connection_label(&self) -> &'static str {
        if self.status.is_null() {
            "Carregando áudio…"
        } else if self.running() {
            "Áudio conectado"
        } else if self.audio_start_pending.is_some()
            || self.status["audio_connecting"].as_bool().unwrap_or(false)
        {
            "Conectando áudio…"
        } else if self.audio_requested() {
            "Aguardando dispositivos…"
        } else {
            "Áudio desconectado"
        }
    }
    fn route_description(&self) -> String {
        let input = device_selection_label(
            &self.devices,
            DeviceKind::Input,
            self.status["config"]["input_device"].as_str(),
            self.status["resolved_config"]["input_device"].as_str(),
        );
        let output = device_selection_label(
            &self.devices,
            DeviceKind::Output,
            self.status["config"]["output_device"].as_str(),
            self.status["resolved_config"]["output_device"].as_str(),
        );
        format!("Microfone: {input}\nFones: {output}")
    }
    fn monitor_volume(&mut self, ui: &mut egui::Ui, compact: bool) {
        let mut percent = self.monitor_gain * 100.;
        let response = ui
            .scope(|ui| {
                ui.spacing_mut().item_spacing.x = 4.;
                ui.spacing_mut().slider_width = if compact {
                    66.
                } else {
                    (ui.available_width() - 75.).max(100.)
                };
                ui.add(
                    egui::Slider::new(&mut percent, 0.0..=200.)
                        .fixed_decimals(0)
                        .suffix("%"),
                )
                .on_hover_text(
                    "Volume de Ouvir-me nos fones. Não altera a gravação da voz processada.",
                )
            })
            .inner;
        self.monitor_gain_dragging |= response.dragged();
        if response.changed() {
            self.monitor_gain = percent / 100.;
            self.monitor_gain_dirty = true;
            self.monitor_gain_last_edit = Instant::now();
        }
    }
    fn flush_monitor_volume(&mut self) {
        if self.monitor_gain_dirty
            && !self.monitor_gain_pending
            && !self.monitor_gain_dragging
            && self.monitor_gain_last_edit.elapsed() >= Duration::from_millis(180)
        {
            let command = control(None, None, None, Some(self.monitor_gain), None);
            match self.tx.try_send((Tag::MonitorGain, command)) {
                Ok(()) => {
                    self.monitor_gain_dirty = false;
                    self.monitor_gain_pending = true;
                }
                Err(e) => {
                    self.monitor_gain_last_edit = Instant::now();
                    self.notice = Some((format!("O serviço está ocupado: {e}"), true));
                }
            }
        }
    }
    fn selected_id(&self) -> &str {
        self.status["selected_preset"].as_str().unwrap_or(
            if self.settings == DspSettings::default() {
                "builtin.clean"
            } else {
                "custom.current"
            },
        )
    }
    fn running(&self) -> bool {
        self.status["audio_running"].as_bool().unwrap_or(false)
    }
    fn update_replies(&mut self, ctx: &egui::Context) {
        while let Ok(reply) = self.rx.try_recv() {
            if let Some(image) = reply.preset_image {
                if self.pending_preset_images.get(&image.id) == Some(&image.serial) {
                    self.pending_preset_images.remove(&image.id);
                    let serial = image.serial;
                    let texture = match image.image {
                        Ok(image) => image.map(|image| {
                            ctx.load_texture(
                                format!("preset-image-{serial}"),
                                image,
                                egui::TextureOptions::LINEAR,
                            )
                        }),
                        Err(error) => {
                            self.notice = Some((
                                format!("Não foi possível carregar a imagem: {error}"),
                                true,
                            ));
                            None
                        }
                    };
                    self.preset_images.retain(|cached| cached.id != image.id);
                    if self.preset_images.len() == PRESET_IMAGE_CACHE_SIZE {
                        self.preset_images.remove(0);
                    }
                    self.preset_images.push(CachedPresetImage {
                        id: image.id,
                        texture,
                    });
                }
                continue;
            }
            if reply.value.is_ok()
                && let Some(id) = &reply.changed_preset_image
            {
                self.invalidate_preset_image(id);
            }
            match reply.value {
                Err(message) => {
                    if matches!(reply.tag, Tag::Status) {
                        self.poll_pending = false;
                    }
                    if matches!(reply.tag, Tag::Devices) {
                        self.devices_pending = false;
                        continue;
                    }
                    if matches!(reply.tag, Tag::AudioStart) {
                        self.audio_start_pending = None;
                    }
                    if matches!(reply.tag, Tag::DesktopCapture) {
                        self.desktop_capture_pending = false;
                        self.request_status();
                    }
                    if matches!(reply.tag, Tag::Configure) {
                        self.settings_pending = false;
                        if !self.settings_dirty
                            && let Ok(settings) =
                                serde_json::from_value(self.status["settings"].clone())
                        {
                            self.settings = settings;
                        }
                    }
                    if matches!(reply.tag, Tag::MonitorGain) {
                        self.monitor_gain_pending = false;
                        if !self.monitor_gain_dirty {
                            self.monitor_gain =
                                self.status["monitor_gain"].as_f64().unwrap_or(0.5) as f32;
                        }
                    }
                    self.notice = Some((message, true));
                }
                Ok(value) => match reply.tag {
                    Tag::Status => {
                        self.poll_pending = false;
                        if let Some(revision) = value["preset_image_revision"].as_u64() {
                            if self
                                .preset_image_revision
                                .is_some_and(|old| old != revision)
                            {
                                self.preset_images.clear();
                                self.pending_preset_images.clear();
                            }
                            self.preset_image_revision = Some(revision);
                        }
                        if !self.settings_dirty
                            && !self.settings_pending
                            && let Ok(v) = serde_json::from_value(value["settings"].clone())
                        {
                            self.settings = v;
                        }
                        if !self.audio_config_dirty
                            && self.audio_start_pending.is_none()
                            && let Ok(v) = serde_json::from_value(value["config"].clone())
                        {
                            self.audio_config = v;
                        }
                        if !self.monitor_gain_dirty
                            && !self.monitor_gain_pending
                            && !self.monitor_gain_dragging
                        {
                            self.monitor_gain =
                                value["monitor_gain"].as_f64().unwrap_or(0.5) as f32;
                        }
                        if value["recording"].as_bool().unwrap_or(false)
                            && let Ok(source) =
                                serde_json::from_value(value["recording_source"].clone())
                        {
                            self.recorder_source = source;
                        }
                        if value["desktop"]["active"].as_bool().unwrap_or(false)
                            && let Some(output) = value["desktop"]["output_device"].as_str()
                        {
                            self.desktop_output = Some(output.into());
                        }
                        if value["last_audio_error"] != self.status["last_audio_error"]
                            && let Some(error) = value["last_audio_error"].as_str()
                        {
                            self.notice = Some((error.into(), true));
                        } else if value["last_audio_error"].is_null()
                            && self.status["last_audio_error"]
                                .as_str()
                                .is_some_and(|error| {
                                    self.notice
                                        .as_ref()
                                        .is_some_and(|(message, _)| message == error)
                                })
                        {
                            self.notice = None;
                        }
                        self.status = value;
                    }
                    Tag::Devices => {
                        self.devices_pending = false;
                        if let Ok(v) = serde_json::from_value(value) {
                            self.devices = v;
                        }
                    }
                    Tag::Presets => {
                        if let Ok(v) = serde_json::from_value(value) {
                            self.presets = v;
                        }
                    }
                    Tag::Boards => {
                        if let Ok(v) = serde_json::from_value(value) {
                            self.boards = v;
                        }
                    }
                    Tag::Clips => {
                        if let Ok(v) = serde_json::from_value(value) {
                            self.clips = v;
                        }
                    }
                    Tag::PresetImage(_) => {}
                    Tag::MonitorGain => {
                        self.monitor_gain_pending = false;
                        self.request_status();
                    }
                    Tag::Configure => {
                        self.settings_pending = false;
                        self.notice = None;
                        self.request_status();
                    }
                    Tag::DesktopCapture => {
                        self.desktop_capture_pending = false;
                        self.notice = None;
                        self.request_status();
                    }
                    Tag::Action | Tag::AudioStart => {
                        if matches!(reply.tag, Tag::AudioStart)
                            && let Some(applied) = self.audio_start_pending.take()
                            && same_audio_config(&self.audio_config, &applied)
                        {
                            self.audio_config_dirty = false;
                        }
                        self.notice = None;
                        self.request_status();
                        if reply.refresh_library {
                            self.send(Tag::Presets, Command::Presets);
                            self.send(Tag::Boards, Command::Boards);
                            self.send(
                                Tag::Clips,
                                Command::Clips {
                                    board_id: self.board.clone(),
                                },
                            );
                        }
                    }
                },
            }
        }
    }
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("rail")
            .exact_size(66.)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(SIDEBAR)
                    .inner_margin(egui::Margin::symmetric(10, 16)),
            )
            .show(ui, |ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(44., 42.), egui::Sense::hover());
                art::paint_icon(ui.painter(), rect.shrink(6.), Icon::Wave, ACCENT);
                ui.add_space(20.);
                for (page, icon) in [
                    (Page::Voices, Icon::Mic),
                    (Page::Soundboard, Icon::Grid),
                    (Page::Lab, Icon::Flask),
                    (Page::Recorder, Icon::Record),
                ] {
                    if icon_button(ui, icon, self.page == page, page.title()).clicked() {
                        self.page = page;
                    }
                    ui.add_space(7.);
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    if icon_button(
                        ui,
                        Icon::Settings,
                        self.page == Page::Settings,
                        "Configurações",
                    )
                    .clicked()
                    {
                        self.page = Page::Settings;
                    }
                });
            });
        if matches!(self.page, Page::Voices | Page::Soundboard) {
            egui::Panel::left("library_navigation")
                .exact_size(186.)
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .fill(Color32::from_rgb(18, 22, 23))
                        .inner_margin(egui::Margin::symmetric(14, 22)),
                )
                .show(ui, |ui| {
                    ui.label(RichText::new("voice app").size(20.).strong());
                    ui.add_space(28.);
                    ui.label(RichText::new("BIBLIOTECA").size(10.).color(MUTED));
                    ui.add_space(5.);
                    if self.page == Page::Voices {
                        for (index, label, count) in [
                            (0, "Todas as vozes", self.presets.len()),
                            (
                                3,
                                "Vozes originais",
                                self.presets
                                    .iter()
                                    .filter(|p| p.id.starts_with("builtin."))
                                    .count(),
                            ),
                            (1, "Favoritas", self.favorites.len()),
                            (
                                2,
                                "Minhas vozes",
                                self.presets
                                    .iter()
                                    .filter(|p| !p.id.starts_with("builtin."))
                                    .count(),
                            ),
                        ] {
                            let active = self.voice_filter == index;
                            let text = format!("{label}   {count}");
                            if ui
                                .add_sized(
                                    [156., 37.],
                                    egui::Button::new(
                                        RichText::new(text).size(13.).color(if active {
                                            TEXT
                                        } else {
                                            MUTED
                                        }),
                                    )
                                    .fill(if active {
                                        Color32::from_rgb(35, 43, 45)
                                    } else {
                                        Color32::TRANSPARENT
                                    })
                                    .stroke(egui::Stroke::NONE),
                                )
                                .clicked()
                            {
                                self.voice_filter = index;
                            }
                        }
                        ui.add_space(23.);
                        ui.separator();
                        ui.add_space(14.);
                        if ui
                            .add_sized([156., 34.], egui::Button::new("+  Criar voz"))
                            .clicked()
                        {
                            self.page = Page::Lab;
                        }
                        if ui
                            .add_sized([156., 34.], egui::Button::new("Importar preset"))
                            .clicked()
                            && let Some(path) = rfd::FileDialog::new()
                                .add_filter("Preset JSON", &["json"])
                                .pick_file()
                        {
                            self.action(Command::ImportPreset { path });
                        }
                    } else {
                        for board in self.boards.clone() {
                            let active = self.board == board.id;
                            if ui
                                .add_sized(
                                    [156., 37.],
                                    egui::Button::new(
                                        RichText::new(if board.id == "default" {
                                            "Meu soundboard"
                                        } else {
                                            &board.name
                                        })
                                        .size(13.)
                                        .color(if active { TEXT } else { MUTED }),
                                    )
                                    .fill(if active {
                                        Color32::from_rgb(35, 43, 45)
                                    } else {
                                        Color32::TRANSPARENT
                                    })
                                    .stroke(egui::Stroke::NONE),
                                )
                                .clicked()
                            {
                                self.board = board.id;
                                self.send(
                                    Tag::Clips,
                                    Command::Clips {
                                        board_id: self.board.clone(),
                                    },
                                );
                            }
                        }
                        ui.add_space(16.);
                        ui.collapsing("Novo painel", |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.new_board)
                                    .hint_text("Nome")
                                    .desired_width(140.),
                            );
                            if ui
                                .add_enabled(
                                    !self.new_board.trim().is_empty(),
                                    egui::Button::new("Criar"),
                                )
                                .clicked()
                            {
                                let name = std::mem::take(&mut self.new_board);
                                self.action(Command::CreateBoard { name });
                            }
                        });
                    }
                });
        }
    }
    fn footer(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("master")
            .exact_size(82.)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(24, 29, 30))
                    .inner_margin(egui::Margin::symmetric(20, 14)),
            )
            .show(ui, |ui| {
                let compact = ui.available_width() < 900.;
                ui.horizontal(|ui| {
                    let selected = self.selected_id().to_owned();
                    let name = self
                        .presets
                        .iter()
                        .find(|p| p.id == selected)
                        .map(|p| voice_name(&p.id, &p.name))
                        .unwrap_or(if selected == "builtin.clean" {
                            "Natural"
                        } else {
                            "Personalizada"
                        })
                        .to_owned();
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(48.), egui::Sense::hover());
                    self.paint_preset_avatar(ui, rect, &selected, true);
                    ui.allocate_ui_with_layout(
                        Vec2::new(if compact { 128. } else { 160. }, 48.),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(&name).size(14.).strong())
                                    .truncate(),
                            )
                            .on_hover_text(&name);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(self.connection_label())
                                        .size(11.)
                                        .color(if self.running() { ACCENT } else { MUTED }),
                                )
                                .truncate(),
                            )
                            .on_hover_text(self.route_description());
                        },
                    );
                    ui.add_space(10.);
                    ui.separator();
                    let mut transform = !self.status["bypass"].as_bool().unwrap_or(false);
                    let mut monitor = self.status["monitor"].as_bool().unwrap_or(false);
                    let mut muted = self.status["muted"].as_bool().unwrap_or(false);
                    if switch(ui, &mut transform, "Modificar voz") {
                        self.action(control(Some(!transform), None, None, None, None));
                    }
                    if switch(ui, &mut monitor, "Ouvir-me") {
                        self.action(control(None, None, Some(monitor), None, None));
                    }
                    self.monitor_volume(ui, true);
                    if icon_button(
                        ui,
                        Icon::Mic,
                        muted,
                        if muted {
                            "Ativar microfone"
                        } else {
                            "Silenciar microfone"
                        },
                    )
                    .clicked()
                    {
                        muted = !muted;
                        self.action(control(None, Some(muted), None, None, None));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_button(ui, Icon::Stop, false, "Parar todos os sons").clicked() {
                            self.action(Command::StopSounds);
                        }
                        if !compact {
                            let peak =
                                self.status["metrics"]["output_peak"].as_f64().unwrap_or(0.) as f32;
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::new(90., 20.), egui::Sense::hover());
                            for i in 0..18 {
                                let x = rect.left() + i as f32 * 5.;
                                let on = self.running() && peak > i as f32 / 18.;
                                ui.painter().rect_filled(
                                    egui::Rect::from_min_size(
                                        egui::pos2(x, rect.top() + 4.),
                                        Vec2::new(3., 12.),
                                    ),
                                    1.,
                                    if on {
                                        ACCENT
                                    } else {
                                        Color32::from_rgb(49, 59, 60)
                                    },
                                );
                            }
                            ui.label(RichText::new("SAÍDA").size(9.).color(MUTED));
                        }
                    });
                });
            });
    }
    fn inspector(&mut self, ui: &mut egui::Ui) {
        if self.page != Page::Voices {
            return;
        }
        egui::Panel::right("voice_inspector")
            .exact_size(242.)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(18, 22, 23))
                    .inner_margin(egui::Margin::same(22)),
            )
            .show(ui, |ui| {
                ui.label(RichText::new("VOZ SELECIONADA").size(10.).color(MUTED));
                ui.add_space(24.);
                let selected = self.selected_id().to_owned();
                let preset = self.presets.iter().find(|p| p.id == selected).cloned();
                ui.vertical_centered(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(124.), egui::Sense::hover());
                    self.paint_preset_avatar(ui, rect, &selected, true);
                    ui.add_space(10.);
                    ui.label(
                        RichText::new(
                            preset
                                .as_ref()
                                .map(|p| voice_name(&p.id, &p.name))
                                .unwrap_or(if selected == "builtin.clean" {
                                    "Natural"
                                } else {
                                    "Personalizada"
                                }),
                        )
                        .size(22.)
                        .strong(),
                    );
                    ui.label(
                        RichText::new(voice_description(&selected))
                            .size(12.)
                            .color(MUTED),
                    );
                });
                if preset
                    .as_ref()
                    .is_some_and(|preset| !preset.id.starts_with("builtin."))
                {
                    ui.add_space(12.);
                    ui.menu_button("Imagem do preset", |ui| {
                        self.preset_image_menu(ui, &selected)
                    });
                }
                ui.add_space(24.);
                ui.separator();
                ui.add_space(16.);
                ui.label(RichText::new("Ganho da voz").size(12.).color(MUTED));
                let mut gain = self.settings.gain_db;
                if ui
                    .add(egui::Slider::new(&mut gain, -24.0..=18.0).suffix(" dB"))
                    .changed()
                {
                    self.settings.gain_db = gain;
                    self.settings_dirty = true;
                    self.last_edit = Instant::now();
                }
                ui.add_space(14.);
                if ui
                    .add_sized([198., 36.], egui::Button::new("Editar no laboratório"))
                    .clicked()
                {
                    self.page = Page::Lab;
                }
                if ui
                    .add_enabled(
                        preset.is_some(),
                        egui::Button::new(if self.favorites.contains(&selected) {
                            "Remover dos favoritos"
                        } else {
                            "Adicionar aos favoritos"
                        }),
                    )
                    .clicked()
                {
                    self.toggle_favorite(&selected);
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.label(
                        RichText::new(if self.virtual_microphone() {
                            "Entrada do aplicativo de chamada:"
                        } else {
                            "Saída desta versão:"
                        })
                        .size(10.)
                        .color(MUTED),
                    );
                    ui.label(
                        RichText::new(if self.virtual_microphone() {
                            "Voice App Virtual Microphone"
                        } else {
                            "Monitoração e gravação locais"
                        })
                        .size(11.),
                    );
                });
            });
    }
    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let search_width = (ui.available_width() - 158.).max(180.);
            if matches!(self.page, Page::Voices | Page::Soundboard) {
                egui::Frame::new()
                    .fill(Color32::from_rgb(26, 32, 33))
                    .corner_radius(18)
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(16.), egui::Sense::hover());
                            art::paint_icon(ui.painter(), rect, Icon::Search, MUTED);
                            ui.add(
                                egui::TextEdit::singleline(&mut self.search)
                                    .hint_text(if self.page == Page::Voices {
                                        "Buscar vozes"
                                    } else {
                                        "Buscar sons"
                                    })
                                    .frame(egui::Frame::new())
                                    .desired_width(search_width - 55.),
                            );
                        });
                    });
            } else {
                ui.label(RichText::new(self.page.title()).size(20.).strong());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let requested = self.audio_requested();
                if ui
                    .add_enabled(
                        !self.status.is_null(),
                        egui::Button::new(
                            RichText::new(if self.running() {
                                "Desconectar"
                            } else if requested {
                                "Cancelar conexão"
                            } else {
                                "Conectar áudio"
                            })
                            .size(12.)
                            .color(if requested { TEXT } else { BG }),
                        )
                        .fill(if requested { CARD } else { ACCENT })
                        .corner_radius(20),
                    )
                    .clicked()
                {
                    if requested {
                        self.action(Command::Stop);
                    } else {
                        self.action(Command::Start {
                            config: self.audio_config.clone(),
                        });
                    }
                }
            });
        });
        ui.add_space(22.);
        if let Some((message, error)) = self.notice.clone() {
            egui::Frame::new()
                .fill(if error {
                    Color32::from_rgb(54, 30, 33)
                } else {
                    CARD
                })
                .inner_margin(egui::Margin::same(10))
                .corner_radius(5)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(message).size(12.).color(if error {
                            Color32::from_rgb(255, 169, 177)
                        } else {
                            TEXT
                        }));
                        if ui.small_button("×").clicked() {
                            self.notice = None;
                        }
                    });
                });
            ui.add_space(10.);
        }
    }
    fn toggle_favorite(&mut self, id: &str) {
        let mut candidate = self.favorites.clone();
        if !candidate.remove(id) {
            candidate.insert(id.into());
        }
        let result = serde_json::to_vec(&candidate)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| {
                let path = self.paths.data.join("ui-favorites.json");
                voice_engine::persist_private_file(&path, &bytes)
            });
        if let Err(e) = result {
            self.notice = Some((format!("Não foi possível salvar favoritos: {e}"), true));
        } else {
            self.favorites = candidate;
        }
    }
    fn voices(&mut self, ui: &mut egui::Ui) {
        let selected = self.selected_id().to_owned();
        let presets: Vec<_> = self
            .presets
            .iter()
            .filter(|p| {
                let matches_search = format!("{} {}", p.name, voice_name(&p.id, &p.name))
                    .to_lowercase()
                    .contains(&self.search.to_lowercase());
                matches_search
                    && match self.voice_filter {
                        1 => self.favorites.contains(&p.id),
                        2 => !p.id.starts_with("builtin."),
                        3 => p.id.starts_with("builtin."),
                        _ => true,
                    }
            })
            .cloned()
            .collect();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(match self.voice_filter {
                    1 => "Favoritas",
                    2 => "Minhas vozes",
                    3 => "Vozes originais",
                    _ => "Todas as vozes",
                })
                .size(20.)
                .strong(),
            );
            ui.label(
                RichText::new(format!("{}", presets.len()))
                    .size(12.)
                    .color(MUTED),
            );
        });
        ui.add_space(24.);
        if presets.is_empty() {
            ui.add_space(40.);
            ui.label(RichText::new("Nenhuma voz nesta seleção.").color(MUTED));
            return;
        }
        let columns = ((ui.available_width() / 132.).floor() as usize).clamp(2, 6);
        let width = (ui.available_width() - (columns - 1) as f32 * 10.) / columns as f32;
        for row in presets.chunks(columns) {
            ui.horizontal_top(|ui| {
                for preset in row {
                    ui.allocate_ui_with_layout(
                        Vec2::new(width, 170.),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            let active = selected == preset.id;
                            let (rect, response) =
                                ui.allocate_exact_size(Vec2::splat(108.), egui::Sense::click());
                            self.paint_preset_avatar(ui, rect, &preset.id, active);
                            let response = response
                                .on_hover_text("Selecionar voz")
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            if !preset.id.starts_with("builtin.") {
                                response.context_menu(|ui| self.preset_image_menu(ui, &preset.id));
                            }
                            if response.clicked() {
                                self.settings_dirty = false;
                                self.action(Command::SelectPreset {
                                    id: preset.id.clone(),
                                });
                            }
                            ui.add_space(7.);
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(voice_name(&preset.id, &preset.name))
                                            .size(13.)
                                            .color(if active { ACCENT } else { TEXT }),
                                    )
                                    .frame(false),
                                )
                                .clicked()
                            {
                                self.settings_dirty = false;
                                self.action(Command::SelectPreset {
                                    id: preset.id.clone(),
                                });
                            }
                            let custom = !preset.id.starts_with("builtin.");
                            ui.allocate_ui_with_layout(
                                Vec2::new(if custom { 100. } else { 24. }, 24.),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    let fav = self.favorites.contains(&preset.id);
                                    let (rect, response) = ui.allocate_exact_size(
                                        Vec2::new(24., 18.),
                                        egui::Sense::click(),
                                    );
                                    art::paint_icon(
                                        ui.painter(),
                                        rect.shrink(2.),
                                        Icon::Heart,
                                        if fav {
                                            ACCENT
                                        } else {
                                            Color32::from_rgb(74, 87, 89)
                                        },
                                    );
                                    if response
                                        .on_hover_text(if fav {
                                            "Remover dos favoritos"
                                        } else {
                                            "Favoritar"
                                        })
                                        .clicked()
                                    {
                                        self.toggle_favorite(&preset.id);
                                    }
                                    if custom {
                                        ui.menu_button("Imagem", |ui| {
                                            self.preset_image_menu(ui, &preset.id)
                                        });
                                    }
                                },
                            );
                        },
                    );
                }
            });
            ui.add_space(24.);
        }
    }
    fn soundboard(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(
                    self.boards
                        .iter()
                        .find(|b| b.id == self.board)
                        .map(|b| {
                            if b.id == "default" {
                                "Meu soundboard"
                            } else {
                                b.name.as_str()
                            }
                        })
                        .unwrap_or("Soundboard"),
                )
                .size(20.)
                .strong(),
            );
            ui.label(
                RichText::new(format!("{} sons", self.clips.len()))
                    .size(12.)
                    .color(MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("+  Adicionar som").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Áudio", &["wav", "mp3", "ogg", "flac"])
                        .pick_file()
                {
                    self.import_clip(path);
                }
            });
        });
        ui.add_space(10.);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.looped, "Repetir");
            ui.checkbox(&mut self.overlap, "Sobrepor sons");
        });
        ui.add_space(22.);
        let clips: Vec<_> = self
            .clips
            .iter()
            .filter(|c| c.name.to_lowercase().contains(&self.search.to_lowercase()))
            .cloned()
            .collect();
        let columns = ((ui.available_width() / 152.).floor() as usize).clamp(2, 6);
        let width = (ui.available_width() - (columns - 1) as f32 * 14.) / columns as f32;
        for row in clips.chunks(columns) {
            ui.horizontal_top(|ui| {
                for clip in row {
                    ui.allocate_ui_with_layout(
                        Vec2::new(width, 172.),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            let (rect, response) = ui.allocate_exact_size(
                                Vec2::new(width.min(144.), 116.),
                                egui::Sense::click(),
                            );
                            let color = Color32::from_rgb(44, 68, 75);
                            ui.painter().rect_filled(
                                rect,
                                10.,
                                if response.hovered() {
                                    color
                                } else {
                                    Color32::from_rgb(31, 44, 49)
                                },
                            );
                            art::paint_icon(
                                ui.painter(),
                                egui::Rect::from_center_size(rect.center(), Vec2::splat(45.)),
                                Icon::Wave,
                                Color32::from_rgb(112, 184, 193),
                            );
                            ui.painter().text(
                                rect.right_bottom() - Vec2::new(8., 8.),
                                egui::Align2::RIGHT_BOTTOM,
                                format!("{:.1}s", clip.duration_seconds),
                                egui::FontId::proportional(10.),
                                MUTED,
                            );
                            if response
                                .clone()
                                .on_hover_text("Reproduzir · botão direito para opções")
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                self.play(&clip.id, false);
                            }
                            response.context_menu(|ui| {
                                if ui.button("Pré-escutar nos fones").clicked() {
                                    self.play(&clip.id, true);
                                    ui.close();
                                }
                                if ui.button("Cortar áudio").clicked() {
                                    self.trim_id = Some(clip.id.clone());
                                    self.trim_start = 0.;
                                    self.trim_end = clip.duration_seconds;
                                    self.trim_name = format!("{} — corte", clip.name);
                                    ui.close();
                                }
                            });
                            ui.add_space(5.);
                            ui.label(RichText::new(&clip.name).size(13.));
                        },
                    );
                }
            });
            ui.add_space(12.);
        }
        if self.clips.is_empty() {
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(144., 116.), egui::Sense::click());
            ui.painter().rect_filled(rect, 10., CARD);
            ui.painter().rect_stroke(
                rect,
                10.,
                egui::Stroke::new(1., LINE),
                egui::StrokeKind::Inside,
            );
            art::paint_icon(
                ui.painter(),
                egui::Rect::from_center_size(rect.center(), Vec2::splat(28.)),
                Icon::Plus,
                MUTED,
            );
            if response
                .on_hover_text("Importar arquivo de áudio")
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Áudio", &["wav", "mp3", "ogg", "flac"])
                    .pick_file()
            {
                self.import_clip(path);
            }
            ui.add_space(12.);
            ui.label(
                RichText::new("Adicione um som ou arraste um arquivo para este painel.")
                    .size(12.)
                    .color(MUTED),
            );
        }
        if let Some(id) = self.trim_id.clone() {
            egui::Window::new("Cortar áudio")
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.label("O corte será salvo como um novo som.");
                    ui.add_space(12.);
                    ui.horizontal(|ui| {
                        ui.label("Início (s)");
                        ui.add(egui::DragValue::new(&mut self.trim_start).speed(0.1));
                        ui.label("Fim (s)");
                        ui.add(egui::DragValue::new(&mut self.trim_end).speed(0.1));
                    });
                    ui.text_edit_singleline(&mut self.trim_name);
                    ui.horizontal(|ui| {
                        if ui.button("Salvar corte").clicked() {
                            self.action(Command::TrimClip {
                                id,
                                start_seconds: self.trim_start,
                                end_seconds: self.trim_end,
                                name: self.trim_name.clone(),
                            });
                            self.trim_id = None;
                        }
                        if ui.button("Cancelar").clicked() {
                            self.trim_id = None;
                        }
                    });
                });
        }
    }
    fn import_clip(&mut self, path: PathBuf) {
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.action(Command::ImportClip {
            path,
            board_id: self.board.clone(),
            name,
        });
    }
    fn play(&mut self, id: &str, preview: bool) {
        self.action(Command::Play {
            id: id.into(),
            preview,
            looped: self.looped,
            overlap: self.overlap,
            gain: 1.,
        });
    }
    fn lab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.preset_name)
                    .hint_text("Nome do preset")
                    .desired_width(240.),
            );
            if ui
                .add_enabled(
                    !self.preset_name.trim().is_empty(),
                    egui::Button::new("Salvar preset"),
                )
                .clicked()
            {
                self.action(Command::SavePreset {
                    id: format!("custom.{}", stamp()),
                    name: self.preset_name.clone(),
                    settings: self.settings,
                });
            }
            if ui.button("Importar").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Preset JSON", &["json"])
                    .pick_file()
            {
                self.action(Command::ImportPreset { path });
            }
            if ui.button("Exportar").clicked() {
                if let Some(id) = self.status["selected_preset"].as_str().map(str::to_owned) {
                    if let Some(path) = rfd::FileDialog::new().set_file_name("voz.json").save_file()
                    {
                        self.action(Command::ExportPreset { id, path });
                    }
                } else {
                    self.notice = Some(("Selecione um preset antes de exportar.".into(), true));
                }
            }
        });
        if let Some(id) = self.status["selected_preset"]
            .as_str()
            .filter(|id| !id.starts_with("builtin."))
            .map(str::to_owned)
        {
            ui.add_space(10.);
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(40.), egui::Sense::hover());
                self.paint_preset_avatar(ui, rect, &id, true);
                ui.menu_button("Imagem do preset", |ui| self.preset_image_menu(ui, &id));
            });
        }
        ui.add_space(22.);
        let mut changed = voice_blend_editor(ui, &mut self.settings, &self.presets);
        ui.add_space(18.);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(238., 0.),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    changed |= effect_order_editor(
                        ui,
                        &mut self.settings.effect_order,
                        &mut self.settings.effect_mix,
                    );
                },
            );
            ui.add_space(8.);
            let controls_width = ui.available_width().max(280.);
            ui.allocate_ui_with_layout(
                Vec2::new(controls_width, 0.),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.columns(2, |columns| {
                        card().show(&mut columns[0], |ui| {
                            ui.label(RichText::new("VOZ").size(11.).color(ACCENT));
                            ui.add_space(8.);
                            changed |= parameter(
                                ui,
                                "Pitch",
                                &mut self.settings.pitch_semitones,
                                -24.0..=24.0,
                                " st",
                            );
                            changed |= parameter(
                                ui,
                                "Formante",
                                &mut self.settings.formant_semitones,
                                -12.0..=12.0,
                                " st",
                            );
                            changed |= parameter(
                                ui,
                                "Ganho",
                                &mut self.settings.gain_db,
                                -60.0..=24.0,
                                " dB",
                            );
                        });
                        columns[0].add_space(14.);
                        card().show(&mut columns[0], |ui| {
                            ui.label(RichText::new("FILTROS").size(11.).color(ACCENT));
                            ui.add_space(8.);
                            changed |= parameter(
                                ui,
                                "Passa-altas",
                                &mut self.settings.highpass_hz,
                                10.0..=10000.0,
                                " Hz",
                            );
                            changed |= parameter(
                                ui,
                                "Passa-baixas",
                                &mut self.settings.lowpass_hz,
                                100.0..=23000.0,
                                " Hz",
                            );
                            changed |=
                                parameter(ui, "Saturação", &mut self.settings.drive, 0.0..=1.0, "");
                        });
                        columns[0].add_space(14.);
                        card().show(&mut columns[0], |ui| {
                            ui.label(RichText::new("DINÂMICA").size(11.).color(ACCENT));
                            ui.add_space(8.);
                            changed |= parameter(
                                ui,
                                "Gate",
                                &mut self.settings.gate_db,
                                -100.0..=0.0,
                                " dB",
                            );
                            changed |= ui
                                .checkbox(&mut self.settings.compressor, "Compressor")
                                .changed();
                            changed |= ui
                                .checkbox(&mut self.settings.denoise, "Redução de ruído")
                                .changed();
                        });
                        card().show(&mut columns[1], |ui| {
                            ui.label(RichText::new("MODULAÇÃO").size(11.).color(ACCENT));
                            ui.add_space(8.);
                            changed |=
                                parameter(ui, "Robô", &mut self.settings.robot_mix, 0.0..=1.0, "");
                            changed |= parameter(
                                ui,
                                "Frequência",
                                &mut self.settings.robot_hz,
                                1.0..=2000.0,
                                " Hz",
                            );
                            changed |= parameter(
                                ui,
                                "Chorus",
                                &mut self.settings.chorus_mix,
                                0.0..=1.0,
                                "",
                            );
                        });
                        columns[1].add_space(14.);
                        card().show(&mut columns[1], |ui| {
                            ui.label(RichText::new("AMBIÊNCIA").size(11.).color(ACCENT));
                            ui.add_space(8.);
                            changed |=
                                parameter(ui, "Eco", &mut self.settings.echo_mix, 0.0..=1.0, "");
                            changed |= parameter(
                                ui,
                                "Atraso",
                                &mut self.settings.echo_delay_ms,
                                10.0..=1500.0,
                                " ms",
                            );
                            changed |= parameter(
                                ui,
                                "Retorno",
                                &mut self.settings.echo_feedback,
                                0.0..=0.9,
                                "",
                            );
                            changed |= parameter(
                                ui,
                                "Reverb",
                                &mut self.settings.reverb_mix,
                                0.0..=1.0,
                                "",
                            );
                        });
                    });
                },
            );
        });
        if changed {
            self.settings_dirty = true;
            self.last_edit = Instant::now();
        }
    }
    fn recorder(&mut self, ui: &mut egui::Ui) {
        let recording = self.status["recording"].as_bool().unwrap_or(false);
        let desktop_active = self.status["desktop"]["active"].as_bool().unwrap_or(false);
        card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new("Fonte de áudio").size(18.).strong());
            ui.add_space(10.);
            ui.add_enabled_ui(!recording, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut self.recorder_source,
                        RecordingSource::ProcessedVoice,
                        "Voz processada",
                    );
                    ui.selectable_value(
                        &mut self.recorder_source,
                        RecordingSource::Desktop,
                        "Áudio do desktop",
                    );
                });
            });
            ui.add_space(12.);
            if self.recorder_source == RecordingSource::Desktop {
                ui.label(RichText::new("Saída para capturar").size(12.).color(MUTED));
                let selected_text = self
                    .desktop_output
                    .as_ref()
                    .map(|name| {
                        self.devices
                            .iter()
                            .find(|device| {
                                device.kind == DeviceKind::Output && &device.name == name
                            })
                            .map(|device| device.description.clone())
                            .unwrap_or_else(|| format!("{name} (indisponível)"))
                    })
                    .unwrap_or_else(|| "Selecione uma saída de áudio".into());
                ui.add_enabled_ui(!desktop_active && !self.desktop_capture_pending, |ui| {
                    egui::ComboBox::from_id_salt("desktop-output")
                        .width(ui.available_width().min(650.))
                        .truncate()
                        .selected_text(selected_text)
                        .show_ui(ui, |ui| {
                            for device in self
                                .devices
                                .iter()
                                .filter(|device| device.kind == DeviceKind::Output)
                            {
                                ui.selectable_value(
                                    &mut self.desktop_output,
                                    Some(device.name.clone()),
                                    &device.description,
                                );
                            }
                        });
                });
                ui.horizontal(|ui| {
                    if desktop_active {
                        ui.label(
                            RichText::new("Capturando em estéreo")
                                .size(12.)
                                .color(ACCENT),
                        );
                        if ui
                            .add_enabled(
                                !self.desktop_capture_pending,
                                egui::Button::new("Parar captura"),
                            )
                            .clicked()
                        {
                            self.action(Command::StopDesktopCapture);
                        }
                    } else if ui
                        .add_enabled(
                            self.desktop_output.as_ref().is_some_and(|name| {
                                self.devices.iter().any(|device| {
                                    device.kind == DeviceKind::Output && &device.name == name
                                })
                            }) && !self.desktop_capture_pending,
                            egui::Button::new(if self.desktop_capture_pending {
                                "Iniciando captura…"
                            } else {
                                "Ativar captura"
                            }),
                        )
                        .clicked()
                        && let Some(output_device) = self.desktop_output.clone()
                    {
                        self.action(Command::StartDesktopCapture { output_device });
                    }
                });
                ui.label(
                    RichText::new(
                        "Captura o áudio da saída selecionada, incluindo chamadas e Ouvir-me.",
                    )
                    .size(11.)
                    .color(MUTED),
                );
                if desktop_active {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Nível capturado").size(11.).color(MUTED));
                        let peak = self.status["desktop"]["metrics"]["peak"]
                            .as_f64()
                            .unwrap_or(0.)
                            .clamp(0., 1.) as f32;
                        ui.add(
                            egui::ProgressBar::new(peak)
                                .desired_width(120.)
                                .desired_height(6.)
                                .fill(ACCENT),
                        );
                    });
                }
                let overruns = self.status["desktop"]["metrics"]["overruns"]
                    .as_u64()
                    .unwrap_or(0);
                if overruns > 0 {
                    ui.label(
                        RichText::new(format!(
                            "A captura perdeu áudio ({overruns} ocorrências). A gravação e o replay podem conter cortes."
                        ))
                        .size(11.)
                        .color(Color32::from_rgb(255, 184, 112)),
                    );
                }
                if let Some(error) = self.status["desktop"]["error"].as_str() {
                    ui.label(
                        RichText::new(error)
                            .size(11.)
                            .color(Color32::from_rgb(255, 139, 150)),
                    );
                }
            } else {
                ui.label(
                    RichText::new("Microfone com os efeitos e sons do soundboard.")
                        .size(12.)
                        .color(MUTED),
                );
                if desktop_active {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Captura do desktop ativa")
                                .size(11.)
                                .color(ACCENT),
                        );
                        if ui
                            .add_enabled(
                                !self.desktop_capture_pending,
                                egui::Button::new("Parar captura"),
                            )
                            .clicked()
                        {
                            self.action(Command::StopDesktopCapture);
                        }
                    });
                }
            }
        });
        ui.add_space(16.);
        let desktop = self.recorder_source == RecordingSource::Desktop;
        let ready = if desktop {
            desktop_active
        } else {
            self.running()
        };
        let replay_seconds = if desktop {
            self.status["desktop"]["replay_seconds"].as_f64()
        } else {
            self.status["replay_seconds"].as_f64()
        }
        .unwrap_or(0.);
        card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(24.), egui::Sense::hover());
                art::paint_icon(
                    ui.painter(),
                    rect,
                    Icon::Record,
                    if recording {
                        Color32::from_rgb(255, 92, 113)
                    } else {
                        MUTED
                    },
                );
                ui.label(
                    RichText::new(if recording {
                        "Gravação em andamento"
                    } else {
                        "Gravação e replay"
                    })
                    .size(18.)
                    .strong(),
                );
            });
            ui.label(
                RichText::new(if desktop {
                    "Desktop · WAV estéreo · 48 kHz"
                } else {
                    "Voz processada · WAV mono · 48 kHz"
                })
                .size(12.)
                .color(MUTED),
            );
            ui.add_space(16.);
            ui.horizontal_wrapped(|ui| {
                if recording {
                    if ui
                        .add(
                            egui::Button::new("Finalizar gravação")
                                .fill(Color32::from_rgb(101, 36, 49)),
                        )
                        .clicked()
                    {
                        self.action(Command::StopRecording);
                    }
                } else if ui
                    .add_enabled(ready, egui::Button::new("Iniciar gravação"))
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .set_file_name(format!(
                            "{}-{}.wav",
                            if desktop { "desktop" } else { "gravacao" },
                            stamp()
                        ))
                        .save_file()
                {
                    self.action(Command::StartRecording {
                        path,
                        source: self.recorder_source,
                    });
                }
                if ui
                    .add_enabled(replay_seconds > 0., egui::Button::new("Salvar replay"))
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .set_file_name(format!("replay-{}.wav", stamp()))
                        .save_file()
                {
                    self.action(Command::SaveReplay {
                        path,
                        seconds: 30,
                        source: self.recorder_source,
                    });
                }
            });
            if !ready && !recording {
                ui.label(
                    RichText::new(if desktop {
                        "Ative a captura da saída para gravar."
                    } else {
                        "Conecte o áudio para gravar."
                    })
                    .size(11.)
                    .color(MUTED),
                );
            }
            ui.add_space(8.);
            ui.label(
                RichText::new(format!(
                    "Replay disponível: {:.1} s de 30 s",
                    replay_seconds.clamp(0., 30.)
                ))
                .size(12.)
                .color(MUTED),
            );
            if let Some(error) = self.status["recording_error"].as_str() {
                ui.label(
                    RichText::new(error)
                        .size(11.)
                        .color(Color32::from_rgb(255, 139, 150)),
                );
            }
            ui.add_space(12.);
            if ui.button("Importar gravação no soundboard").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Áudio", &["wav"])
                    .pick_file()
            {
                self.import_clip(path);
                self.page = Page::Soundboard;
            }
        });
    }
    fn settings_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Dispositivos de áudio").size(18.).strong());
        ui.add_space(18.);
        card().show(ui, |ui| {
            let resolved_input = self.status["resolved_config"]["input_device"].as_str();
            let resolved_output = self.status["resolved_config"]["output_device"].as_str();
            for (kind, selected, label, icon, resolved) in [
                (
                    DeviceKind::Input,
                    &mut self.audio_config.input_device,
                    "Microfone",
                    Icon::Mic,
                    resolved_input,
                ),
                (
                    DeviceKind::Output,
                    &mut self.audio_config.output_device,
                    "Fones de ouvido",
                    Icon::Headphones,
                    resolved_output,
                ),
            ] {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.), egui::Sense::hover());
                    art::paint_icon(ui.painter(), rect, icon, ACCENT);
                    ui.label(RichText::new(label).size(13.).strong());
                });
                let selected_text = device_selection_label(
                    &self.devices,
                    kind.clone(),
                    selected.as_deref(),
                    resolved,
                );
                let previous = selected.clone();
                egui::ComboBox::from_id_salt(label)
                    .width(ui.available_width().min(570.))
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(selected, None, "Automático (padrão do sistema)");
                        for device in self.devices.iter().filter(|d| d.kind == kind) {
                            ui.selectable_value(
                                selected,
                                Some(device.name.clone()),
                                &device.description,
                            );
                        }
                    });
                if *selected != previous {
                    self.audio_config_dirty = true;
                }
                ui.add_space(16.);
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(!self.devices_pending, egui::Button::new("Atualizar lista")).clicked() {
                    self.request_devices();
                }
                if ui
                    .add_enabled(
                        !self.status.is_null() && self.audio_start_pending.is_none(),
                        egui::Button::new(if self.audio_requested() {
                            "Aplicar e reconectar"
                        } else {
                            "Conectar áudio"
                        }),
                    )
                    .clicked()
                {
                    self.action(Command::Start {
                        config: self.audio_config.clone(),
                    });
                }
            });
            ui.add_space(8.);
            ui.label(
                RichText::new("Automático usa os dispositivos padrão do sistema. A lista é atualizada continuamente.")
                    .size(11.)
                    .color(MUTED),
            );
        });
        ui.add_space(18.);
        card().show(ui, |ui| {
            ui.label(RichText::new("Volumes").size(16.).strong());
            ui.add_space(10.);
            let mut soundboard_gain = self.status["soundboard_gain"].as_f64().unwrap_or(1.) as f32;
            ui.label(RichText::new("Ouvir-me").size(12.).color(MUTED));
            self.monitor_volume(ui, false);
            if parameter(ui, "Soundboard", &mut soundboard_gain, 0.0..=2., "") {
                self.action(control(None, None, None, None, Some(soundboard_gain)));
            }
        });
        ui.add_space(18.);
        ui.label(RichText::new("Microfone virtual").size(16.).strong());
        if self.virtual_microphone() {
            ui.label(
                RichText::new("Voice App Virtual Microphone")
                    .color(ACCENT)
                    .size(13.),
            );
            ui.label(
                RichText::new("Selecione esta entrada no Discord, OBS ou jogo.")
                    .size(12.)
                    .color(MUTED),
            );
        } else {
            ui.label(RichText::new("Ainda não disponível no Windows").size(13.));
            ui.label(
                RichText::new(
                    "Use esta versão para testar efeitos, Ouvir-me e gravação. O envio da voz para chamadas depende do driver virtual.",
                )
                .size(12.)
                .color(MUTED),
            );
        }
        ui.add_space(22.);
        ui.collapsing("Diagnóstico", |ui| {
            ui.label(format!("Biblioteca: {}", self.paths.data.display()));
            ui.label(format!(
                "{} · voz mono · fones estéreo · 48 kHz",
                self.status["backend"]
                    .as_str()
                    .unwrap_or(voice_engine::BACKEND_NAME)
            ));
            if let Some(error) = self.status["last_audio_error"].as_str() {
                ui.colored_label(Color32::LIGHT_RED, error);
            }
            let frames = self.status["metrics"]["algorithmic_latency_frames"]
                .as_u64()
                .unwrap_or(0);
            ui.label(format!(
                "Atraso do algoritmo: {:.1} ms",
                frames as f64 / 48.
            ));
            ui.label(
                RichText::new(
                    "Este valor não inclui a latência do dispositivo e do aplicativo de destino.",
                )
                .size(11.)
                .color(MUTED),
            );
            ui.monospace(serde_json::to_string_pretty(&self.status["metrics"]).unwrap_or_default());
        });
    }
}
impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.update_replies(ui.ctx());
        if self.last_poll.elapsed() > Duration::from_millis(400) && !self.poll_pending {
            self.request_status();
        }
        if self.last_devices_poll.elapsed() > Duration::from_secs(2) {
            self.request_devices();
        }
        if self.settings_dirty
            && !self.settings_pending
            && self.last_edit.elapsed() > Duration::from_millis(180)
        {
            match self.settings.validate() {
                Ok(()) => {
                    self.settings_dirty = false;
                    self.action(Command::Configure {
                        settings: self.settings,
                    });
                }
                Err(e) => {
                    self.notice = Some((e, true));
                    self.settings_dirty = false;
                }
            }
        }
        let dropped = ui.ctx().input(|i| i.raw.dropped_files.clone());
        for dropped in dropped {
            self.import_clip(dropped.path().to_owned());
            self.page = Page::Soundboard;
        }
        self.monitor_gain_dragging = false;
        self.sidebar(ui);
        self.footer(ui);
        self.inspector(ui);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::same(24)),
            )
            .show(ui, |ui| {
                self.header(ui);
                egui::ScrollArea::vertical().show(ui, |ui| match self.page {
                    Page::Voices => self.voices(ui),
                    Page::Soundboard => self.soundboard(ui),
                    Page::Lab => self.lab(ui),
                    Page::Recorder => self.recorder(ui),
                    Page::Settings => self.settings_page(ui),
                });
            });
        self.flush_monitor_volume();
        if !self.screenshot_requested
            && self.started.elapsed() > Duration::from_secs(2)
            && let Some(path) = self.screenshot.clone()
        {
            self.screenshot_requested = true;
            ui.ctx().request_screenshot(move |image| {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                if let Err(e) = image::save_buffer(
                    &path,
                    &bytes,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                ) {
                    eprintln!("screenshot: {e}");
                }
            });
        }
        if self
            .smoke_seconds
            .is_some_and(|s| self.started.elapsed().as_secs_f32() > s)
        {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
}
fn same_audio_config(left: &AudioConfig, right: &AudioConfig) -> bool {
    left.input_device == right.input_device
        && left.output_device == right.output_device
        && left.sample_rate == right.sample_rate
        && left.quantum == right.quantum
}

fn device_selection_label(
    devices: &[DeviceInfo],
    kind: DeviceKind,
    selected: Option<&str>,
    resolved: Option<&str>,
) -> String {
    let description = |name: &str| {
        devices
            .iter()
            .find(|device| {
                device.kind == kind && (device.name == name || device.id.to_string() == name)
            })
            .map(|device| device.description.as_str())
    };
    match selected {
        Some(name) => description(name)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Indisponível · {name}")),
        None => match resolved {
            Some(name) => format!("Automático · {}", description(name).unwrap_or(name)),
            None => "Automático (padrão do sistema)".to_owned(),
        },
    }
}

fn control(
    bypass: Option<bool>,
    muted: Option<bool>,
    monitor: Option<bool>,
    monitor_gain: Option<f32>,
    soundboard_gain: Option<f32>,
) -> Command {
    Command::SetControls {
        bypass,
        muted,
        monitor,
        monitor_gain,
        soundboard_gain,
    }
}
fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(egui::Stroke::new(1., LINE))
        .corner_radius(12)
        .inner_margin(egui::Margin::same(20))
}
fn stamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
fn voice_name<'a>(id: &str, fallback: &'a str) -> &'a str {
    match id {
        "builtin.clean" => "Natural",
        "builtin.radio" => "Rádio",
        "builtin.robot" => "Robô",
        "builtin.echo" => "Eco",
        "builtin.space" => "Espacial",
        "builtin.megaphone" => "Megafone",
        "builtin.deep" => "Grave",
        "builtin.high" => "Aguda",
        "builtin.alien" => "Alien",
        "builtin.chorus" => "Chorus",
        "builtin.broadcast" => "Locução",
        "builtin.monster" => "Monstro",
        _ => fallback,
    }
}
fn voice_description(id: &str) -> &'static str {
    match id {
        "builtin.clean" => "Voz sem efeitos adicionais.",
        "builtin.radio" => "Filtro de rádio e compressão.",
        "builtin.robot" => "Modulação de voz robótica.",
        "builtin.echo" => "Repetições com atraso.",
        "builtin.space" => "Reverb e eco.",
        "builtin.megaphone" => "Banda limitada e saturação.",
        "builtin.deep" => "Pitch e formante graves.",
        "builtin.high" => "Pitch e formante agudos.",
        "builtin.alien" => "Formante elevado e modulação.",
        "builtin.chorus" => "Modulação com vozes em paralelo.",
        "builtin.broadcast" => "Limpeza e compressão de voz.",
        "builtin.monster" => "Pitch grave com saturação.",
        _ => "Preset personalizado.",
    }
}
fn effect_name(effect: EffectKind) -> &'static str {
    match effect {
        EffectKind::Denoise => "Redução de ruído",
        EffectKind::PitchFormant => "Pitch / formante",
        EffectKind::Gate => "Gate",
        EffectKind::Gain => "Ganho",
        EffectKind::Filter => "Filtros",
        EffectKind::Compressor => "Compressor",
        EffectKind::Distortion => "Saturação",
        EffectKind::Robot => "Robô",
        EffectKind::Chorus => "Chorus",
        EffectKind::Echo => "Eco",
        EffectKind::Reverb => "Reverb",
    }
}
fn voice_blend_editor(ui: &mut egui::Ui, settings: &mut DspSettings, presets: &[Preset]) -> bool {
    let mut changed = false;
    card().inner_margin(egui::Margin::same(14)).show(ui, |ui| {
        ui.label(RichText::new("MISTURAR VOZES").size(11.).color(ACCENT));
        let mut enabled = settings.blend.is_some();
        if ui.checkbox(&mut enabled, "Misturar duas vozes").changed() {
            settings.blend = enabled.then(|| VoiceBlend {
                secondary: presets
                    .iter()
                    .filter(|preset| preset.settings.blend.is_none())
                    .map(|preset| VoiceLayer::from(preset.settings))
                    .find(|layer| *layer != VoiceLayer::from(*settings))
                    .unwrap_or_default(),
                ..VoiceBlend::default()
            });
            changed = true;
        }
        let Some(mut blend) = settings.blend else {
            return;
        };
        let primary = VoiceLayer::from(*settings);
        let name_for = |layer: VoiceLayer, fallback: &str| -> String {
            presets
                .iter()
                .find(|preset| {
                    preset.settings.blend.is_none() && VoiceLayer::from(preset.settings) == layer
                })
                .map(|preset| voice_name(&preset.id, &preset.name).to_owned())
                .unwrap_or_else(|| fallback.to_owned())
        };
        ui.add_space(8.);
        ui.columns(2, |columns| {
            columns[0].label(RichText::new("Voz A").size(12.).color(MUTED));
            egui::ComboBox::from_id_salt("blend-primary")
                .width(columns[0].available_width().min(280.))
                .truncate()
                .selected_text(name_for(primary, "Cadeia atual"))
                .show_ui(&mut columns[0], |ui| {
                    ui.label(
                        RichText::new("Usar um preset como voz A")
                            .size(11.)
                            .color(MUTED),
                    );
                    for preset in presets
                        .iter()
                        .filter(|preset| preset.settings.blend.is_none())
                    {
                        let layer = VoiceLayer::from(preset.settings);
                        if ui
                            .selectable_label(
                                layer == primary,
                                voice_name(&preset.id, &preset.name),
                            )
                            .clicked()
                        {
                            *settings = layer.to_settings();
                            changed = true;
                        }
                    }
                });
            let mut percent = blend.primary_mix * 100.;
            if columns[0]
                .add(
                    egui::Slider::new(&mut percent, 0.0..=100.)
                        .text("Nível A")
                        .suffix("%")
                        .fixed_decimals(0),
                )
                .changed()
            {
                blend.primary_mix = percent / 100.;
                changed = true;
            }
            columns[1].label(RichText::new("Voz B").size(12.).color(MUTED));
            egui::ComboBox::from_id_salt("blend-secondary")
                .width(columns[1].available_width().min(280.))
                .truncate()
                .selected_text(name_for(blend.secondary, "Voz personalizada"))
                .show_ui(&mut columns[1], |ui| {
                    for preset in presets
                        .iter()
                        .filter(|preset| preset.settings.blend.is_none())
                    {
                        changed |= ui
                            .selectable_value(
                                &mut blend.secondary,
                                VoiceLayer::from(preset.settings),
                                voice_name(&preset.id, &preset.name),
                            )
                            .changed();
                    }
                });
            let mut percent = blend.secondary_mix * 100.;
            if columns[1]
                .add(
                    egui::Slider::new(&mut percent, 0.0..=100.)
                        .text("Nível B")
                        .suffix("%")
                        .fixed_decimals(0),
                )
                .changed()
            {
                blend.secondary_mix = percent / 100.;
                changed = true;
            }
        });
        ui.collapsing("Intensidade dos efeitos da voz B", |ui| {
            for effect in blend.secondary.effect_order.0 {
                let mut percent = blend.secondary.effect_mix[effect as usize] * 100.;
                if parameter(ui, effect_name(effect), &mut percent, 0.0..=100.0, "%") {
                    blend.secondary.effect_mix[effect as usize] = percent / 100.;
                    changed = true;
                }
            }
        });
        ui.label(
            RichText::new("Os controles abaixo editam a voz A.")
                .size(11.)
                .color(MUTED),
        );
        if blend.primary_mix + blend.secondary_mix > 1.0 {
            ui.label(
                RichText::new("Níveis combinados acima de 100% são equilibrados automaticamente.")
                    .size(11.)
                    .color(MUTED),
            );
        } else if blend.primary_mix == 0.0 && blend.secondary_mix == 0.0 {
            ui.label(
                RichText::new("As duas vozes estão em 0%: saída silenciosa.")
                    .size(11.)
                    .color(MUTED),
            );
        }
        settings.blend = Some(blend);
    });
    changed
}

fn effect_order_editor(
    ui: &mut egui::Ui,
    order: &mut EffectOrder,
    mix: &mut [f32; EFFECT_COUNT],
) -> bool {
    let mut changed = false;
    card().inner_margin(egui::Margin::same(14)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.;
        ui.label(RichText::new("CADEIA DE EFEITOS").size(11.).color(ACCENT));
        ui.label(RichText::new("De cima para baixo").size(11.).color(MUTED));
        ui.add_space(8.);
        if ui
            .add_enabled(
                *order != EffectOrder::default(),
                egui::Button::new(RichText::new("Ordem padrão").size(11.)),
            )
            .on_hover_text("Restaura somente a ordem. Os parâmetros serão mantidos.")
            .clicked()
        {
            *order = EffectOrder::default();
            changed = true;
        }
        ui.add_space(8.);
        let mut movement = None;
        for (index, effect) in order.0.iter().enumerate() {
            ui.push_id(index, |ui| {
                egui::Frame::new()
                    .fill(BG)
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(7, 3))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 4.;
                        ui.spacing_mut().button_padding = Vec2::new(4., 3.);
                        ui.horizontal(|ui| {
                            let label_width = (ui.available_width() - 74.).max(75.);
                            ui.add_sized(
                                [18., 24.],
                                egui::Label::new(
                                    RichText::new(format!("{:02}", index + 1))
                                        .size(10.)
                                        .monospace()
                                        .color(MUTED),
                                ),
                            );
                            ui.add_sized(
                                [label_width, 24.],
                                egui::Label::new(RichText::new(effect_name(*effect)).size(12.))
                                    .truncate(),
                            )
                            .on_hover_text(effect_name(*effect));
                            if order_arrow_button(ui, Icon::ChevronUp, index > 0, "Processar antes")
                                .clicked()
                            {
                                movement = Some((index, index - 1));
                            }
                            if order_arrow_button(
                                ui,
                                Icon::ChevronDown,
                                index + 1 < order.0.len(),
                                "Processar depois",
                            )
                            .clicked()
                            {
                                movement = Some((index, index + 1));
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Intensidade").size(10.).color(MUTED));
                            ui.spacing_mut().slider_width = (ui.available_width() - 55.).max(30.);
                            let mut percent = mix[*effect as usize] * 100.;
                            if ui
                                .add(
                                    egui::Slider::new(&mut percent, 0.0..=100.)
                                        .suffix("%")
                                        .fixed_decimals(0),
                                )
                                .changed()
                            {
                                mix[*effect as usize] = percent / 100.;
                                changed = true;
                            }
                        });
                    });
            });
        }
        if let Some((from, to)) = movement {
            order.0.swap(from, to);
            changed = true;
        }
        ui.add_space(8.);
        ui.label(
            RichText::new("Pitch e formante compartilham um bloco. Os dois filtros também.")
                .size(11.)
                .color(MUTED),
        );
        ui.label(
            RichText::new("Teto de segurança sempre ao final.")
                .size(11.)
                .color(MUTED),
        );
    });
    changed
}
fn order_arrow_button(
    ui: &mut egui::Ui,
    icon: Icon,
    enabled: bool,
    tooltip: &str,
) -> egui::Response {
    let response = ui.add_enabled(enabled, egui::Button::new("").min_size(Vec2::splat(22.)));
    art::paint_icon(
        ui.painter(),
        response.rect.shrink(4.),
        icon,
        if enabled {
            TEXT
        } else {
            MUTED.gamma_multiply(0.4)
        },
    );
    response.on_hover_text(tooltip)
}
fn parameter(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
) -> bool {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(105., 26.),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(RichText::new(label).size(12.).color(MUTED));
            },
        );
        ui.scope(|ui| {
            ui.spacing_mut().slider_width = (ui.available_width() - 85.).max(90.);
            ui.add(egui::Slider::new(value, range).suffix(suffix))
                .changed()
        })
        .inner
    })
    .inner
}
fn icon_button(ui: &mut egui::Ui, icon: Icon, active: bool, tooltip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(44., 40.), egui::Sense::click());
    if active || response.hovered() {
        ui.painter().rect_filled(
            rect,
            7.,
            if active {
                Color32::from_rgb(0, 83, 86)
            } else {
                Color32::from_rgb(38, 46, 48)
            },
        );
    }
    art::paint_icon(
        ui.painter(),
        rect.shrink2(Vec2::new(11., 9.)),
        icon,
        if active { ACCENT } else { MUTED },
    );
    response
        .on_hover_text(tooltip)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}
fn switch(ui: &mut egui::Ui, value: &mut bool, label: &str) -> bool {
    ui.horizontal(|ui| {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(31., 18.), egui::Sense::click());
        let changed = response.clicked();
        if changed {
            *value = !*value;
        }
        ui.painter().rect_filled(
            rect,
            9.,
            if *value {
                ACCENT
            } else {
                Color32::from_rgb(55, 65, 67)
            },
        );
        ui.painter().circle_filled(
            egui::pos2(
                if *value {
                    rect.right() - 9.
                } else {
                    rect.left() + 9.
                },
                rect.center().y,
            ),
            6.,
            if *value {
                BG
            } else {
                Color32::from_rgb(180, 190, 192)
            },
        );
        if ui
            .add(egui::Label::new(RichText::new(label).size(12.)).sense(egui::Sense::click()))
            .clicked()
        {
            *value = !*value;
            return true;
        }
        changed
    })
    .inner
}
fn run() -> Result<()> {
    let mut paths = Paths::discover()?;
    let mut smoke = None;
    let mut qa_size = None;
    let mut screenshot = None;
    let mut page = Page::Voices;
    let mut recorder_source = RecordingSource::ProcessedVoice;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => {
                paths.data = PathBuf::from(args.next().context("--data-dir requires a path")?)
            }
            "--socket" => {
                paths.socket = PathBuf::from(args.next().context("--socket requires a path")?)
            }
            "--smoke-seconds" => {
                smoke = Some(
                    args.next()
                        .context("--smoke-seconds requires a number")?
                        .parse::<f32>()?,
                )
            }
            "--screenshot" => {
                screenshot = Some(PathBuf::from(
                    args.next().context("--screenshot requires a path")?,
                ))
            }
            "--qa-size" => {
                let size = args.next().context("--qa-size requires WIDTHxHEIGHT")?;
                let (width, height) = size
                    .split_once('x')
                    .context("--qa-size requires WIDTHxHEIGHT with integer dimensions")?;
                let width = width
                    .parse::<u32>()
                    .context("--qa-size width must be an integer")?;
                let height = height
                    .parse::<u32>()
                    .context("--qa-size height must be an integer")?;
                if !(1080..=3840).contains(&width) || !(720..=2160).contains(&height) {
                    anyhow::bail!("--qa-size must be within 1080x720 and 3840x2160");
                }
                qa_size = Some([width as f32, height as f32]);
            }
            "--recorder-source" => {
                recorder_source = match args.next().as_deref() {
                    Some("processed-voice") => RecordingSource::ProcessedVoice,
                    Some("desktop") => RecordingSource::Desktop,
                    _ => anyhow::bail!("--recorder-source requires processed-voice or desktop"),
                };
            }
            "--page" => {
                page = match args.next().as_deref() {
                    Some("voices") => Page::Voices,
                    Some("soundboard") => Page::Soundboard,
                    Some("lab") => Page::Lab,
                    Some("recorder") => Page::Recorder,
                    Some("settings") => Page::Settings,
                    _ => anyhow::bail!("unknown page"),
                }
            }
            "--help" | "-h" => {
                println!(
                    "voice-app [--data-dir PATH] [--socket PATH] [--recorder-source processed-voice|desktop]\nQA: --smoke-seconds N --screenshot PATH --page voices|soundboard|lab|recorder|settings [--qa-size WIDTHxHEIGHT (requires --smoke-seconds)]"
                );
                return Ok(());
            }
            _ => anyhow::bail!("unknown argument: {arg}"),
        }
    }
    if qa_size.is_some() && smoke.is_none() {
        anyhow::bail!("--qa-size requires --smoke-seconds");
    }
    let service = if client::send(&paths.socket, Command::Status).is_ok() {
        None
    } else {
        Some(voice_engine::spawn_service(paths.clone())?)
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("voice app")
            .with_inner_size(qa_size.unwrap_or([1280., 820.]))
            .with_min_inner_size([1080., 720.]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "voice-app",
        options,
        Box::new(move |cc| {
            Ok(Box::new(App::new(
                cc,
                paths,
                service,
                smoke,
                screenshot,
                page,
                recorder_source,
            )))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
fn main() {
    if let Err(e) = run() {
        eprintln!("voice-app: {e:#}");
        #[cfg(windows)]
        rfd::MessageDialog::new()
            .set_title("Voice App")
            .set_description(format!("Não foi possível abrir o aplicativo: {e:#}"))
            .set_level(rfd::MessageLevel::Error)
            .show();
        std::process::exit(1);
    }
}
