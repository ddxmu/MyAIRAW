//! The optional AI photo assistant.
//!
//! The assistant speaks the OpenAI-compatible Chat Completions protocol. The
//! model never receives a file path or the catalog: when the user grants the
//! preview permission it receives only a small JPEG preview plus the current
//! develop values. Its response is constrained to a small, allow-listed set of
//! develop sliders and is applied as one undoable history step.
use super::{Editor, history::Step, preferences::Tab};
use crate::app::commands::Param;
use crate::camera_data::CameraImage;
use crate::color::srgb_encode;
use base64::Engine;
use eframe::egui::{self, Color32, Stroke, Vec2};
use image::ImageEncoder;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

const CONFIG_FILE: &str = "ai.json";
const DEFAULT_API_URL: &str = "https://api.deepseek.com/v1";
const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_PREVIEW_EDGE: u32 = 768;

/// Settings saved in the user's MyAIRAW data directory, never in the catalog
/// or the repository. The API key is intentionally not sent anywhere except
/// the user-entered API host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Config {
    pub(super) api_url: String,
    pub(super) api_key: String,
    pub(super) model: String,
    pub(super) models: Vec<String>,
    /// Send a down-sized JPEG preview with the prompt.
    pub(super) send_preview: bool,
    /// Apply a valid response immediately; when false, the assistant shows an
    /// Apply button instead.
    pub(super) allow_apply: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            api_url: DEFAULT_API_URL.into(),
            api_key: String::new(),
            model: String::new(),
            models: Vec::new(),
            send_preview: true,
            allow_apply: true,
        }
    }
}
impl Config {
    fn path() -> PathBuf {
        crate::storage::data_dir().join(CONFIG_FILE)
    }
    fn load() -> Self {
        crate::storage::read_json_or_default(&Self::path())
    }
    fn save(&self) -> anyhow::Result<()> {
        crate::storage::atomic_json(&Self::path(), self)?;
        // Keep the locally stored API key private on Unix. atomic_json is
        // shared by other app settings and intentionally does not pick a mode.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(Self::path(), std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Adjustment {
    param: Param,
    value: f32,
    name: String,
}
#[derive(Clone, Debug, PartialEq)]
struct Plan {
    summary: String,
    adjustments: Vec<Adjustment>,
}
#[derive(Clone, Debug)]
struct Line {
    user: bool,
    text: String,
}

enum Reply {
    Models {
        request: u64,
        result: Result<Vec<String>, String>,
    },
    Plan {
        request: u64,
        generation: u64,
        result: Result<Plan, String>,
    },
}

/// Editor-owned state. Network work is isolated on short-lived worker threads;
/// only this state is touched by the UI thread.
pub(super) struct State {
    pub(super) open: bool,
    pub(super) config: Config,
    persistent: bool,
    pub(super) prompt: String,
    status: String,
    history: Vec<Line>,
    pending: Option<(u64, Plan)>,
    request: Option<u64>,
    request_count: u64,
    models_request: Option<u64>,
    tx: Sender<Reply>,
    rx: Receiver<Reply>,
    ctx: egui::Context,
}
impl State {
    pub(super) fn new(ctx: &egui::Context, persistent: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            open: false,
            config: persistent.then(Config::load).unwrap_or_default(),
            persistent,
            prompt: String::new(),
            status: String::new(),
            history: Vec::new(),
            pending: None,
            request: None,
            request_count: 0,
            models_request: None,
            tx,
            rx,
            ctx: ctx.clone(),
        }
    }
    fn next_request(&mut self) -> u64 {
        self.request_count = self.request_count.wrapping_add(1).max(1);
        self.request_count
    }
    fn save(&mut self) {
        if !self.persistent {
            return;
        }
        if let Err(error) = self.config.save() {
            self.status = format!("AI 设置保存失败：{error:#}");
        }
    }
}

