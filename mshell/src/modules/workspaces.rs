use crate::{
    config::{AppearanceColor, InvertScrollDirection, WorkspaceVisibilityMode, WorkspacesModuleConfig},
    outputs::Outputs,
    services::{
        ReadOnlyService, Service, ServiceEvent,
        compositor::{CompositorCommand, CompositorService, CompositorState},
    },
    theme::{MshellTheme, use_theme},
};
use iced::{
    Element, Font, Length, Subscription, SurfaceId, alignment,
    widget::{MouseArea, Row, Space, Stack, button, container, text},
};
use iced_anim::{AnimationBuilder, transition::Easing};
use itertools::Itertools;
use std::collections::HashMap;

#[derive(PartialEq, Eq, Debug, Clone)]
pub enum Displayed {
    Active,
    Visible,
    Hidden,
}

#[derive(Debug, Clone)]
pub struct UiWorkspace {
    pub id: i32,
    pub index: i32,
    pub name: String,
    pub monitor_id: Option<i128>,
    pub monitor: String,
    pub displayed: Displayed,
    pub windows: u16,
}

#[derive(Debug, Clone)]
struct VirtualDesktop {
    pub active: bool,
    pub windows: u16,
}

#[derive(Debug, Clone)]
pub enum Message {
    ServiceEvent(ServiceEvent<CompositorService>),
    ChangeWorkspace(i32),
    Scroll(i32),
    ConfigReloaded(WorkspacesModuleConfig),
    ScrollAccumulator(f32),
}

pub struct Workspaces {
    config: WorkspacesModuleConfig,
    service: Option<CompositorService>,
    ui_workspaces: Vec<UiWorkspace>,
    scroll_accumulator: f32,
}

