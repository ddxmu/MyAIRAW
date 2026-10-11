//! Device instances have their own settings. Script/AI setup is independent.
use super::*;
use crate::app::{
    inspector::BANDS,
    preferences::{gap, group},
    theme,
    widgets::form_row,
};
use mapping::supported_action as parse_action;
/// What a button can be set to from the list: its name and the key it presses,
/// spelled as in `midi.json`. Anything else is typed in the field beside it.
pub(super) const PRESETS: [(&str, &str); 47] = [
    ("不执行操作", ""),
    ("撤销", "cmd+z"),
    ("重做", "cmd+shift+z"),
    ("上一张照片", "left"),
    ("下一张照片", "right"),
    ("Arrow up", "arrowup"),
    ("Arrow down", "arrowdown"),
    ("评分 0 星", "0"),
    ("评分 1 星", "1"),
    ("评分 2 星", "2"),
    ("评分 3 星", "3"),
    ("评分 4 星", "4"),
    ("评分 5 星", "5"),
    ("选中", "p"),
    ("拒绝", "x"),
    ("清除旗标", "u"),
    ("红色标签", "6"),
    ("黄色标签", "7"),
    ("绿色标签", "8"),
    ("蓝色标签", "9"),
    ("前后对比", "backslash"),
    ("显示剪裁警告", "j"),
    ("切换缩放", "z"),
    ("裁剪与拉直", "r"),
    ("污点移除", "q"),
    ("白平衡吸管", "w"),
    ("蒙版", "shift+w"),
    ("隐藏／显示侧边面板", "tab"),
    ("隐藏／显示全部面板", "shift+tab"),
    ("转换为黑白或彩色", "toggle:bw"),
    ("颜色混合器：色相", "mixer:hue"),
    ("颜色混合器：饱和度", "mixer:sat"),
    ("颜色混合器：明亮度", "mixer:lum"),
    ("复制设置…", "cmd+shift+c"),
    ("粘贴设置", "cmd+shift+v"),
    ("粘贴上一张设置", "cmd+alt+v"),
    ("同步设置…", "cmd+shift+s"),
    ("重置所有设置", "cmd+shift+r"),
    ("自动色调", "cmd+shift+u"),
    ("导出…", "cmd+shift+e"),
    ("使用上次设置导出", "cmd+alt+shift+e"),
    ("照片库网格", "g"),
    ("显影", "d"),
    ("按住 Shift", "hold:shift"),
    ("按住 Command", "hold:cmd"),
    ("按住 Alt", "hold:alt"),
    ("在放大视图显示", "e"),
];

/// What a dial does.
#[derive(Clone, Copy, PartialEq)]
enum DialUse {
    None,
    Slider(Param),
    /// Previous / next photo.
    Photos,
}
impl DialUse {
    fn label(self) -> String {
        match self {
            DialUse::None => "不执行操作".into(),
            DialUse::Photos => "上一张／下一张照片".into(),
            DialUse::Slider(Param::Band(i)) => format!("颜色混合器：{}", BANDS[i]),
            DialUse::Slider(p) => {
                let name = p.spec();
                let mut chars = name.chars();
                chars
                    .next()
                    .map(|c| c.to_uppercase().chain(chars).collect())
                    .unwrap_or_default()
            }
        }
    }
    /// Every choice, in the order of the list.
    fn all() -> Vec<DialUse> {
        let sliders = Param::NAMED.iter().map(|(_, p)| DialUse::Slider(*p));
        let bands = (0..8).map(|i| DialUse::Slider(Param::Band(i)));
        [DialUse::None]
            .into_iter()
            .chain(sliders)
            .chain(bands)
            .chain([DialUse::Photos])
            .collect()
    }
}

fn dial_use(config: &Config, cc: u8) -> DialUse {
    if config.photo_dial == Some(cc) {
        DialUse::Photos
    } else {
        config
            .dials
            .get(&cc)
            .map_or(DialUse::None, |p| DialUse::Slider(*p))
    }
}
fn set_dial(config: &mut Config, cc: u8, choice: DialUse) {
    config.dials.remove(&cc);
    if config.photo_dial == Some(cc) {
        config.photo_dial = None;
    }
    match choice {
        DialUse::None => {}
        DialUse::Slider(p) => {
            config.dials.insert(cc, p);
        }
        // Only one dial moves between photos.
        DialUse::Photos => config.photo_dial = Some(cc),
    }
}

fn small(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .size(12.)
            .color(theme::palette(ui.ctx()).gray(135)),
    );
}
pub(super) fn hint(ui: &mut egui::Ui, text: &str) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .size(12.)
                .color(theme::palette(ui.ctx()).gray(135)),
        )
        .wrap(),
    );
}