impl Editor {
    /// Polls AI replies at the same UI boundary as the renderer and catalog
    /// workers. A stale photo generation can never receive a prior response.
    pub(super) fn poll_ai(&mut self) {
        while let Ok(reply) = self.ai_reply() {
            match reply {
                Reply::Models { request, result } => {
                    if self.ai.models_request != Some(request) {
                        continue;
                    }
                    self.ai.models_request = None;
                    match result {
                        Ok(models) => {
                            if self.ai.config.model.is_empty()
                                || !models.iter().any(|m| m == &self.ai.config.model)
                            {
                                self.ai.config.model = models.first().cloned().unwrap_or_default();
                            }
                            self.ai.config.models = models;
                            self.ai.status =
                                format!("已拉取 {} 个模型", self.ai.config.models.len());
                            self.ai.save();
                        }
                        Err(error) => self.ai.status = format!("拉取模型失败：{error}"),
                    }
                }
                Reply::Plan {
                    request,
                    generation,
                    result,
                } => {
                    if self.ai.request != Some(request) {
                        continue;
                    }
                    self.ai.request = None;
                    match result {
                        Ok(_plan) if generation != self.load.id() => {
                            self.ai.pending = None;
                            self.ai.status = "照片已经切换，未应用旧的 AI 结果".into();
                        }
                        Ok(plan) if self.ai.config.allow_apply => {
                            self.apply_ai_plan(generation, plan);
                        }
                        Ok(plan) => {
                            self.ai.status = format!(
                                "AI 返回 {} 项可用调整，请确认应用",
                                plan.adjustments.len()
                            );
                            self.ai.pending = Some((generation, plan));
                        }
                        Err(error) => self.ai.status = format!("AI 请求失败：{error}"),
                    }
                }
            }
        }
    }

    fn ai_reply(&mut self) -> Result<Reply, mpsc::TryRecvError> {
        self.ai.rx.try_recv()
    }