fn calculate_ui_workspaces(
    config: &WorkspacesModuleConfig,
    state: &CompositorState,
) -> Vec<UiWorkspace> {
    let active_id = state.active_workspace_id;
    let monitors = &state.monitors;
    let monitor_order = monitors
        .iter()
        .enumerate()
        .map(|(idx, monitor)| (monitor.name.clone(), idx))
        .collect::<HashMap<_, _>>();

    // margo is tag-based: every output independently has tags 1..=9,
    // so the same workspace id legitimately appears once per monitor.
    // Dedupe on (id, monitor) instead of just id — otherwise the
    // second/external output's workspace cells get silently dropped.
    let workspaces = state
        .workspaces
        .clone()
        .into_iter()
        .unique_by(|w| (w.id, w.monitor.clone()))
        .collect_vec();

    let mut result: Vec<UiWorkspace> = Vec::with_capacity(workspaces.len());
    // margo has no special / scratchpad-as-workspace concept (tags
    // are positive 1..=9); everything in `workspaces` is a normal
    // tag.
    let normal: Vec<_> = workspaces.into_iter().collect();

    if config.enable_virtual_desktops {
        let monitor_count = monitors.len().max(1);
        let mut virtual_desktops: HashMap<i32, VirtualDesktop> = HashMap::new();

        for w in normal.iter() {
            let vdesk_id = ((w.id - 1) / monitor_count as i32) + 1;
            let is_active = Some(w.id) == active_id;

            if let Some(vdesk) = virtual_desktops.get_mut(&vdesk_id) {
                vdesk.windows += w.windows;
                vdesk.active = vdesk.active || is_active;
            } else {
                virtual_desktops.insert(
                    vdesk_id,
                    VirtualDesktop {
                        active: is_active,
                        windows: w.windows,
                    },
                );
            }
        }

        virtual_desktops.into_iter().for_each(|(id, vdesk)| {
            let idx = (id - 1) as usize;
            let display_name = config
                .workspace_names
                .get(idx)
                .cloned()
                .unwrap_or_else(|| id.to_string());

            result.push(UiWorkspace {
                id,
                index: id,
                name: display_name,
                monitor_id: None,
                monitor: "".to_string(),
                displayed: if vdesk.active {
                    Displayed::Active
                } else {
                    Displayed::Hidden
                },
                windows: vdesk.windows,
            });
        });
    } else {
        for w in normal.iter() {
            let display_name = if w.id > 0 {
                let idx = (w.id - 1) as usize;
                config
                    .workspace_names
                    .get(idx)
                    .cloned()
                    .or_else(|| Some(w.name.clone()))
                    .unwrap_or_else(|| w.id.to_string())
            } else {
                w.name.clone()
            };

            // margo: aynı tag id'si (1..=9) her output'ta bağımsız
            // olarak yaşıyor. Active/visible'ı global `active_id` ile
            // karşılaştırırsak iki monitör için aynı tag'in her
            // ikisinin hücresi de yanar. Workspace'in kendi
            // `monitor` adına bakarak o output'un `active_workspace_id`
            // değeri ile eşleyelim.
            let is_active = monitors
                .iter()
                .any(|m| m.name == w.monitor && m.active_workspace_id == w.id);
            let is_visible = is_active;
            // Unused-on-margo: ashell upstream'inde `active_id` global
            // bir tek değer (fokuslu monitorun aktif workspace'i).
            // Ekosistemde başka kullanım kalırsa diye dokunmuyoruz.
            let _ = active_id;

            result.push(UiWorkspace {
                id: w.id,
                index: w.index,
                name: display_name,
                monitor_id: w.monitor_id,
                monitor: w.monitor.clone(),
                displayed: match (is_active, is_visible) {
                    (true, _) => Displayed::Active,
                    (false, true) => Displayed::Visible,
                    (false, false) => Displayed::Hidden,
                },
                windows: w.windows,
            });
        }
    }

    if config.enable_workspace_filling && !result.is_empty() {
        let existing_indices = result.iter().map(|w| w.index).collect_vec();
        let mut max_index = *existing_indices
            .iter()
            .filter(|&&idx| idx > 0)
            .max()
            .unwrap_or(&0);

        if let Some(max_cfg) = config.max_workspaces
            && max_cfg > max_index as u32
        {
            max_index = max_cfg as i32;
        }

        let missing_indices: Vec<i32> = (1..=max_index)
            .filter(|idx| !existing_indices.contains(idx))
            .collect();

        for index in missing_indices {
            let display_name = if index > 0 {
                let name_idx = (index - 1) as usize;
                config
                    .workspace_names
                    .get(name_idx)
                    .cloned()
                    .unwrap_or_else(|| index.to_string())
            } else {
                index.to_string()
            };

            result.push(UiWorkspace {
                id: index,
                index,
                name: display_name,
                monitor_id: None,
                monitor: "".to_string(),
                displayed: Displayed::Hidden,
                windows: 0,
            });
        }
    }

    if config.group_by_monitor {
        result.sort_by(|a, b| {
            let a_order = monitor_order.get(&a.monitor).copied().unwrap_or(usize::MAX);
            let b_order = monitor_order.get(&b.monitor).copied().unwrap_or(usize::MAX);

            a_order
                .cmp(&b_order)
                .then(a.index.cmp(&b.index))
                .then(a.id.cmp(&b.id))
        });
    } else {
        result.sort_by(|a, b| a.index.cmp(&b.index).then(a.id.cmp(&b.id)));
    }

    result
}

impl Workspaces {
    pub fn new(config: WorkspacesModuleConfig) -> Self {
        Self {
            config,
            service: None,
            ui_workspaces: Vec::new(),
            scroll_accumulator: 0.,
        }
    }