fn mappings(
    ui: &mut egui::Ui,
    config: &mut Config,
    profile: Profile,
    device_id: u64,
    last: Option<Last>,
) {
    let ctx = ui.ctx().clone();
    form_row(ui, "添加控制器", |ui| {
        let id = egui::Id::new(("control-number", device_id));
        let mut number = ui.data(|d| d.get_temp::<u8>(id)).unwrap_or(0);
        ui.add(egui::DragValue::new(&mut number).range(0..=127));
        ui.data_mut(|d| d.insert_temp(id, number));
        if ui.button("添加旋钮／滑块").clicked() {
            ui.data_mut(|d| d.insert_temp(egui::Id::new(("add-cc", device_id)), number));
        }
        if ui.button("添加按钮").clicked() {
            ui.data_mut(|d| d.insert_temp(egui::Id::new(("add-note", device_id)), number));
        }
    });
    hint(
        ui,
        "移动控制器以识别编号，或按 CC／音符编号添加，再在下方分配操作。",
    );
    group(ui, "旋钮与推子");
    let choices = DialUse::all();
    let mut dials: Vec<(String, u8)> = profile
        .dials()
        .iter()
        .map(|(n, id)| ((*n).into(), *id))
        .collect();
    let mut extra: Vec<u8> = config
        .dials
        .keys()
        .copied()
        .chain(config.encoders.keys().copied())
        .chain(match last {
            Some(Last::Dial(n, _)) => Some(n),
            _ => None,
        })
        .filter(|id| !profile.dials().iter().any(|(_, n)| n == id))
        .collect();
    extra.sort_unstable();
    extra.dedup();
    dials.extend(extra.into_iter().map(|id| (format!("CC {id}"), id)));
    let draft = ui.data(|d| d.get_temp::<u8>(egui::Id::new(("add-cc", device_id))));
    if let Some(cc) = draft
        && !dials.iter().any(|(_, n)| *n == cc)
    {
        dials.push((format!("CC {cc}"), cc));
    }
    for (name, cc) in dials {
        let now = dial_use(config, cc);
        let mut choice = now;
        form_row(ui, &name, |ui| {
            egui::ComboBox::from_id_salt(("automation-dial", device_id, cc))
                .width(210.)
                .selected_text(now.label())
                .show_ui(ui, |ui| {
                    for option in &choices {
                        ui.selectable_value(&mut choice, *option, option.label());
                    }
                });
            small(ui, &format!("CC {cc}"));
        });
        form_row(ui, "控制格式", |ui| {
            let mut encoder = config.encoder(cc);
            egui::ComboBox::from_id_salt(("encoder", device_id, cc))
                .selected_text(encoder.label())
                .show_ui(ui, |ui| {
                    for mode in Encoder::ALL {
                        ui.selectable_value(&mut encoder, mode, mode.label());
                    }
                });
            if encoder != config.encoder(cc) {
                config.encoders.insert(cc, encoder);
            }
        });
        if choice == DialUse::Photos && config.encoder(cc) == Encoder::Absolute {
            hint(ui, "Photo navigation needs a relative control format.");
        }
        if choice != now {
            set_dial(config, cc, choice);
        }
    }
    gap(ui);

    group(ui, "按钮");
    let mut buttons: Vec<(String, u8)> = profile
        .buttons()
        .iter()
        .map(|(n, id)| ((*n).into(), *id))
        .collect();
    let mut extra: Vec<u8> = config
        .buttons
        .keys()
        .copied()
        .chain(match last {
            Some(Last::Button(n)) => Some(n),
            _ => None,
        })
        .filter(|id| !profile.buttons().iter().any(|(_, n)| n == id))
        .collect();
    extra.sort_unstable();
    extra.dedup();
    buttons.extend(extra.into_iter().map(|id| (format!("Note {id}"), id)));
    let draft = ui.data(|d| d.get_temp::<u8>(egui::Id::new(("add-note", device_id))));
    if let Some(note) = draft
        && !buttons.iter().any(|(_, n)| *n == note)
    {
        buttons.push((format!("Note {note}"), note));
    }
    for (name, note) in buttons {
        let current = config.buttons.get(&note).copied();
        let preset = PRESETS
            .iter()
            .find(|(_, spec)| match current {
                None => spec.is_empty(),
                Some(action) => !spec.is_empty() && parse_action(spec) == Some(action),
            })
            .map(|(label, _)| *label);
        let mut picked = None;
        let mut typed = None;
        form_row(ui, &name, |ui| {
            egui::ComboBox::from_id_salt(("automation-button", device_id, note))
                .width(170.)
                .selected_text(preset.unwrap_or("自定义"))
                .show_ui(ui, |ui| {
                    for (label, spec) in PRESETS {
                        if ui.selectable_label(preset == Some(label), label).clicked() {
                            picked = Some(spec);
                        }
                    }
                });
            // What is typed stays as typed while the field has focus, even
            // while it is not yet a key.
            let id = egui::Id::new(("automation-keys", device_id, note));
            let mut text = ui
                .data(|d| d.get_temp::<String>(id))
                .unwrap_or_else(|| current.map(super::action_spec).unwrap_or_default());
            let valid = text.trim().is_empty() || parse_action(text.trim()).is_some();
            let response = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .desired_width(100.)
                    .hint_text("action")
                    .text_color_opt((!valid).then(|| egui::Color32::from_rgb(230, 100, 90))),
            );
            if response.changed() {
                ui.data_mut(|d| d.insert_temp(id, text.clone()));
                typed = Some(text);
            }
            if response.lost_focus() {
                ui.data_mut(|d| d.remove_temp::<String>(id));
            }
            small(ui, &format!("Note {note}"));
        });
        let spec = picked.or(typed.as_deref()).map(str::trim);
        if picked.is_some() {
            ctx.data_mut(|d| {
                d.remove_temp::<String>(egui::Id::new(("automation-keys", device_id, note)))
            });
        }
        match spec {
            Some("") => {
                config.buttons.remove(&note);
            }
            Some(spec) => {
                if let Some(action) = parse_action(spec) {
                    config.buttons.insert(note, action);
                }
            }
            None => {}
        }
    }
    gap(ui);
    hint(
        ui,
        "按钮执行应用操作，例如 undo、pick、treatment:bw，或 cmd+shift+z 等快捷键。蒙版工具开启时，旋钮作用于所选蒙版。",
    );
}
impl Editor {
    pub(in crate::app) fn automation_page(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some(error) = &self.controls.load_error {
            hint(ui, error);
            hint(
                ui,
                "Fix the configuration file and restart to edit automation settings. Existing files have been left intact.",
            );
            return;
        }
        let mut settings = self.controls.settings.clone();
        group(ui, "脚本与 AI");
        form_row(ui, "本地访问", |ui| {
            ui.checkbox(&mut settings.socket, "允许本地脚本和应用");
        });
        form_row(ui, "状态", |ui| {
            small(
                ui,
                if self.controls.socket.is_some() {
                    "已就绪，可连接脚本及 MCP 客户端"
                } else if settings.socket {
                    "Could not start local access"
                } else {
                    "关闭"
                },
            );
        });
        form_row(ui, "", |ui| {
            hint(
                ui,
                "连接 Codex 或其他 MCP 客户端以控制编辑器。无需 MIDI 设备，仅允许本机用户账户下的程序访问。",
            );
        });
        form_row(ui, "设置", |ui| {
            let error_id = egui::Id::new("mcp-guide-error");
            if ui
                .link("MCP 配置 · macOS、Linux 和 Windows")
                .on_hover_text("在默认浏览器中打开配置指南")
                .clicked()
            {
                // This build omits eframe's links feature. Use the same platform
                // launcher as app updates; build.rs selects matching documentation.
                match crate::platform::web::open(env!("RAWMAKASE_MCP_GUIDE")) {
                    Ok(()) => {
                        ui.data_mut(|d| d.remove_temp::<String>(error_id));
                    }
                    Err(error) => ui.data_mut(|d| {
                        d.insert_temp(error_id, format!("Could not open the setup guide: {error}"));
                    }),
                }
            }
            if let Some(error) = ui.data(|d| d.get_temp::<String>(error_id)) {
                hint(ui, &error);
            }
        });
        gap(ui);
        group(ui, "MIDI 设备");
        ui.horizontal(|ui| {
            ui.menu_button("添加设备", |ui| {
                for profile in Profile::ALL {
                    if ui.button(profile.label()).clicked() {
                        self.controls.selected = Some(settings.add(profile));
                        ui.close();
                    }
                }
            });
            if ui.button("刷新输入").clicked() || !self.controls.ports_scanned {
                self.controls.ports = midi::ports();
                self.controls.ports_scanned = true;
            }
        });
        if settings.devices.is_empty() {
            hint(
                ui,
                "尚未配置 MIDI 设备。可添加 Loupedeck+ 或自定义控制器，各设备独立保存映射。",
            );
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        hint(
            ui,
            "MIDI connections are currently supported on macOS and Windows. Scripts and MCP work independently on this platform.",
        );
        for device in &settings.devices {
            let status = self
                .controls
                .devices
                .iter()
                .find(|d| d.binding.id == device.id)
                .map(|d| locked(&d.status).connected.is_some())
                .unwrap_or(false);
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(self.controls.selected == Some(device.id), &device.name)
                    .clicked()
                {
                    self.controls.selected = Some(device.id);
                }
                small(
                    ui,
                    if !device.enabled {
                        "已禁用"
                    } else if status {
                        "已连接"
                    } else if device.port.is_empty() {
                        "选择输入"
                    } else {
                        "未连接"
                    },
                );
            });
        }
        if self.controls.selected.is_none() {
            self.controls.selected = settings.devices.first().map(|d| d.id);
        }
        let mut remove = None;
        if let Some(device) = settings
            .devices
            .iter_mut()
            .find(|d| Some(d.id) == self.controls.selected)
        {
            gap(ui);
            group(ui, "设备设置");
            form_row(ui, "名称", |ui| {
                ui.text_edit_singleline(&mut device.name);
            });
            form_row(ui, "已启用", |ui| {
                ui.checkbox(&mut device.enabled, "监听此设备");
            });
            form_row(ui, "配置文件", |ui| {
                small(ui, device.profile.label());
                if ui.button("将映射重置为配置文件默认值").clicked() {
                    device.mapping = device.profile.mapping();
                }
            });
            form_row(ui, "MIDI 输入", |ui| {
                egui::ComboBox::from_id_salt(("port", device.id))
                    .selected_text(if device.port.is_empty() {
                        "选择输入"
                    } else {
                        &device.port
                    })
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(device.port.is_empty(), "未分配")
                            .clicked()
                        {
                            device.port.clear();
                            device.port_id = None;
                            device.exact = true;
                        }
                        for port in &self.controls.ports {
                            if ui
                                .selectable_label(
                                    device.port_id.as_ref() == Some(&port.id),
                                    format!("{} · {}", port.name, port.id),
                                )
                                .clicked()
                            {
                                device.port = port.name.clone();
                                device.port_id = Some(port.id.clone());
                                device.exact = true;
                            }
                        }
                    });
            });
            if self.controls.ports.is_empty() {
                form_row(ui, "", |ui| {
                    hint(ui, "未找到 MIDI 输入。请连接设备后刷新输入列表。")
                });
            }
            ui.collapsing("高级连接", |ui| {
                form_row(ui, "端口名称", |ui| {
                    if ui.text_edit_singleline(&mut device.port).changed() {
                        device.port_id = None;
                    }
                });
                form_row(ui, "", |ui| {
                    if ui.checkbox(&mut device.exact, "匹配完整端口名称").changed() {
                        device.port_id = None;
                    }
                });
                form_row(ui, "MIDI 通道", |ui| {
                    egui::ComboBox::from_id_salt(("channel", device.id))
                        .selected_text(device.channel.map_or("所有通道".into(), |c| c.to_string()))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut device.channel, None, "所有通道");
                            for c in 1..=16 {
                                ui.selectable_value(&mut device.channel, Some(c), c.to_string());
                            }
                        });
                });
                form_row(ui, "旋转速度", |ui| {
                    ui.add(
                        egui::DragValue::new(&mut device.mapping.sensitivity)
                            .range(1..=16)
                            .suffix("×"),
                    );
                });
                form_row(ui, "照片切换旋钮", |ui| {
                    ui.add(
                        egui::DragValue::new(&mut device.mapping.photo_detent)
                            .range(1..=64)
                            .suffix(" 刻度／张照片"),
                    );
                });
            });
            let runtime = self
                .controls
                .devices
                .iter()
                .find(|d| d.binding.id == device.id);
            let last = runtime.and_then(|d| locked(&d.status).last);
            let problem = runtime.and_then(|d| locked(&d.status).problem.clone());
            if let Some(problem) = problem {
                hint(ui, &problem);
            }
            form_row(ui, "最近输入", |ui| {
                small(
                    ui,
                    &match last {
                        Some(Last::Dial(cc, v)) => format!("CC {cc} · value {v}"),
                        Some(Last::Button(n)) => format!("Note {n}"),
                        None => "转动旋钮或按下按钮以识别".into(),
                    },
                );
            });
            ui.collapsing("控制映射", |ui| {
                mappings(ui, &mut device.mapping, device.profile, device.id, last);
            });
            if ui.button("移除设备").clicked() {
                remove = Some(device.id);
            }
        }
        if let Some(id) = remove {
            settings.devices.retain(|d| d.id != id);
            self.controls.selected = None;
        }
        if settings != self.controls.settings && !ctx.text_edit_focused() {
            match settings.save() {
                Ok(()) => self.controls.apply(settings),
                Err(e) => self.status = format!("Automation settings not saved: {e:#}"),
            }
        } else if settings != self.controls.settings {
            // Persist text edits on every change; defer listener replacement until focus leaves.
            match settings.save() {
                Ok(()) => self.controls.settings = settings,
                Err(e) => self.status = format!("Automation settings not saved: {e:#}"),
            }
        }
        if !ctx.text_edit_focused()
            && self.controls.devices.iter().map(|d| &d.binding).ne(self
                .controls
                .settings
                .devices
                .iter())
        {
            self.controls.apply(self.controls.settings.clone());
        }
    }
}