    fn start_ai_request(&mut self) {
        if self.module != super::Module::Develop || self.document.full().is_none() {
            self.ai.status = "请先在显影中打开一张已加载的照片".into();
            return;
        }
        let prompt = self.ai.prompt.trim().to_string();
        if prompt.is_empty() {
            self.ai.status = "请先输入想要的调整，例如：提亮主体并压低高光".into();
            return;
        }
        if self.ai.config.api_url.trim().is_empty()
            || self.ai.config.api_key.trim().is_empty()
            || self.ai.config.model.trim().is_empty()
        {
            self.ai.status = "请先在 AI 设置中填写地址、API Key 并选择模型".into();
            self.ai.open = false;
            self.open_preferences(Tab::Ai);
            return;
        }
        let preview = if self.ai.config.send_preview {
            match self.document.full().map(encode_preview).transpose() {
                Ok(image) => image,
                Err(error) => {
                    self.ai.status = format!("无法准备照片预览：{error:#}");
                    return;
                }
            }
        } else {
            None
        };
        let mut current = self.document.edit.recipe().clone();
        let values = Param::NAMED
            .iter()
            .map(|(name, param)| ((*name).to_string(), param.shown(&mut current, 0)))
            .collect::<BTreeMap<_, _>>();
        let filename = self
            .document
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "当前照片".into());
        let request = self.ai.next_request();
        let generation = self.load.id();
        let config = self.ai.config.clone();
        let tx = self.ai.tx.clone();
        let ctx = self.ai.ctx.clone();
        self.ai.request = Some(request);
        self.ai.pending = None;
        self.ai.status = "正在请求 AI…".into();
        self.ai.history.push(Line {
            user: true,
            text: prompt.clone(),
        });
        self.ai.prompt.clear();
        if std::thread::Builder::new()
            .name("ai-photo-adjustment".into())
            .spawn(move || {
                let result = request_plan(&config, &prompt, &filename, &values, preview)
                    .and_then(|body| parse_plan(&body));
                let _ = tx.send(Reply::Plan {
                    request,
                    generation,
                    result,
                });
                ctx.request_repaint();
            })
            .is_err()
        {
            self.ai.request = None;
            self.ai.status = "无法启动 AI 请求线程".into();
        }
    }

    fn apply_ai_plan(&mut self, generation: u64, plan: Plan) {
        if generation != self.load.id() {
            self.ai.status = "照片已经切换，未应用旧的 AI 结果".into();
            return;
        }
        if plan.adjustments.is_empty() {
            self.ai.status = "AI 没有返回可应用的显影参数".into();
            return;
        }
        let summary = if plan.summary.is_empty() {
            "按提示词调整".to_string()
        } else {
            plan.summary.clone()
        };
        let changes = plan.adjustments;
        let count = changes.len();
        self.change_edit(Some(Step::new("AI 调整", summary.clone())), |recipe| {
            for change in &changes {
                change.param.set(recipe, change.value, 0);
            }
        });
        self.ai.history.push(Line {
            user: false,
            text: format!("已应用 {count} 项调整：{summary}"),
        });
        self.ai.status = format!("AI 已应用 {count} 项调整，可用撤销恢复");
    }

    pub(super) fn ai_windows(&mut self, ctx: &egui::Context) {
        if !self.ai.open {
            return;
        }
        let palette = super::theme::palette(ctx);
        let mut send = false;
        let mut apply = None;
        let mut open_settings = false;
        let response = egui::Modal::new(egui::Id::new("ai-assistant"))
            .backdrop_color(Color32::from_black_alpha(150))
            .frame(super::widgets::modal_frame(&palette).inner_margin(20))
            .show(ctx, |ui| {
                ui.set_min_width(700.);
                ui.set_max_width(860.);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("MyAIRAW AI 助手").size(19.));
                    ui.add_space(10.);
                    let model = if self.ai.config.model.is_empty() {
                        "未选择模型"
                    } else {
                        &self.ai.config.model
                    };
                    ui.label(
                        egui::RichText::new(format!("模型：{model}"))
                            .color(palette.gray(175))
                            .size(13.),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("AI 设置…").clicked() {
                            open_settings = true;
                        }
                        if ui.button("清空对话").clicked() {
                            self.ai.history.clear();
                            self.ai.pending = None;
                        }
                    });
                });
                ui.add_space(10.);
                egui::Frame::new()
                    .fill(palette.gray(28))
                    .stroke(Stroke::new(1., palette.gray(57)))
                    .inner_margin(12.)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(300.)
                            .auto_shrink(false)
                            .show(ui, |ui| {
                                if self.ai.history.is_empty() {
                                    ui.label(
                                        egui::RichText::new(
                                            "配置模型后，描述你想要的照片效果。AI 只返回可撤销的显影参数。",
                                        )
                                        .color(palette.gray(150)),
                                    );
                                }
                                for line in &self.ai.history {
                                    let who = if line.user { "你" } else { "AI" };
                                    ui.label(egui::RichText::new(who).strong().color(if line.user {
                                        palette.accent()
                                    } else {
                                        palette.gray(200)
                                    }));
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&line.text)
                                                .color(palette.gray(225)),
                                        )
                                        .wrap(),
                                    );
                                    ui.add_space(8.);
                                }
                                if let Some((_, plan)) = &self.ai.pending {
                                    ui.separator();
                                    ui.label(egui::RichText::new("待应用调整").strong());
                                    ui.label(format_adjustments(plan));
                                    if ui.button("应用这组调整").clicked() {
                                        apply = self.ai.pending.take();
                                    }
                                }
                            });
                    });
                ui.add_space(10.);
                if ui
                    .checkbox(
                        &mut self.ai.config.send_preview,
                        "将当前照片预览发送给模型",
                    )
                    .changed()
                {
                    self.ai.save();
                }
                if ui
                    .checkbox(
                        &mut self.ai.config.allow_apply,
                        "允许 AI 自动应用返回的显影参数",
                    )
                    .changed()
                {
                    self.ai.save();
                }
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "发送时会把一张缩小的 JPEG 预览和当前滑块数值发到你填写的 API 地址；原图不会被上传或覆盖。",
                        )
                        .size(12.)
                        .color(palette.gray(145)),
                    )
                    .wrap(),
                );
                ui.add_space(6.);
                ui.horizontal(|ui| {
                    let field = ui.add(
                        egui::TextEdit::multiline(&mut self.ai.prompt)
                            .desired_rows(3)
                            .desired_width(ui.available_width() - 110.)
                            .hint_text("描述你想要的编辑，例如：提亮主体、压低天空高光、整体更通透"),
                    );
                    if field.changed() {
                        self.ai.status.clear();
                    }
                    if ui
                        .add_enabled(
                            self.ai.request.is_none(),
                            egui::Button::new(if self.ai.request.is_some() {
                                "请求中…"
                            } else {
                                "发送"
                            })
                            .min_size(Vec2::new(92., 70.)),
                        )
                        .clicked()
                    {
                        send = true;
                    }
                });
                if !self.ai.status.is_empty() {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&self.ai.status)
                                .size(12.)
                                .color(palette.gray(165)),
                        )
                        .wrap(),
                    );
                }
            });
        if open_settings {
            self.ai.open = false;
            self.open_preferences(Tab::Ai);
        }
        if let Some((generation, plan)) = apply {
            self.apply_ai_plan(generation, plan);
        }
        if send {
            self.start_ai_request();
        }
        if response.should_close() {
            self.ai.open = false;
        }
    }

    /// Preferences › AI. It intentionally lives in the app layer because the
    /// transport and API key are user-interface settings, not photo-domain data.
    pub(super) fn ai_settings_page(&mut self, ui: &mut egui::Ui) {
        use super::preferences::{gap, group, hint};
        use super::widgets::form_row;
        group(ui, "模型服务");
        hint(
            ui,
            "支持 OpenAI 兼容的 /models 与 /chat/completions 接口。API Key 只保存在本机 MyAIRAW 数据目录。",
        );
        form_row(ui, "API 地址", |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.ai.config.api_url)
                    .desired_width(430.)
                    .hint_text("https://api.deepseek.com/v1"),
            );
        });
        form_row(ui, "API Key", |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.ai.config.api_key)
                    .password(true)
                    .desired_width(430.)
                    .hint_text("sk-…"),
            );
        });
        form_row(ui, "模型", |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.ai.config.model)
                    .desired_width(250.)
                    .hint_text("先拉取模型，或手动填写"),
            );
            if !self.ai.config.models.is_empty() {
                egui::ComboBox::from_id_salt("ai-model-list")
                    .width(190.)
                    .selected_text("选择已拉取模型")
                    .show_ui(ui, |ui| {
                        for model in self.ai.config.models.clone() {
                            if ui
                                .selectable_label(self.ai.config.model == model, &model)
                                .clicked()
                            {
                                self.ai.config.model = model;
                                ui.close();
                            }
                        }
                    });
            }
            if ui
                .add_enabled(
                    self.ai.models_request.is_none(),
                    egui::Button::new(if self.ai.models_request.is_some() {
                        "拉取中…"
                    } else {
                        "拉取模型"
                    }),
                )
                .clicked()
            {
                self.fetch_ai_models();
            }
        });
        form_row(ui, "权限", |ui| {
            ui.checkbox(&mut self.ai.config.send_preview, "允许发送当前照片预览");
            ui.checkbox(&mut self.ai.config.allow_apply, "允许自动应用显影参数");
        });
        hint(
            ui,
            "AI 返回的是曝光、对比度、高光、阴影、白平衡等显影滑块值；每次应用都会进入历史记录，可以撤销。",
        );
        if !self.ai.status.is_empty() {
            hint(ui, &self.ai.status.clone());
        }
        gap(ui);
        form_row(ui, "", |ui| {
            if ui.button("测试连接并拉取模型").clicked() {
                self.fetch_ai_models();
            }
            if ui.button("保存 AI 设置").clicked() {
                self.ai.save();
                self.status = "AI 设置已保存到本机".into();
            }
        });
    }

    fn fetch_ai_models(&mut self) {
        if self.ai.config.api_url.trim().is_empty() || self.ai.config.api_key.trim().is_empty() {
            self.ai.status = "请先填写 API 地址和 API Key".into();
            return;
        }
        let request = self.ai.next_request();
        self.ai.models_request = Some(request);
        self.ai.status = "正在拉取模型…".into();
        let config = self.ai.config.clone();
        let tx = self.ai.tx.clone();
        let ctx = self.ai.ctx.clone();
        if std::thread::Builder::new()
            .name("ai-models".into())
            .spawn(move || {
                let result = fetch_models(&config);
                let _ = tx.send(Reply::Models { request, result });
                ctx.request_repaint();
            })
            .is_err()
        {
            self.ai.models_request = None;
            self.ai.status = "无法启动 AI 模型请求线程".into();
        }
    }
}