    pub fn update(&mut self, message: Message) -> iced::Task<Message> {
        match message {
            Message::ServiceEvent(event) => {
                match event {
                    ServiceEvent::Init(s) => {
                        self.service = Some(s);
                        self.recalculate_ui_workspaces();
                    }
                    ServiceEvent::Update(e) => {
                        if let Some(s) = &mut self.service {
                            s.update(e);
                            self.recalculate_ui_workspaces();
                        }
                    }
                    _ => {}
                }
                iced::Task::none()
            }
            Message::ChangeWorkspace(id) => {
                if let Some(service) = &mut self.service {
                    let already_active = self
                        .ui_workspaces
                        .iter()
                        .any(|w| w.displayed == Displayed::Active && w.id == id);

                    if !already_active {
                        if self.config.enable_virtual_desktops {
                            return service
                                .command(CompositorCommand::CustomDispatch(
                                    "vdesk".to_string(),
                                    id.to_string(),
                                ))
                                .map(Message::ServiceEvent);
                        } else {
                            return service
                                .command(CompositorCommand::FocusWorkspace(id))
                                .map(Message::ServiceEvent);
                        }
                    }
                }
                iced::Task::none()
            }
            Message::Scroll(direction) => {
                self.scroll_accumulator = 0.;

                /* TODO: should we use the native service implementation instead?
                if let Some(service) = &mut self.service {
                    return service
                        .command(CompositorCommand::ScrollWorkspace(direction))
                        .map(Message::ServiceEvent);
                }
                return iced::Task::none();*/
                let Some(pos) = self
                    .ui_workspaces
                    .iter()
                    .position(|w| w.displayed == Displayed::Active)
                else {
                    return iced::Task::none();
                };

                let current_monitor = self.ui_workspaces[pos].monitor.clone();
                let current_monitor_id = self.ui_workspaces[pos].monitor_id;

                let restrict_to_monitor = matches!(
                    self.config.visibility_mode,
                    WorkspaceVisibilityMode::MonitorSpecific
                        | WorkspaceVisibilityMode::MonitorSpecificExclusive
                );

                let in_current_group = |w: &&UiWorkspace| -> bool {
                    if !restrict_to_monitor {
                        return true;
                    }

                    if let Some(w_monitor_id) = w.monitor_id
                        && let Some(active_monitor_id) = current_monitor_id
                    {
                        return w_monitor_id == active_monitor_id;
                    }

                    if !w.monitor.is_empty() && !current_monitor.is_empty() {
                        return w.monitor == current_monitor;
                    }

                    // monitor doesn't seem to contain any useful info, so assume it's part of the group
                    true
                };

                // Navigate by position in the already-sorted ui_workspaces
                // vector, which represents exact visual order regardless of
                // group_by_monitor or visibility_mode configuration.
                let next_workspace = if direction > 0 {
                    self.ui_workspaces[..pos]
                        .iter()
                        .rev()
                        .find(|w| in_current_group(w))
                } else {
                    self.ui_workspaces[pos + 1..]
                        .iter()
                        .find(|w| in_current_group(w))
                };

                if let Some(next) = next_workspace {
                    return self.update(Message::ChangeWorkspace(next.id));
                }
                iced::Task::none()
            }
            Message::ConfigReloaded(cfg) => {
                self.config = cfg;
                self.recalculate_ui_workspaces();
                iced::Task::none()
            }
            Message::ScrollAccumulator(value) => {
                if value == 0. {
                    self.scroll_accumulator = 0.;
                } else {
                    self.scroll_accumulator += value;
                }

                iced::Task::none()
            }
        }
    }

    fn recalculate_ui_workspaces(&mut self) {
        if let Some(service) = &self.service {
            self.ui_workspaces = calculate_ui_workspaces(&self.config, service);
        }
    }