/// Build a small, ordinary RGB JPEG from the decoded image. This intentionally
/// does not expose the original path, EXIF or catalog metadata.
fn encode_preview(image: &Arc<CameraImage>) -> anyhow::Result<String> {
    let scale = (MAX_PREVIEW_EDGE as f32 / image.width.max(image.height) as f32).min(1.);
    let width = ((image.width as f32 * scale).round() as u32).max(1);
    let height = ((image.height as f32 * scale).round() as u32).max(1);
    let mut rgb = image::RgbImage::new(width, height);
    for y in 0..height {
        let sy = ((y as u64 * image.height as u64) / height as u64) as u32;
        for x in 0..width {
            let sx = ((x as u64 * image.width as u64) / width as u64) as u32;
            let p = image.pixels[(sy * image.width + sx) as usize];
            rgb.put_pixel(
                x,
                y,
                image::Rgb(p.map(|v| (srgb_encode(v.clamp(0., 1.)) * 255.).round() as u8)),
            );
        }
    }
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 82).write_image(
        rgb.as_raw(),
        width,
        height,
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(jpeg)
    ))
}

fn api_url(base: &str, resource: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    if base == resource || base.ends_with(&format!("/{resource}")) {
        base.to_string()
    } else {
        format!("{base}/{resource}")
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(Duration::from_secs(180)))
        .build()
        .into()
}

fn fetch_models(config: &Config) -> Result<Vec<String>, String> {
    let response = agent()
        .get(&api_url(&config.api_url, "models"))
        .header("Authorization", format!("Bearer {}", config.api_key.trim()))
        .header("Accept", "application/json")
        .header("User-Agent", "MyAIRAW-AI")
        .call()
        .map_err(|e| format!("无法连接 API（{e}）"))?;
    let status = response.status().as_u16();
    let body = response
        .into_body()
        .into_with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_string()
        .map_err(|e| format!("读取 API 响应失败（{e}）"))?;
    if !(200..300).contains(&status) {
        return Err(api_error(status, &body));
    }
    parse_models(&body)
}

fn request_plan(
    config: &Config,
    prompt: &str,
    filename: &str,
    values: &BTreeMap<String, f64>,
    preview: Option<String>,
) -> Result<String, String> {
    let values = serde_json::to_string(values).map_err(|e| e.to_string())?;
    let instruction = format!(
        "照片文件名（仅作上下文）：{filename}\n当前显影滑块（绝对值）：{values}\n用户要求：{prompt}\n\n请只返回 JSON，不要 Markdown 代码围栏。格式：{{\"summary\":\"简短中文说明\",\"adjustments\":{{\"exposure\":0.0,\"contrast\":0}}}}。adjustments 只能使用 exposure、contrast、highlights、shadows、whites、blacks、texture、clarity、dehaze、vibrance、saturation、temperature、tint、straighten；值必须是这些滑块的绝对显示值（曝光为 EV，温度为 kelvin，其余通常为 -100 到 100）。只输出确实需要改变的参数，不能确定时返回空对象。"
    );
    let content = if let Some(preview) = preview {
        json!([
            {"type":"text", "text": instruction},
            {"type":"image_url", "image_url":{"url": preview}}
        ])
    } else {
        json!(instruction)
    };
    let body = json!({
        "model": config.model.trim(),
        "temperature": 0.1,
        "max_tokens": 800,
        "messages": [
            {"role":"system", "content":"你是 MyAIRAW 的照片显影助手。你只能提出安全、可撤销的显影滑块调整，严禁返回文件操作、代码或 base64。"},
            {"role":"user", "content": content}
        ]
    });
    let response = agent()
        .post(&api_url(&config.api_url, "chat/completions"))
        .header("Authorization", format!("Bearer {}", config.api_key.trim()))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("User-Agent", "MyAIRAW-AI")
        .send(body.to_string())
        .map_err(|e| format!("无法连接 API（{e}）"))?;
    let status = response.status().as_u16();
    let body = response
        .into_body()
        .into_with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_string()
        .map_err(|e| format!("读取 API 响应失败（{e}）"))?;
    if !(200..300).contains(&status) {
        return Err(api_error(status, &body));
    }
    let value: Value =
        serde_json::from_str(&body).map_err(|e| format!("API 返回不是 JSON（{e}）"))?;
    value["choices"][0]["message"]["content"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "API 响应缺少 choices[0].message.content".into())
}

fn api_error(status: u16, body: &str) -> String {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| body.trim().chars().take(240).collect());
    if message.is_empty() {
        format!("HTTP {status}")
    } else {
        format!("HTTP {status}：{message}")
    }
}

fn parse_models(body: &str) -> Result<Vec<String>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|e| format!("模型列表不是 JSON（{e}）"))?;
    let list = value
        .get("data")
        .or_else(|| value.get("models"))
        .and_then(Value::as_array)
        .ok_or_else(|| "模型列表缺少 data 或 models 数组".to_string())?;
    let mut models = list
        .iter()
        .filter_map(|item| item["id"].as_str().or_else(|| item["name"].as_str()))
        .filter(|id| !id.trim().is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    models.sort_unstable();
    models.dedup();
    if models.is_empty() {
        Err("API 没有返回可用模型".into())
    } else {
        Ok(models)
    }
}