    pub fn view<'a>(&'a self, id: SurfaceId, outputs: &Outputs) -> Element<'a, Message> {
        let monitor_name = outputs.get_monitor_name(id);

        let row = use_theme(|theme| {
            Row::with_children(
                self.ui_workspaces
                    .iter()
                    .filter_map(|w| {
                        let show = match self.config.visibility_mode {
                            WorkspaceVisibilityMode::All => true,
                            WorkspaceVisibilityMode::MonitorSpecific => {
                                monitor_name
                                    .unwrap_or_else(|| &w.monitor)
                                    .contains(&w.monitor)
                                    || !outputs.has_name(&w.monitor)
                            }
                            WorkspaceVisibilityMode::MonitorSpecificExclusive => monitor_name
                                .unwrap_or_else(|| &w.monitor)
                                .contains(&w.monitor),
                        };

                        if show {
                            let empty = w.windows == 0;
                            let color_index = if self.config.enable_virtual_desktops {
                                Some(w.id as i128)
                            } else {
                                w.monitor_id
                            };

                            // [workspaces.colors] override > theme.workspace_colors
                            let override_color = self
                                .config
                                .colors
                                .get(&w.id.to_string())
                                .and_then(|hex| parse_hex_color(hex));
                            let color = color_index.map(|i| {
                                if let Some(c) = override_color {
                                    Some(c)
                                } else if w.id > 0 {
                                    theme.workspace_colors.get(i as usize).copied()
                                } else {
                                    theme
                                        .special_workspace_colors
                                        .as_ref()
                                        .unwrap_or(&theme.workspace_colors)
                                        .get(i as usize)
                                        .copied()
                                }
                            });

                            {
                                let target_width = match (w.id < 0, &w.displayed) {
                                    (true, _) => None,
                                    (_, Displayed::Active) => Some(theme.space.xl),
                                    (_, Displayed::Visible) => Some(theme.space.lg),
                                    (_, Displayed::Hidden) => Some(theme.space.md),
                                };
                                let name = w.name.clone();
                                let padding = if w.id < 0 {
                                    match w.displayed {
                                        Displayed::Active => [0.0, theme.space.md],
                                        Displayed::Visible => [0.0, theme.space.sm],
                                        Displayed::Hidden => [0.0, theme.space.xs],
                                    }
                                } else {
                                    [0.0, 0.0]
                                };
                                // margo tag IDs are always positive
                                // (1..=9); the upstream special-
                                // workspace branch (negative IDs) is
                                // unreachable.
                                let on_press = Message::ChangeWorkspace(w.id);
                                // Font: özel font_name varsa onu kullan; yoksa
                                // bar'ın global fontu (None = default).
                                let custom_font: Option<Font> = self
                                    .config
                                    .font_name
                                    .as_deref()
                                    .map(|name| Font::with_name(Box::leak(name.to_string().into_boxed_str())));
                                // Size override — yoksa bar_font_size (birlik).
                                let font_size = self
                                    .config
                                    .font_size
                                    .unwrap_or(theme.bar_font_size);
                                let height = theme.space.md;
                                let display_name = name.clone();
                                let displayed = w.displayed.clone();
                                let w_windows = w.windows;
                                let show_count = self.config.show_window_count;

                                Some(match target_width {
                                    Some(tw) if theme.animations_enabled => {
                                        let display_name = display_name.clone();
                                        let displayed = displayed.clone();
                                        AnimationBuilder::new(tw, move |pw| {
                                            use_theme(|theme| {
                                                let mut t = text(display_name.clone()).size(font_size);
                                                if let Some(f) = custom_font {
                                                    t = t.font(f);
                                                }
                                                let pill = button(
                                                    container(t)
                                                        .align_x(alignment::Horizontal::Center)
                                                        .align_y(alignment::Vertical::Center),
                                                )
                                                .style(theme.workspace_button_style(empty, color))
                                                .padding(padding)
                                                .on_press(on_press.clone())
                                                .width(Length::Fixed(pw))
                                                .height(height);
                                                build_pill_with_indicator(
                                                    theme,
                                                    pill.into(),
                                                    &displayed,
                                                    w_windows,
                                                    show_count,
                                                    color,
                                                    pw,
                                                    height,
                                                )
                                            })
                                        })
                                        .animates_layout(true)
                                        .animation(Easing::EASE_OUT.very_quick())
                                        .into()
                                    }
                                    Some(tw) => {
                                        let mut t = text(display_name.clone()).size(font_size);
                                        if let Some(f) = custom_font {
                                            t = t.font(f);
                                        }
                                        let pill = button(
                                            container(t)
                                                .align_x(alignment::Horizontal::Center)
                                                .align_y(alignment::Vertical::Center),
                                        )
                                        .style(theme.workspace_button_style(empty, color))
                                        .padding(padding)
                                        .on_press(on_press)
                                        .width(Length::Fixed(tw))
                                        .height(height);
                                        build_pill_with_indicator(
                                            theme,
                                            pill.into(),
                                            &displayed,
                                            w_windows,
                                            show_count,
                                            color,
                                            tw,
                                            height,
                                        )
                                    }
                                    None => {
                                        let mut t = text(display_name).size(font_size);
                                        if let Some(f) = custom_font {
                                            t = t.font(f);
                                        }
                                        button(
                                            container(t)
                                                .align_x(alignment::Horizontal::Center)
                                                .align_y(alignment::Vertical::Center),
                                        )
                                        .style(theme.workspace_button_style(empty, color))
                                        .padding(padding)
                                        .on_press(on_press)
                                        .width(Length::Shrink)
                                        .height(height)
                                        .into()
                                    }
                                })
                            }
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>(),
            )
            .spacing(theme.space.xxs)
        });

        MouseArea::new(row)
            .on_scroll(move |direction| match direction {
                iced::mouse::ScrollDelta::Lines { y, .. } => {
                    if y.is_sign_positive() {
                        match self.config.invert_scroll_direction {
                            Some(InvertScrollDirection::All | InvertScrollDirection::Mouse) => {
                                Message::Scroll(-1)
                            }
                            Some(InvertScrollDirection::Trackpad) => Message::Scroll(1),
                            None => Message::Scroll(1),
                        }
                    } else {
                        match self.config.invert_scroll_direction {
                            Some(InvertScrollDirection::All | InvertScrollDirection::Mouse) => {
                                Message::Scroll(1)
                            }
                            Some(InvertScrollDirection::Trackpad) => Message::Scroll(-1),
                            None => Message::Scroll(-1),
                        }
                    }
                }
                iced::mouse::ScrollDelta::Pixels { y, .. } => {
                    let sensibility = 3.;

                    if self.scroll_accumulator.abs() < sensibility {
                        Message::ScrollAccumulator(y)
                    } else if self.scroll_accumulator.is_sign_positive() {
                        match self.config.invert_scroll_direction {
                            Some(InvertScrollDirection::All | InvertScrollDirection::Trackpad) => {
                                Message::Scroll(-1)
                            }
                            Some(InvertScrollDirection::Mouse) => Message::Scroll(1),
                            None => Message::Scroll(1),
                        }
                    } else {
                        match self.config.invert_scroll_direction {
                            Some(InvertScrollDirection::All | InvertScrollDirection::Trackpad) => {
                                Message::Scroll(1)
                            }
                            Some(InvertScrollDirection::Mouse) => Message::Scroll(-1),
                            None => Message::Scroll(-1),
                        }
                    }
                }
            })
            .into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        CompositorService::subscribe().map(Message::ServiceEvent)
    }
}

/// `[workspaces.colors]` blokundaki "#cba6f7" gibi hex string'i
/// AppearanceColor::Simple'a çevir. Geçersizse None.
fn parse_hex_color(hex: &str) -> Option<AppearanceColor> {
    hex_color::HexColor::parse(hex)
        .ok()
        .map(AppearanceColor::Simple)
}

/// Wraps a workspace `pill` button in a Stack with a bottom-edge
/// indicator. Active workspace gets a 2.5px accent bar (~55% of pill
/// width); inactive workspaces with open windows get a row of small
/// dots (max 4) signalling occupancy. Empty inactive tags get no
/// indicator. The Stack overlay sits inside the pill's footprint so
/// bar height stays unchanged.
fn build_pill_with_indicator<'a>(
    theme: &MshellTheme,
    pill: Element<'a, Message>,
    displayed: &Displayed,
    windows: u16,
    show_count: bool,
    color: Option<Option<AppearanceColor>>,
    pill_w: f32,
    pill_h: f32,
) -> Element<'a, Message> {
    let indicator: Element<'a, Message> = match displayed {
        Displayed::Active => {
            let bar_w = (pill_w * 0.55).max(10.0);
            container(
                Space::new()
                    .width(Length::Fixed(bar_w))
                    .height(Length::Fixed(2.5)),
            )
            .style(theme.workspace_active_indicator_style(color))
            .into()
        }
        _ if windows > 0 && show_count => {
            let n = (windows as usize).min(4);
            let dots: Vec<Element<'a, Message>> = (0..n)
                .map(|_| {
                    container(
                        Space::new()
                            .width(Length::Fixed(3.0))
                            .height(Length::Fixed(3.0)),
                    )
                    .style(theme.workspace_window_dot_style(color))
                    .into()
                })
                .collect();
            Row::with_children(dots).spacing(2.0).into()
        }
        _ => Space::new().into(),
    };

    let indicator_layer = container(indicator)
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Bottom)
        .width(Length::Fill)
        .height(Length::Fixed(pill_h));

    Stack::new().push(pill).push(indicator_layer).into()
}