fn parse_plan(content: &str) -> Result<Plan, String> {
    let json = content.trim();
    let json = json.strip_prefix("\x60\x60\x60json").unwrap_or(json);
    let json = json.strip_prefix("\x60\x60\x60JSON").unwrap_or(json);
    let json = json.strip_prefix("\x60\x60\x60").unwrap_or(json);
    let json = json.strip_suffix("\x60\x60\x60").unwrap_or(json).trim();
    let start = json
        .find('{')
        .ok_or_else(|| "AI 没有返回 JSON 调整方案".to_string())?;
    let end = json
        .rfind('}')
        .ok_or_else(|| "AI 返回的调整方案不完整".to_string())?;
    let value: Value = serde_json::from_str(&json[start..=end])
        .map_err(|e| format!("AI 调整 JSON 无法解析（{e}）"))?;
    let summary = value["summary"]
        .as_str()
        .unwrap_or("按提示词调整")
        .to_string();
    let mut adjustments = Vec::new();
    if let Some(object) = value["adjustments"].as_object() {
        for (name, value) in object {
            push_adjustment(&mut adjustments, name, value)?;
        }
    } else if let Some(items) = value["adjustments"].as_array() {
        for item in items {
            let name = item["parameter"]
                .as_str()
                .or_else(|| item["name"].as_str())
                .ok_or_else(|| "AI 调整项缺少 parameter".to_string())?;
            push_adjustment(&mut adjustments, name, &item["value"])?;
        }
    } else if !value["adjustments"].is_null() {
        return Err("AI adjustments 必须是对象或数组".into());
    }
    Ok(Plan {
        summary,
        adjustments,
    })
}

fn push_adjustment(out: &mut Vec<Adjustment>, name: &str, value: &Value) -> Result<(), String> {
    let name = name.trim().to_ascii_lowercase();
    let allowed = [
        "exposure",
        "contrast",
        "highlights",
        "shadows",
        "whites",
        "blacks",
        "texture",
        "clarity",
        "dehaze",
        "vibrance",
        "saturation",
        "temperature",
        "tint",
        "straighten",
    ];
    if !allowed.contains(&name.as_str()) {
        return Ok(());
    }
    let value = value
        .as_f64()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("AI 调整 {name} 不是数字"))? as f32;
    let param = Param::parse(&name).ok_or_else(|| format!("不支持的 AI 参数 {name}"))?;
    let (min, max) = param.range(false);
    if !(min..=max).contains(&value) {
        return Err(format!("AI 调整 {name} 超出范围 {min}–{max}"));
    }
    out.push(Adjustment { param, value, name });
    Ok(())
}

fn format_adjustments(plan: &Plan) -> String {
    plan.adjustments
        .iter()
        .map(|item| format!("{}={:.2}", item.name, item.value))
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_list_accepts_openai_and_models_shapes() {
        assert_eq!(
            parse_models(r#"{"data":[{"id":"z"},{"id":"a"},{"id":"z"}]}"#).unwrap(),
            vec!["a", "z"]
        );
        assert_eq!(
            parse_models(r#"{"models":[{"name":"vision"}]}"#).unwrap(),
            vec!["vision"]
        );
    }

    #[test]
    fn plan_only_allows_safe_slider_values() {
        let plan = parse_plan(
            r#"{"summary":"提亮","adjustments":{"exposure":0.7,"contrast":12,"delete_file":1}}"#,
        )
        .unwrap();
        assert_eq!(plan.adjustments.len(), 2);
        assert!(plan.adjustments.iter().any(|item| item.name == "exposure"));
        assert!(plan.adjustments.iter().any(|item| item.name == "contrast"));
        assert!(parse_plan(r#"{"adjustments":{"exposure":99}}"#).is_err());
    }

    #[test]
    fn api_paths_do_not_duplicate_resources() {
        assert_eq!(
            api_url("https://example/v1/", "models"),
            "https://example/v1/models"
        );
        assert_eq!(
            api_url("https://example/v1/models", "models"),
            "https://example/v1/models"
        );
    }
}
