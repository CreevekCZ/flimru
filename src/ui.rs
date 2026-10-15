use crate::{App, Page};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap,
    },
};

const ACCENT: Color = Color::Rgb(96, 210, 225);
const MUTED: Color = Color::Rgb(151, 163, 180);
const GREEN: Color = Color::Rgb(128, 214, 159);
const PANEL: Color = Color::Rgb(22, 28, 39);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    SelectDevice(usize),
    SelectConfig(usize),
    ToggleFavorite(usize),
    NextOrRun,
    Back,
    Refresh,
    ReloadConfigs,
    LastLog,
    Quit,
    Help,
    DismissHelp,
    ToggleAnimations,
    ConsoleScroll(u16),
    ConfirmQuit,
    CancelQuit,
    Actions,
    DismissActions,
    SelectAction(usize),
    Filter,
    ApplyFilter,
    ClearFilter,
    ToggleColors,
    FlutterKey(char),
    OldestOutput,
    FollowOutput,
    ScrollActions(usize),
    ToggleMulti,
    ToggleDevice(usize),
    PreviousConsole,
    NextConsole,
    SwitchConsole(usize),
    Devices,
    StopAll,
    AddSession,
}

pub struct ActionItem {
    pub label: String,
    pub shortcut: &'static str,
    pub action: MouseAction,
}

/// One command inventory drives the menu for both pointer and keyboard input.
pub fn available_actions(app: &App) -> Vec<ActionItem> {
    let mut items = Vec::new();
    let mut add = |label: &str, shortcut, action| {
        items.push(ActionItem {
            label: label.into(),
            shortcut,
            action,
        });
    };
    add("Add session", "F9", MouseAction::AddSession);
    match app.page {
        Page::Devices | Page::Configs => {
            if app.page == Page::Configs {
                add("Back to devices", "Esc", MouseAction::Back);
            }
            add(
                if app.page == Page::Devices {
                    if app.multi {
                        "Choose configuration"
                    } else {
                        "Choose device"
                    }
                } else {
                    "Run configuration"
                },
                "Enter",
                MouseAction::NextOrRun,
            );
            add("Filter list", "/", MouseAction::Filter);
            if app.search.is_some() {
                add("Apply filter", "Enter", MouseAction::ApplyFilter);
                add("Clear filter", "Esc", MouseAction::ClearFilter);
            }
            if app.page == Page::Devices && !app.visible_devices().is_empty() {
                add(
                    "Toggle favorite",
                    "f",
                    MouseAction::ToggleFavorite(app.device_index),
                );
            }
            if app.page == Page::Devices {
                add(
                    if app.multi {
                        "Multi-device: on"
                    } else {
                        "Multi-device: off"
                    },
                    "m",
                    MouseAction::ToggleMulti,
                );
                if app.multi && !app.visible_devices().is_empty() {
                    add(
                        "Check / uncheck device",
                        "Space",
                        MouseAction::ToggleDevice(app.device_index),
                    );
                }
            }
            add("Refresh devices", "r", MouseAction::Refresh);
            add("Reload configs", "c", MouseAction::ReloadConfigs);
            if app.last_log.is_some() {
                add("Last console", "l", MouseAction::LastLog);
            }
        }
        Page::Console => {
            add("Device picker", "F7", MouseAction::Devices);
            if app.active_session().is_some() {
                add("Hot reload", "r", MouseAction::FlutterKey('r'));
                add("Hot restart", "R", MouseAction::FlutterKey('R'));
                add("Flutter help", "h", MouseAction::FlutterKey('h'));
                add("Open DevTools", "v", MouseAction::FlutterKey('v'));
                add("Animation speed", "F2", MouseAction::ToggleAnimations);
            }
            add("Oldest output", "Home", MouseAction::OldestOutput);
            add("Follow output", "End", MouseAction::FollowOutput);
            add(
                if app.active_session().is_some() {
                    "Stop session"
                } else {
                    "Back to devices"
                },
                "q / Esc",
                MouseAction::Back,
            );
        }
        Page::LastLog => {
            add("Back to devices", "Esc", MouseAction::Back);
            add("Oldest output", "Home", MouseAction::OldestOutput);
            add("Latest output", "End", MouseAction::FollowOutput);
        }
    }
    if !app.runs.is_empty() {
        add("Previous console", "F5", MouseAction::PreviousConsole);
        add("Next console", "F6", MouseAction::NextConsole);
        for (index, run) in app.runs.iter().enumerate() {
            add(
                &format!(
                    "Console {}: {} · {} ({})",
                    index + 1,
                    run.device.name,
                    run.config,
                    if run.session.is_some() {
                        "running"
                    } else {
                        "ended"
                    }
                ),
                "",
                MouseAction::SwitchConsole(index),
            );
        }
        if app.running_count() > 0 {
            add("Stop all sessions", "F8", MouseAction::StopAll);
        }
    }
    add(
        if app.colors {
            "Colors: on"
        } else {
            "Colors: off"
        },
        "F3",
        MouseAction::ToggleColors,
    );
    add("Launcher help", "F1", MouseAction::Help);
    add("Quit launcher", "Ctrl+q", MouseAction::Quit);
    items
}

fn action_menu_layout(area: Rect, count: usize) -> (Rect, Rect, Rect) {
    let width = area.width.saturating_sub(2).min(66);
    let height =
        (count.saturating_add(3).min(u16::MAX as usize) as u16).min(area.height.saturating_sub(2));
    let modal = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    let inner = modal.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 1,
    });
    (
        modal,
        Rect::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height.saturating_sub(1),
        ),
        Rect::new(
            inner.x,
            inner.bottom().saturating_sub(1),
            inner.width,
            inner.height.min(1),
        ),
    )
}

fn search_buttons(area: Rect) -> (Rect, Rect) {
    let x = area.right().saturating_sub(15).max(area.x);
    (
        Rect::new(x, area.y, 7.min(area.width), area.height.min(1)),
        Rect::new(
            x.saturating_add(8),
            area.y,
            7.min(area.width),
            area.height.min(1),
        ),
    )
}

pub fn update_action_viewport(app: &mut App, area: Rect) {
    let count = available_actions(app).len();
    let (_, list, _) = action_menu_layout(area, count);
    let height = usize::from(list.height).max(1);
    app.action_index = app.action_index.min(count.saturating_sub(1));
    app.action_offset = app.action_offset.min(count.saturating_sub(height));
    if app.action_index < app.action_offset {
        app.action_offset = app.action_index;
    } else if app.action_index >= app.action_offset + height {
        app.action_offset = app.action_index.saturating_sub(height - 1);
    }
}

pub fn action_scroll_index(app: &App, area: Rect, row: u16) -> usize {
    let count = available_actions(app).len();
    let (_, list, _) = action_menu_layout(area, count);
    let relative = row
        .saturating_sub(list.y)
        .min(list.height.saturating_sub(1));
    usize::from(relative) * count.saturating_sub(1)
        / usize::from(list.height.saturating_sub(1)).max(1)
}

fn body_layout(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(2),
        Constraint::Length(2),
    ])
    .split(area)
}

fn picker_layout(area: Rect) -> std::rc::Rc<[Rect]> {
    if area.width >= 90 {
        Layout::horizontal([Constraint::Percentage(53), Constraint::Percentage(47)]).split(area)
    } else {
        Layout::horizontal([Constraint::Percentage(100), Constraint::Length(0)]).split(area)
    }
}

fn list_offset(selected: usize, count: usize, height: u16, previous: usize) -> usize {
    if count == 0 {
        return 0;
    }
    let capacity = usize::from(height / 2).max(1);
    let selected = selected.min(count - 1);
    let offset = previous.min(count.saturating_sub(capacity));
    if selected < offset {
        selected
    } else if selected >= offset + capacity {
        selected.saturating_sub(capacity - 1)
    } else {
        offset
    }
}

pub fn update_picker_viewport(app: &mut App, area: Rect) {
    let list = picker_layout(body_layout(area)[2])[0];
    let height = panel(String::new()).inner(list).height;
    app.device_list_offset = list_offset(
        app.device_index,
        app.visible_devices().len(),
        height,
        app.device_list_offset,
    );
    app.config_list_offset = list_offset(
        app.config_index,
        app.visible_configs().len(),
        height,
        app.config_list_offset,
    );
}

fn list_hit(
    inner: Rect,
    selected: usize,
    count: usize,
    previous: usize,
    x: u16,
    y: u16,
) -> Option<usize> {
    if !inner.contains((x, y).into()) || inner.height < 2 {
        return None;
    }
    let row = usize::from((y - inner.y) / 2);
    // The list only renders complete two-line entries; ignore the spare bottom row.
    if row >= usize::from(inner.height / 2) {
        return None;
    }
    let index = list_offset(selected, count, inner.height, previous) + row;
    (index < count).then_some(index)
}

fn navigation_tabs(area: Rect) -> Vec<(&'static str, Rect)> {
    let labels = if area.width < 52 {
        ["  1 Device", " › 2 Config", " › 3 Console"]
    } else {
        [
            "  1  Device",
            "   ›   2  Configuration",
            "   ›   3  Console",
        ]
    };
    let mut x = area.x;
    labels
        .into_iter()
        .map(|label| {
            let width = (label.chars().count() as u16).min(area.right().saturating_sub(x));
            let rect = Rect::new(x, area.y, width, area.height.min(1));
            x += width;
            (label, rect)
        })
        .collect()
}

struct Control {
    label: String,
    rect: Rect,
    action: MouseAction,
}

fn console_navigation(app: &App, area: Rect) -> Vec<Control> {
    let Some(run) = app.runs.get(app.active_run) else {
        return vec![];
    };
    let state = if run.session.is_some() {
        "running"
    } else {
        "ended"
    };
    let summary = format!(
        " {}/{} {state} · {}",
        app.active_run + 1,
        app.runs.len(),
        run.device.name
    );
    vec![
        Control {
            label: "[F5 <]".into(),
            rect: Rect::new(area.x, area.y, 6, 1),
            action: MouseAction::PreviousConsole,
        },
        Control {
            label: "[F6 >]".into(),
            rect: Rect::new(area.x + 7, area.y, 6, 1),
            action: MouseAction::NextConsole,
        },
        Control {
            label: summary,
            rect: Rect::new(area.x + 14, area.y, area.width.saturating_sub(14), 1),
            action: MouseAction::Actions,
        },
    ]
}

fn controls(app: &App, area: Rect) -> Vec<Control> {
    let compact = area.width < 76;
    let mut buttons = vec![];
    match app.page {
        Page::Devices => {
            buttons.push(("Next".into(), MouseAction::NextOrRun));
            if !compact {
                buttons.push((
                    format!("m Multi {}", if app.multi { "on" } else { "off" }),
                    MouseAction::ToggleMulti,
                ));
                buttons.push(("Refresh".into(), MouseAction::Refresh));
                buttons.push(("Configs".into(), MouseAction::ReloadConfigs));
                buttons.push(("Last console".into(), MouseAction::LastLog));
            } else if area.width >= 34 {
                buttons.push(("Refresh".into(), MouseAction::Refresh));
            }
        }
        Page::Configs => {
            buttons.push(("Back".into(), MouseAction::Back));
            buttons.push(("Run".into(), MouseAction::NextOrRun));
            if !compact {
                buttons.push(("Refresh".into(), MouseAction::Refresh));
                buttons.push(("Configs".into(), MouseAction::ReloadConfigs));
            }
        }
        Page::Console => {
            buttons.push((
                if app.active_session().is_some() {
                    "Stop"
                } else {
                    "Back"
                }
                .into(),
                MouseAction::Back,
            ));
            if !compact {
                buttons.push(("F9 Add".into(), MouseAction::AddSession));
            }
            if let Some(session) = app.active_session() {
                buttons.push((
                    if compact {
                        format!("F2 {}", session.animations.short_label())
                    } else {
                        format!("F2 {}", session.animations.label())
                    },
                    MouseAction::ToggleAnimations,
                ));
            }
            buttons.push((
                format!("F3 Colors {}", if app.colors { "on" } else { "off" }),
                MouseAction::ToggleColors,
            ));
        }
        Page::LastLog => buttons.push(("Back".into(), MouseAction::Back)),
    }
    buttons.push((
        if compact {
            "Menu".into()
        } else {
            "F4 Actions".into()
        },
        MouseAction::Actions,
    ));
    buttons.push(("Help".into(), MouseAction::Help));
    buttons.push(("Quit".into(), MouseAction::Quit));
    let essential_start = buttons.len() - 3;
    let mut x = area.x + 1;
    buttons
        .into_iter()
        .enumerate()
        .filter_map(|(index, (mut label, action))| {
            // Keep Help and Quit visible even if an externally changed animation
            // factor needs more space than the usual 1× / 5× label.
            let reserved = if index < essential_start {
                if compact { 21 } else { 33 }
            } else {
                0
            };
            let available = area.right().saturating_sub(x).saturating_sub(reserved);
            let padding = if compact { 2 } else { 4 };
            if action == MouseAction::ToggleAnimations
                && label.chars().count() + padding > usize::from(available)
            {
                let limit = usize::from(available).saturating_sub(padding);
                label = label
                    .chars()
                    .take(limit.saturating_sub(1))
                    .collect::<String>();
                label.push('…');
            }
            let label = if compact {
                format!("[{label}]")
            } else {
                format!("[ {label} ]")
            };
            let width = label.chars().count() as u16;
            if width > available {
                return None;
            }
            let rect = Rect::new(x, area.y + 1, width, 1);
            x += width + 1;
            Some(Control {
                label,
                rect,
                action,
            })
        })
        .collect()
}

fn quit_modal(area: Rect) -> (Rect, Rect, Rect) {
    let width = area.width.saturating_sub(4).min(54);
    let height = area.height.saturating_sub(2).min(8);
    let modal = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let buttons_x = modal.x + (width.saturating_sub(21)) / 2;
    let y = modal.bottom().saturating_sub(3);
    (
        modal,
        Rect::new(buttons_x, y, 8, 1),
        Rect::new(buttons_x + 11, y, 10, 1),
    )
}

fn small_exit_buttons(area: Rect) -> Option<(Rect, Rect)> {
    (area.width >= 18 && area.height >= 3).then(|| {
        (
            Rect::new(area.x + 1, area.bottom() - 1, 6, 1),
            Rect::new(area.x + 9, area.bottom() - 1, 8, 1),
        )
    })
}

/// The scrollbar gutter for the full terminal area, shared with pointer dragging.
pub fn console_scrollbar_area(area: Rect) -> Rect {
    let inner = panel(String::new()).inner(body_layout(area)[2]);
    Rect::new(
        inner.right().saturating_sub(1),
        inner.y,
        inner.width.min(1),
        inner.height,
    )
}

pub fn mouse_action(app: &App, area: Rect, event: MouseEvent) -> Option<MouseAction> {
    if !matches!(event.kind, MouseEventKind::Down(MouseButton::Left)) {
        return None;
    }
    if area.width < 34 || area.height < 12 {
        let (quit, cancel) = small_exit_buttons(area)?;
        return if quit.contains((event.column, event.row).into()) {
            Some(if app.quit_confirmation {
                MouseAction::ConfirmQuit
            } else {
                MouseAction::Quit
            })
        } else if app.quit_confirmation && cancel.contains((event.column, event.row).into()) {
            Some(MouseAction::CancelQuit)
        } else {
            None
        };
    }
    if app.quit_confirmation {
        let (_, quit, cancel) = quit_modal(area);
        return if quit.contains((event.column, event.row).into()) {
            Some(MouseAction::ConfirmQuit)
        } else if cancel.contains((event.column, event.row).into()) {
            Some(MouseAction::CancelQuit)
        } else {
            None
        };
    }
    if app.help {
        return Some(MouseAction::DismissHelp);
    }
    if app.actions_open {
        let count = available_actions(app).len();
        let (_, list, _) = action_menu_layout(area, count);
        return if list.contains((event.column, event.row).into()) {
            if count > usize::from(list.height) && event.column == list.right().saturating_sub(1) {
                return Some(MouseAction::ScrollActions(action_scroll_index(
                    app, area, event.row,
                )));
            }
            let index = app.action_offset + usize::from(event.row - list.y);
            (index < count).then_some(MouseAction::SelectAction(index))
        } else {
            Some(MouseAction::DismissActions)
        };
    }
    let parts = body_layout(area);
    if matches!(app.page, Page::Devices | Page::Configs)
        && parts[3].contains((event.column, event.row).into())
    {
        let (apply, clear) = search_buttons(parts[3]);
        return Some(
            if app.search.is_some() && clear.contains((event.column, event.row).into()) {
                MouseAction::ClearFilter
            } else if app.search.is_some() && apply.contains((event.column, event.row).into()) {
                MouseAction::ApplyFilter
            } else {
                MouseAction::Filter
            },
        );
    }
    for control in controls(app, parts[0]) {
        if control.rect.contains((event.column, event.row).into()) {
            return Some(control.action);
        }
    }
    if app.page == Page::Console && app.runs.len() > 1 {
        for control in console_navigation(app, parts[1]) {
            if control.rect.contains((event.column, event.row).into()) {
                return Some(control.action);
            }
        }
    }
    for (index, (_, rect)) in navigation_tabs(parts[1]).iter().enumerate() {
        if app.page == Page::Console && app.runs.len() > 1 {
            break;
        }
        if rect.contains((event.column, event.row).into()) {
            return match (index, app.page) {
                (0, Page::Configs | Page::LastLog) => Some(MouseAction::Back),
                (1, Page::Devices) => Some(MouseAction::NextOrRun),
                (2, Page::Devices | Page::Configs) if app.last_log.is_some() => {
                    Some(MouseAction::LastLog)
                }
                (2, Page::Devices | Page::Configs) if !app.runs.is_empty() => {
                    Some(MouseAction::SwitchConsole(app.active_run))
                }
                _ => None,
            };
        }
    }
    match app.page {
        Page::Devices | Page::Configs => {
            let list = picker_layout(parts[2])[0];
            let inner = panel(String::new()).inner(list);
            let (selected, count, previous) = if app.page == Page::Devices {
                (
                    app.device_index,
                    app.visible_devices().len(),
                    app.device_list_offset,
                )
            } else {
                (
                    app.config_index,
                    app.visible_configs().len(),
                    app.config_list_offset,
                )
            };
            list_hit(inner, selected, count, previous, event.column, event.row).map(|index| {
                if app.page == Page::Configs {
                    MouseAction::SelectConfig(index)
                } else if event.column == inner.x + 3 && (event.row - inner.y).is_multiple_of(2) {
                    MouseAction::ToggleFavorite(index)
                } else if app.multi
                    && (inner.x + 5..inner.x + 9).contains(&event.column)
                    && (event.row - inner.y).is_multiple_of(2)
                {
                    MouseAction::ToggleDevice(index)
                } else {
                    MouseAction::SelectDevice(index)
                }
            })
        }
        Page::Console | Page::LastLog => {
            let track = console_scrollbar_area(area);
            track
                .contains((event.column, event.row).into())
                .then_some(MouseAction::ConsoleScroll(
                    event.row.saturating_sub(track.y),
                ))
        }
    }
}

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    frame.render_widget(
        Block::default().style(
            Style::default()
                .bg(Color::Rgb(14, 19, 27))
                .fg(Color::Rgb(224, 232, 240)),
        ),
        area,
    );
    if area.width < 34 || area.height < 12 {
        frame.render_widget(
            Paragraph::new(if app.quit_confirmation {
                "Quit Flutter Run?\ny quits · Enter/Esc cancels"
            } else {
                "Enlarge the terminal to at least 34 × 12.\nCtrl+q opens quit confirmation."
            })
            .wrap(Wrap { trim: false }),
            area,
        );
        if let Some((quit, cancel)) = small_exit_buttons(area) {
            frame.render_widget(Clear, Rect::new(area.x, quit.y, area.width, 1));
            frame.render_widget(
                Paragraph::new("[Quit]").style(Style::default().fg(Color::Yellow)),
                quit,
            );
            if app.quit_confirmation {
                frame.render_widget(
                    Paragraph::new("[Cancel]").style(Style::default().fg(ACCENT)),
                    cancel,
                );
            }
        }
        return;
    }
    let parts = body_layout(area);
    let project = app
        .launch
        .workspace
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let mut title = vec![
        Span::styled("  FLUTTER RUN ", Style::default().fg(ACCENT).bold()),
        Span::styled(format!(" / {project}"), Style::default().fg(MUTED)),
    ];
    if app.busy {
        title.push(Span::styled(
            "    ◌ discovering",
            Style::default().fg(ACCENT),
        ));
    } else if let Some(when) = app.refreshed {
        title.push(Span::styled(
            format!("    updated {}s ago", when.elapsed().as_secs()),
            Style::default().fg(MUTED),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(title)), parts[0]);
    for control in controls(app, parts[0]) {
        frame.render_widget(
            Paragraph::new(control.label).style(Style::default().fg(ACCENT)),
            control.rect,
        );
    }
    let active = match app.page {
        Page::Devices => 0,
        Page::Configs => 1,
        _ => 2,
    };
    if app.page == Page::Console && app.runs.len() > 1 {
        for control in console_navigation(app, parts[1]) {
            frame.render_widget(
                Paragraph::new(control.label).style(Style::default().fg(ACCENT).bold()),
                control.rect,
            );
        }
    } else {
        frame.render_widget(
            Paragraph::new(Line::from(
                navigation_tabs(parts[1])
                    .iter()
                    .enumerate()
                    .map(|(i, (text, _))| {
                        Span::styled(
                            *text,
                            Style::default()
                                .fg(if i == active { ACCENT } else { MUTED })
                                .add_modifier(if i == active {
                                    Modifier::BOLD
                                } else {
                                    Modifier::empty()
                                }),
                        )
                    })
                    .collect::<Vec<_>>(),
            )),
            parts[1],
        );
    }

    match app.page {
        Page::Devices | Page::Configs => picker(frame, app, parts[2]),
        Page::Console => {
            if let Some(run) = app.runs.get(app.active_run)
                && let Some(screen) = run.screen()
            {
                console(
                    frame,
                    screen,
                    run.scrollback(),
                    run.outcome.as_ref().map(|_| run.offset),
                    app.colors,
                    parts[2],
                    &format!(
                        " {} · {}{} ",
                        run.config,
                        run.device.name,
                        run.outcome
                            .as_ref()
                            .map(|s| format!(" · ended ({s})"))
                            .unwrap_or_else(|| {
                                if run
                                    .session
                                    .as_ref()
                                    .is_some_and(|s| s.stop_requested.is_some())
                                {
                                    " · stopping (Esc again forces)".into()
                                } else {
                                    String::new()
                                }
                            })
                    ),
                );
            }
        }
        Page::LastLog => {
            if let Some(parser) = &app.last_log {
                console(
                    frame,
                    parser.screen(),
                    app.last_log_scrollback,
                    Some(app.last_log_offset),
                    app.colors,
                    parts[2],
                    &format!(" Last console · {} ", app.active_config),
                );
            }
        }
    }

    let status = if let Some(search) = &app.search {
        format!(
            "  / {search}{}  {}",
            if app.searching { "▏" } else { "" },
            if app.searching {
                "Enter applies · Esc clears"
            } else {
                "Esc clears filter"
            }
        )
    } else {
        format!("  {}", app.status)
    };
    let picker = matches!(app.page, Page::Devices | Page::Configs);
    let mut status_area = parts[3];
    if picker {
        let (apply, clear) = search_buttons(parts[3]);
        if app.search.is_some() {
            status_area.width = apply.x.saturating_sub(status_area.x);
            frame.render_widget(
                Paragraph::new("[Apply]").style(Style::default().fg(ACCENT)),
                apply,
            );
            frame.render_widget(
                Paragraph::new("[Clear]").style(Style::default().fg(ACCENT)),
                clear,
            );
        } else {
            let filter = Rect::new(parts[3].right().saturating_sub(10), parts[3].y, 10, 1);
            status_area.width = filter.x.saturating_sub(status_area.x);
            frame.render_widget(
                Paragraph::new("[Filter /]").style(Style::default().fg(ACCENT)),
                filter,
            );
        }
    }
    frame.render_widget(
        Paragraph::new(status)
            .style(Style::default().fg(if app.search.is_some() { ACCENT } else { MUTED }))
            .wrap(Wrap { trim: false }),
        status_area,
    );
    let help = match app.page {
        Page::Devices => {
            if app.multi {
                "  Space/[ ] check · Enter next · m single mode\n  F4/Menu actions · F6 consoles · Ctrl+q quit"
            } else {
                "  Click/↑↓ select · ☆/f favorite · Enter next\n  m multi-device · F4/Menu actions · Ctrl+q quit"
            }
        }
        Page::Configs => {
            "  Click/↑↓ select · Enter run · Esc back · / filter\n  F4/Menu all actions · Ctrl+q quit · F1 help"
        }
        Page::Console => {
            "  r reload · R restart · F2 slow · F4 menu\n  F5/F6 switch · F9 add · q/Esc stop this"
        }
        Page::LastLog => {
            "  Wheel/PgUp/PgDn scroll · Home/End · F3 colors\n  F4/Menu all actions · Esc back · Ctrl+q quit"
        }
    };
    frame.render_widget(
        Paragraph::new(help).style(Style::default().fg(MUTED)),
        parts[4],
    );
    if app.help {
        help_overlay(frame);
    }
    if app.actions_open {
        actions_overlay(frame, app);
    }
    if app.quit_confirmation {
        quit_overlay(frame, app);
    }
}

fn panel(title: String) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::Rgb(60, 76, 96)))
        .style(Style::default().bg(PANEL))
}

fn picker(frame: &mut Frame, app: &App, area: Rect) {
    let split = picker_layout(area);
    let (items, selected, title) = if app.page == Page::Devices {
        let devices = app.visible_devices();
        let items: Vec<_> = devices
            .iter()
            .map(|device| {
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(
                            if device.favorite { " ★ " } else { " ☆ " },
                            Style::default().fg(if device.favorite {
                                Color::Yellow
                            } else {
                                MUTED
                            }),
                        ),
                        Span::styled(
                            if app.multi {
                                if app.marked_devices.iter().any(|d| d.id == device.id) {
                                    "[x] "
                                } else {
                                    "[ ] "
                                }
                            } else {
                                ""
                            },
                            Style::default().fg(ACCENT),
                        ),
                        Span::styled(device.name.clone(), Style::default().bold()),
                    ]),
                    Line::from(vec![
                        Span::raw("   "),
                        Span::styled(
                            format!(
                                "{} · {}",
                                device.platform,
                                if device.available {
                                    "available"
                                } else if device.favorite {
                                    "saved · try now"
                                } else {
                                    "configured · try now"
                                }
                            ),
                            Style::default().fg(if device.available { GREEN } else { MUTED }),
                        ),
                    ]),
                ])
            })
            .collect();
        (
            items,
            app.device_index,
            if app.multi {
                format!(
                    " Devices · Multi {}/{} ",
                    app.marked_devices.len(),
                    crate::runs::MAX_RUNS
                )
            } else {
                format!(" Devices ({}) ", devices.len())
            },
        )
    } else {
        let configs = app.visible_configs();
        let items: Vec<_> = configs
            .iter()
            .map(|config| {
                ListItem::new(vec![
                    Line::from(Span::styled(
                        format!(" {}", config.name),
                        Style::default().bold(),
                    )),
                    Line::from(Span::styled(
                        format!(
                            " {}{}",
                            if config.is_test() { "test" } else { "run" },
                            config
                                .flutter_mode
                                .as_ref()
                                .map(|mode| format!(" · {mode}"))
                                .unwrap_or_default()
                        ),
                        Style::default().fg(MUTED),
                    )),
                ])
            })
            .collect();
        (
            items,
            app.config_index,
            format!(" Configurations ({}) ", configs.len()),
        )
    };
    let count = items.len();
    let list = List::new(items)
        .block(panel(title))
        .highlight_style(Style::default().bg(Color::Rgb(34, 64, 78)).fg(Color::White))
        .highlight_symbol("› ");
    let inner = panel(String::new()).inner(split[0]);
    let previous = if app.page == Page::Devices {
        app.device_list_offset
    } else {
        app.config_list_offset
    };
    let mut state = ListState::default()
        .with_offset(list_offset(selected, count, inner.height, previous))
        .with_selected(if count == 0 {
            None
        } else {
            Some(selected.min(count - 1))
        });
    frame.render_stateful_widget(list, split[0], &mut state);
    if count == 0 {
        let empty = if app.page == Page::Devices {
            "No devices yet.\n\nConnect a device or start an emulator.\nSaved favorites appear immediately.\nPress r to refresh."
        } else {
            "No matching configurations.\n\nA configuration with deviceId is offered\non that configured device.\nPress Esc to select another device."
        };
        frame.render_widget(
            Paragraph::new(empty)
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(MUTED)),
            split[0].inner(ratatui::layout::Margin {
                horizontal: 3,
                vertical: 2,
            }),
        );
    }
    if split[1].width == 0 {
        return;
    }
    let detail = if app.page == Page::Devices {
        if let Some(device) = app.visible_devices().get(app.device_index) {
            vec![
                Line::styled(device.name.clone(), Style::default().fg(ACCENT).bold()),
                Line::raw(""),
                Line::raw(format!("ID  {}", device.id)),
                Line::raw(format!("Platform  {}", device.platform)),
                Line::raw(""),
                Line::raw(if device.available {
                    "Seen in the last successful discovery."
                } else {
                    "This target is cached. You can launch it while discovery is still running."
                }),
                Line::raw(""),
                Line::raw(if device.favorite {
                    "★ Favorite — click the star or press f to remove."
                } else {
                    "Click ☆ or press f to save as a favorite."
                }),
                Line::raw(""),
                Line::raw("Enter → choose configuration"),
            ]
        } else {
            vec![
                Line::raw("Discovery runs in the background."),
                Line::raw("You can select saved favorites immediately."),
            ]
        }
    } else if let Some(config) = app.visible_configs().get(app.config_index) {
        let mut lines = vec![
            Line::styled(config.name.clone(), Style::default().fg(ACCENT).bold()),
            Line::raw(""),
            Line::raw(format!(
                "Device  {}",
                app.targets()
                    .iter()
                    .map(|d| d.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            Line::raw(format!("Directory  {}", config.cwd.display())),
            Line::raw(format!(
                "Target  {}",
                config.program.as_deref().unwrap_or("lib/main.dart")
            )),
            Line::raw(""),
            Line::raw("Flutter options"),
        ];
        let mut redact_next = false;
        for flag in config.tool_args.iter().chain(&config.args) {
            let text = if redact_next {
                redact_next = false;
                "  <redacted>".into()
            } else if flag.starts_with("--dart-define=") {
                "  --dart-define=<redacted>".into()
            } else {
                if flag == "--dart-define" {
                    redact_next = true;
                }
                format!("  {flag}")
            };
            lines.push(Line::raw(text));
        }
        if let Some(task) = &config.pre_launch_task {
            lines.push(Line::styled(
                format!("Blocked: preLaunchTask '{task}' is not supported."),
                Style::default().fg(Color::Yellow),
            ));
        }
        lines
    } else {
        vec![Line::raw(
            "Choose another device to see its configurations.",
        )]
    };
    let mut detail = detail;
    if !app.launch.warnings.is_empty() {
        detail.push(Line::raw(""));
        detail.push(Line::styled(
            format!("{} configuration warning(s)", app.launch.warnings.len()),
            Style::default().fg(Color::Yellow),
        ));
        detail.extend(app.launch.warnings.iter().map(|w| Line::raw(w.clone())));
    }
    frame.render_widget(
        Paragraph::new(detail)
            .block(panel(" Details ".into()))
            .wrap(Wrap { trim: false }),
        split[1],
    );
}

fn terminal_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(index) => Color::Indexed(index),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

fn console(
    frame: &mut Frame,
    screen: &vt100::Screen,
    scrollback_len: usize,
    snapshot_offset: Option<usize>,
    colors: bool,
    area: Rect,
    title: &str,
) {
    let offset = snapshot_offset.unwrap_or(screen.scrollback());
    let block = panel(format!(
        "{title}{}",
        if offset > 0 { " · scrollback " } else { "" }
    ));
    let inner = block.inner(area);
    let content = Rect::new(
        inner.x,
        inner.y,
        inner.width.saturating_sub(1),
        inner.height,
    );
    let track = Rect::new(
        inner.right().saturating_sub(1),
        inner.y,
        inner.width.min(1),
        inner.height,
    );
    frame.render_widget(block, area);
    let (rows, cols) = screen.size();
    let hidden_rows = if snapshot_offset.is_some() {
        rows.saturating_sub(content.height)
    } else {
        0
    };
    let start_row = hidden_rows.saturating_sub(offset.min(u16::MAX as usize) as u16);
    let capacity = scrollback_len.saturating_add(usize::from(hidden_rows));
    let lines: Vec<_> = (0..rows.saturating_sub(start_row).min(content.height))
        .map(|row| {
            let mut spans = Vec::new();
            let mut plain = String::new();
            let mut ansi_colors = false;
            for col in 0..cols.min(content.width) {
                if let Some(cell) = screen.cell(start_row + row, col) {
                    if cell.is_wide_continuation() {
                        continue;
                    }
                    let mut style = Style::default();
                    ansi_colors |= cell.fgcolor() != vt100::Color::Default
                        || cell.bgcolor() != vt100::Color::Default;
                    if colors && cell.fgcolor() != vt100::Color::Default {
                        style = style.fg(terminal_color(cell.fgcolor()));
                    }
                    if colors && cell.bgcolor() != vt100::Color::Default {
                        style = style.bg(terminal_color(cell.bgcolor()));
                    }
                    if cell.bold() {
                        style = style.bold();
                    }
                    if cell.italic() {
                        style = style.italic();
                    }
                    if cell.underline() {
                        style = style.underlined();
                    }
                    if cell.dim() {
                        style = style.dim();
                    }
                    if cell.inverse() {
                        style = style.reversed();
                    }
                    let text = if cell.has_contents() {
                        cell.contents().to_string()
                    } else {
                        " ".into()
                    };
                    plain.push_str(&text);
                    spans.push(Span::styled(text, style));
                }
            }
            if colors
                && !ansi_colors
                && let Some(color) = crate::console_style::fallback_color(&plain)
            {
                for span in &mut spans {
                    span.style = span.style.fg(color);
                }
            }
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(Text::from(lines)), content);
    let mut scrollbar = ScrollbarState::new(capacity.saturating_add(1))
        .viewport_content_length(usize::from(rows.min(content.height)))
        .position(capacity.saturating_sub(offset));
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("█")
            .style(Style::default().fg(if capacity > 0 { ACCENT } else { MUTED })),
        track,
        &mut scrollbar,
    );
    if snapshot_offset.is_none() && screen.scrollback() == 0 && !screen.hide_cursor() {
        let (row, col) = screen.cursor_position();
        if row < content.height && col < content.width {
            frame.set_cursor_position((content.x + col, content.y + row));
        }
    }
}

fn actions_overlay(frame: &mut Frame, app: &App) {
    let actions = available_actions(app);
    let (modal, list, footer) = action_menu_layout(frame.area(), actions.len());
    frame.render_widget(Clear, modal);
    frame.render_widget(panel(" Actions · F4 / Esc close ".into()), modal);
    let width = list.width.saturating_sub(1);
    let rows = actions
        .iter()
        .skip(app.action_offset)
        .take(list.height as usize)
        .enumerate();
    for (row, action) in rows {
        let hint_width = (action.shortcut.chars().count() as u16).min(width);
        let label_width = width.saturating_sub(hint_width + 5) as usize;
        let label: String = action.label.chars().take(label_width).collect();
        let hint = format!("{:>width$}", action.shortcut, width = hint_width as usize);
        let selected = app.action_offset + row == app.action_index;
        let line = Line::from(vec![
            Span::raw(if selected { "› " } else { "  " }),
            Span::raw(format!("{label:<label_width$}  ")),
            Span::styled(hint, Style::default().fg(MUTED)),
        ]);
        frame.render_widget(
            Paragraph::new(line).style(if selected {
                Style::default().bg(Color::Rgb(34, 64, 78)).fg(Color::White)
            } else {
                Style::default()
            }),
            Rect::new(list.x, list.y + row as u16, width, 1),
        );
    }
    if actions.len() > usize::from(list.height) {
        let mut state = ScrollbarState::new(actions.len())
            .position(app.action_index)
            .viewport_content_length(list.height as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .style(Style::default().fg(ACCENT)),
            list,
            &mut state,
        );
    }
    frame.render_widget(
        Paragraph::new("↑↓ / wheel · Enter / click").style(Style::default().fg(MUTED)),
        footer,
    );
}

fn help_overlay(frame: &mut Frame) {
    let area = frame.area();
    let width = area.width.saturating_sub(4).min(76);
    let height = area.height.saturating_sub(2).min(10);
    let modal = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, modal);
    frame.render_widget(Paragraph::new("m: multi · Space: check\nEnter: next/run · F4: menu\nF5/F6: console · F9: add\nq/Esc: stop · F8: stop all\nWheel / PgUp/PgDn: scroll\nF2: slow · F3: colors\nCtrl+q: confirm quit\nAny key / click: close").block(panel(" Keyboard and mouse ".into())).wrap(Wrap { trim: false }), modal);
}

fn quit_overlay(frame: &mut Frame, app: &App) {
    let (modal, quit, cancel) = quit_modal(frame.area());
    frame.render_widget(Clear, modal);
    frame.render_widget(panel(" Quit Flutter Run? ".into()), modal);
    let inner = modal.inner(ratatui::layout::Margin {
        horizontal: 2,
        vertical: 1,
    });
    let message = if app.running_count() > 1 {
        "All active Flutter sessions will be stopped."
    } else if app.running_count() == 1 {
        "The active Flutter session will be stopped."
    } else {
        "Close the launcher?"
    };
    frame.render_widget(
        Paragraph::new(message)
            .style(Style::default().fg(MUTED))
            .wrap(Wrap { trim: false }),
        Rect::new(inner.x, inner.y, inner.width, 2),
    );
    frame.render_widget(
        Paragraph::new("[ Quit ]").style(Style::default().fg(Color::Yellow)),
        quit,
    );
    frame.render_widget(
        Paragraph::new("[ Cancel ]").style(Style::default().fg(ACCENT).bold()),
        cancel,
    );
    frame.render_widget(
        Paragraph::new("y quit · Enter/Esc cancel").style(Style::default().fg(MUTED)),
        Rect::new(inner.x, modal.bottom().saturating_sub(2), inner.width, 1),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use flutter_run_tui::device::Device;
    use ratatui::{Terminal, backend::TestBackend};

    fn click(x: u16, y: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn row_text(buffer: &ratatui::buffer::Buffer, area: Rect, y: u16) -> String {
        (area.x..area.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn console_preserves_ansi_palette_rgb_background_and_modifiers() {
        let mut parser = vt100::Parser::new(5, 60, 10);
        parser.process(b"\x1b[1;3;4;31;44mRED\x1b[0m plain\r\n\x1b[38;5;208mINDEX\x1b[0m\r\n\x1b[38;2;12;200;99mRGB\x1b[0m\r\nWARNING: \x1b[35mcustom\x1b[0m body");
        let mut terminal = Terminal::new(TestBackend::new(64, 7)).unwrap();
        terminal
            .draw(|frame| {
                console(
                    frame,
                    parser.screen(),
                    0,
                    None,
                    true,
                    frame.area(),
                    " Test ",
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let red = &buffer[(1, 1)];
        assert_eq!(red.fg, Color::Indexed(1));
        assert_eq!(red.bg, Color::Indexed(4));
        assert!(
            red.modifier
                .contains(Modifier::BOLD | Modifier::ITALIC | Modifier::UNDERLINED)
        );
        assert_eq!(buffer[(5, 1)].fg, Color::Reset);
        assert_eq!(buffer[(1, 2)].fg, Color::Indexed(208));
        assert_eq!(buffer[(1, 3)].fg, Color::Rgb(12, 200, 99));
        // Do not repaint an ANSI-authored line, including its reset-to-default cells.
        assert_eq!(buffer[(1, 4)].fg, Color::Reset);
        assert_eq!(buffer[(10, 4)].fg, Color::Indexed(5));
    }

    #[test]
    fn plain_severity_colors_work_in_live_and_saved_console_and_can_be_disabled() {
        let mut parser = vt100::Parser::new(3, 60, 10);
        parser.process(b"flutter: [Orders] WARNING: retry\r\nflutter: [Orders] SEVERE: failed");
        let mut terminal = Terminal::new(TestBackend::new(64, 5)).unwrap();
        for snapshot in [None, Some(0)] {
            terminal
                .draw(|frame| {
                    console(
                        frame,
                        parser.screen(),
                        0,
                        snapshot,
                        true,
                        frame.area(),
                        " Test ",
                    )
                })
                .unwrap();
            assert_eq!(
                terminal.backend().buffer()[(1, 1)].fg,
                Color::Rgb(255, 205, 112)
            );
            assert_eq!(
                terminal.backend().buffer()[(1, 2)].fg,
                Color::Rgb(255, 128, 142)
            );
        }
        terminal
            .draw(|frame| {
                console(
                    frame,
                    parser.screen(),
                    0,
                    None,
                    false,
                    frame.area(),
                    " Test ",
                )
            })
            .unwrap();
        assert_eq!(terminal.backend().buffer()[(1, 1)].fg, Color::Reset);
    }

    #[test]
    fn every_menu_command_remains_visible_and_clickable_at_all_supported_sizes() {
        let mut app = crate::tests::app();
        app.last_log = Some(vt100::Parser::new(2, 2, 0));
        for page in [Page::Devices, Page::Configs, Page::Console, Page::LastLog] {
            app.page = page;
            for (width, height) in [(34, 12), (80, 24), (120, 40)] {
                let area = Rect::new(0, 0, width, height);
                assert!(
                    controls(&app, body_layout(area)[0])
                        .iter()
                        .any(|control| control.action == MouseAction::Actions)
                );
                app.actions_open = true;
                let actions = available_actions(&app);
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                for index in 0..actions.len() {
                    app.action_index = index;
                    update_action_viewport(&mut app, area);
                    terminal.draw(|frame| draw(frame, &app)).unwrap();
                    let (_, list, _) = action_menu_layout(area, actions.len());
                    let row = list.y + (index - app.action_offset) as u16;
                    assert!(row < list.bottom());
                    assert_eq!(
                        mouse_action(&app, area, click(list.x + 1, row)),
                        Some(MouseAction::SelectAction(index))
                    );
                    assert!(
                        row_text(terminal.backend().buffer(), list, row)
                            .contains(actions[index].shortcut)
                    );
                }
                app.actions_open = false;
            }
        }
    }

    #[test]
    fn multi_checkboxes_and_console_switches_match_rendered_targets() {
        let mut app = crate::tests::app();
        app.multi = true;
        let device = app.devices[0].clone();
        app.marked_devices = vec![device.clone()];
        for (width, height) in [(34, 12), (80, 24), (120, 40)] {
            let area = Rect::new(0, 0, width, height);
            app.page = Page::Devices;
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| draw(f, &app)).unwrap();
            let inner = panel(String::new()).inner(picker_layout(body_layout(area)[2])[0]);
            assert!(row_text(terminal.backend().buffer(), inner, inner.y).contains("[x]"));
            assert_eq!(
                mouse_action(&app, area, click(inner.x + 3, inner.y)),
                Some(MouseAction::ToggleFavorite(0))
            );
            assert_eq!(
                mouse_action(&app, area, click(inner.x + 6, inner.y)),
                Some(MouseAction::ToggleDevice(0))
            );
            app.runs = vec![
                crate::runs::Run::failed(device.clone(), "Demo".into(), "test".into(), 8, 40),
                crate::runs::Run::failed(device.clone(), "Demo".into(), "test".into(), 8, 40),
            ];
            app.switch_run(0);
            terminal.draw(|f| draw(f, &app)).unwrap();
            let nav = body_layout(area)[1];
            assert!(row_text(terminal.backend().buffer(), nav, nav.y).contains("1/2 ended"));
            assert_eq!(
                mouse_action(&app, area, click(nav.x + 2, nav.y)),
                Some(MouseAction::PreviousConsole)
            );
            assert_eq!(
                mouse_action(&app, area, click(nav.x + 9, nav.y)),
                Some(MouseAction::NextConsole)
            );
            app.actions_open = true;
            for index in 0..available_actions(&app).len() {
                app.action_index = index;
                update_action_viewport(&mut app, area);
                terminal.draw(|f| draw(f, &app)).unwrap();
                let (_, list, _) = action_menu_layout(area, available_actions(&app).len());
                assert_eq!(
                    mouse_action(
                        &app,
                        area,
                        click(list.x + 1, list.y + (index - app.action_offset) as u16)
                    ),
                    Some(MouseAction::SelectAction(index))
                );
            }
            app.actions_open = false;
        }
    }

    #[test]
    fn filter_field_apply_and_clear_have_pointer_targets() {
        let mut app = crate::tests::app();
        let area = Rect::new(0, 0, 34, 12);
        let status = body_layout(area)[3];
        assert_eq!(
            mouse_action(&app, area, click(status.x, status.y)),
            Some(MouseAction::Filter)
        );
        app.search = Some("phone".into());
        let (apply, clear) = search_buttons(status);
        assert_eq!(
            mouse_action(&app, area, click(apply.x, apply.y)),
            Some(MouseAction::ApplyFilter)
        );
        assert_eq!(
            mouse_action(&app, area, click(clear.x, clear.y)),
            Some(MouseAction::ClearFilter)
        );
    }

    #[test]
    fn action_scrollbar_positions_do_not_activate_commands_and_tiny_exit_is_clickable() {
        let mut app = crate::tests::app();
        app.actions_open = true;
        let area = Rect::new(0, 0, 34, 12);
        let count = available_actions(&app).len();
        let (_, list, _) = action_menu_layout(area, count);
        assert_eq!(
            mouse_action(&app, area, click(list.right() - 1, list.y)),
            Some(MouseAction::ScrollActions(0))
        );
        assert_eq!(
            mouse_action(&app, area, click(list.right() - 1, list.bottom() - 1)),
            Some(MouseAction::ScrollActions(count - 1))
        );
        app.actions_open = false;
        let tiny = Rect::new(0, 0, 20, 5);
        let (quit, cancel) = small_exit_buttons(tiny).unwrap();
        assert_eq!(
            mouse_action(&app, tiny, click(quit.x, quit.y)),
            Some(MouseAction::Quit)
        );
        app.quit_confirmation = true;
        assert_eq!(
            mouse_action(&app, tiny, click(quit.x, quit.y)),
            Some(MouseAction::ConfirmQuit)
        );
        assert_eq!(
            mouse_action(&app, tiny, click(cancel.x, cancel.y)),
            Some(MouseAction::CancelQuit)
        );
    }

    #[test]
    fn menu_clicks_follow_drawn_rows_while_keyboard_scroll_is_pending() {
        let mut app = crate::tests::app();
        app.actions_open = true;
        let area = Rect::new(0, 0, 34, 12);
        update_action_viewport(&mut app, area);
        let actions = available_actions(&app);
        let help = actions
            .iter()
            .position(|item| item.action == MouseAction::Help)
            .unwrap();
        app.action_index = help;
        update_action_viewport(&mut app, area);
        let (_, list, _) = action_menu_layout(area, actions.len());
        assert!(help - app.action_offset < usize::from(list.height));
        let row = list.y + (help - app.action_offset) as u16;
        app.action_index = actions.len() - 1; // Keyboard selection awaits a draw.
        app.mouse(click(list.x + 1, row), area).unwrap();
        assert!(app.help);
        assert!(!app.quit_confirmation);
    }

    #[test]
    fn menu_scrollbar_drag_never_sends_a_command_and_help_fits_the_minimum_size() {
        let mut app = crate::tests::app();
        app.actions_open = true;
        let area = Rect::new(0, 0, 34, 12);
        let count = available_actions(&app).len();
        let (_, list, _) = action_menu_layout(area, count);
        app.mouse(click(list.right() - 1, list.bottom() - 1), area)
            .unwrap();
        assert_eq!(app.action_index, count - 1);
        let mut event = click(list.right() - 1, list.y);
        event.kind = MouseEventKind::Drag(MouseButton::Left);
        app.mouse(event, area).unwrap();
        assert_eq!(app.action_index, 0);
        assert!(app.actions_open && !app.quit_confirmation);
        app.actions_open = false;
        app.help = true;
        let mut terminal = Terminal::new(TestBackend::new(34, 12)).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let text = (0..12)
            .map(|row| row_text(terminal.backend().buffer(), area, row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Any key / click: close"));
        assert!(text.contains("F4: menu"));
    }

    #[test]
    fn spare_row_and_borders_do_not_select_an_item() {
        let inner = Rect::new(3, 4, 20, 5);
        assert_eq!(list_hit(inner, 20, 25, 0, 4, 4), Some(19));
        assert_eq!(list_hit(inner, 20, 25, 0, 4, 6), Some(20));
        assert_eq!(list_hit(inner, 20, 25, 0, 4, 8), None);
        assert_eq!(list_hit(inner, 20, 25, 0, 2, 4), None);
        assert_eq!(list_hit(inner, 20, 25, 0, 23, 4), None);
        assert_eq!(list_hit(inner, 20, 0, 0, 4, 4), None);
    }

    #[test]
    fn device_clicks_match_rendered_scrolled_rows_at_every_layout_size() {
        let mut app = crate::tests::app();
        app.devices = (0..25)
            .map(|index| Device {
                id: format!("device-{index}"),
                name: format!("Device #{index}"),
                platform: "android".into(),
                available: true,
                favorite: false,
            })
            .collect();
        app.device_index = 20;
        for (width, height) in [(34, 12), (80, 24), (120, 40)] {
            let area = Rect::new(0, 0, width, height);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            let inner = panel(String::new()).inner(picker_layout(body_layout(area)[2])[0]);
            let offset = list_offset(20, 25, inner.height, 0);
            for row in 0..usize::from(inner.height / 2) {
                let index = offset + row;
                let y = inner.y + row as u16 * 2;
                assert!(
                    row_text(terminal.backend().buffer(), inner, y)
                        .contains(&format!("Device #{index}"))
                );
                assert_eq!(
                    mouse_action(&app, area, click(inner.x + 6, y)),
                    Some(MouseAction::SelectDevice(index))
                );
                assert_eq!(terminal.backend().buffer()[(inner.x + 3, y)].symbol(), "☆");
                assert_eq!(
                    mouse_action(&app, area, click(inner.x + 3, y)),
                    Some(MouseAction::ToggleFavorite(index))
                );
                assert_eq!(
                    mouse_action(&app, area, click(inner.x + 3, y + 1)),
                    Some(MouseAction::SelectDevice(index))
                );
            }
            if inner.height % 2 == 1 {
                assert_eq!(
                    mouse_action(&app, area, click(inner.x + 6, inner.bottom() - 1)),
                    None
                );
            }
        }
    }

    #[test]
    fn clicking_upper_visible_item_keeps_the_viewport_and_row_label() {
        let mut app = crate::tests::app();
        app.devices = (0..25)
            .map(|index| Device {
                id: format!("device-{index}"),
                name: format!("Device #{index}"),
                platform: "android".into(),
                available: true,
                favorite: false,
            })
            .collect();
        for (width, height) in [(34, 12), (80, 24), (120, 40)] {
            let area = Rect::new(0, 0, width, height);
            app.device_index = 20;
            app.device_list_offset = 0;
            update_picker_viewport(&mut app, area);
            let offset = app.device_list_offset;
            let inner = panel(String::new()).inner(picker_layout(body_layout(area)[2])[0]);
            assert_eq!(
                mouse_action(&app, area, click(inner.x + 6, inner.y)),
                Some(MouseAction::SelectDevice(offset))
            );
            app.device_index = offset;
            update_picker_viewport(&mut app, area);
            assert_eq!(app.device_list_offset, offset);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            assert!(
                row_text(terminal.backend().buffer(), inner, inner.y)
                    .contains(&format!("Device #{offset}"))
            );
            assert_eq!(
                mouse_action(&app, area, click(inner.x + 3, inner.y)),
                Some(MouseAction::ToggleFavorite(offset))
            );
        }
    }

    #[test]
    fn viewport_clamps_after_list_changes_and_reveals_offscreen_selection() {
        assert_eq!(list_offset(20, 25, 10, 16), 16);
        assert_eq!(list_offset(16, 25, 10, 16), 16);
        assert_eq!(list_offset(15, 25, 10, 16), 15);
        assert_eq!(list_offset(21, 25, 10, 16), 17);
        assert_eq!(list_offset(3, 4, 10, 16), 0);
        assert_eq!(list_offset(20, 25, 20, 16), 15);
        assert_eq!(list_offset(0, 0, 10, 16), 0);
    }

    #[test]
    fn configuration_rows_and_toolbar_buttons_use_rendered_hit_regions() {
        let mut app = crate::tests::app();
        app.page = Page::Configs;
        app.selected_device = app
            .devices
            .iter()
            .find(|d| d.id == "android-device")
            .cloned();
        for (width, height) in [(34, 12), (80, 24), (120, 40)] {
            let area = Rect::new(0, 0, width, height);
            let inner = panel(String::new()).inner(picker_layout(body_layout(area)[2])[0]);
            assert_eq!(
                mouse_action(&app, area, click(inner.x + 3, inner.y)),
                Some(MouseAction::SelectConfig(0))
            );
            for control in controls(&app, body_layout(area)[0]) {
                assert_eq!(
                    mouse_action(&app, area, click(control.rect.x, control.rect.y)),
                    Some(control.action)
                );
            }
            assert!(
                controls(&app, body_layout(area)[0])
                    .iter()
                    .any(|control| control.action == MouseAction::Quit)
            );
        }
    }

    #[test]
    fn quit_confirmation_intercepts_background_clicks() {
        let mut app = crate::tests::app();
        app.quit_confirmation = true;
        for (width, height) in [(34, 12), (80, 24), (120, 40)] {
            let area = Rect::new(0, 0, width, height);
            let (_, quit, cancel) = quit_modal(area);
            assert_eq!(
                mouse_action(&app, area, click(quit.x, quit.y)),
                Some(MouseAction::ConfirmQuit)
            );
            assert_eq!(
                mouse_action(&app, area, click(cancel.x, cancel.y)),
                Some(MouseAction::CancelQuit)
            );
            assert_eq!(mouse_action(&app, area, click(0, 0)), None);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            assert_eq!(
                row_text(terminal.backend().buffer(), quit, quit.y),
                "[ Quit ]"
            );
            assert_eq!(
                row_text(terminal.backend().buffer(), cancel, cancel.y),
                "[ Cancel ]"
            );
        }
    }

    #[test]
    fn scrollbar_reaches_both_ends_and_preserves_content_column() {
        for (width, height) in [(34, 12), (80, 24), (120, 40)] {
            let area = Rect::new(0, 0, width, height);
            let body = body_layout(area)[2];
            let track = console_scrollbar_area(area);
            let mut parser = vt100::Parser::new(track.height, width - 3, 1000);
            for _ in 0..100 {
                parser.process(b"history\r\n");
            }
            parser.screen_mut().set_scrollback(usize::MAX);
            let scrollback_len = parser.screen().scrollback();
            parser.screen_mut().set_scrollback(0);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    console(
                        frame,
                        parser.screen(),
                        scrollback_len,
                        None,
                        true,
                        body,
                        " Test ",
                    )
                })
                .unwrap();
            assert_eq!(
                terminal.backend().buffer()[(track.x, track.bottom() - 1)].symbol(),
                "█"
            );
            parser.screen_mut().set_scrollback(scrollback_len);
            terminal
                .draw(|frame| {
                    console(
                        frame,
                        parser.screen(),
                        scrollback_len,
                        None,
                        true,
                        body,
                        " Test ",
                    )
                })
                .unwrap();
            assert_eq!(
                terminal.backend().buffer()[(track.x, track.y)].symbol(),
                "█"
            );
            assert_eq!(track.width, 1);
            assert_eq!(track.x, width - 2);
        }
    }

    #[test]
    fn saved_console_shrinks_to_latest_rows_and_keeps_history_for_expansion() {
        let mut parser = vt100::Parser::new(20, 77, 1000);
        for index in 0..100 {
            parser.process(format!("tail{index}\r\n").as_bytes());
        }
        parser.screen_mut().set_scrollback(usize::MAX);
        let scrollback_len = parser.screen().scrollback();
        parser.screen_mut().set_scrollback(0);
        let original = parser.screen().clone();
        let mut snapshot = original.clone();
        let area = Rect::new(0, 0, 80, 17);
        let body = body_layout(area)[2];
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        let inner = panel(String::new()).inner(body);
        assert_eq!(inner.height, 8);
        terminal
            .draw(|frame| {
                console(
                    frame,
                    &snapshot,
                    scrollback_len,
                    Some(0),
                    true,
                    body,
                    " Saved ",
                )
            })
            .unwrap();
        let visible = (inner.y..inner.bottom())
            .map(|y| row_text(terminal.backend().buffer(), inner, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(visible.contains("tail99"));
        assert!(!visible.contains("tail81"));
        assert_eq!(
            terminal.backend().buffer()[(inner.right() - 1, inner.bottom() - 1)].symbol(),
            "█"
        );

        let virtual_offset = 30;
        let hidden_rows = 20 - usize::from(inner.height);
        snapshot.set_scrollback(virtual_offset - hidden_rows);
        terminal
            .draw(|frame| {
                console(
                    frame,
                    &snapshot,
                    scrollback_len,
                    Some(virtual_offset),
                    true,
                    body,
                    " Saved ",
                )
            })
            .unwrap();
        let visible = (inner.y..inner.bottom())
            .map(|y| row_text(terminal.backend().buffer(), inner, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(visible.contains("tail69"));
        assert!(!visible.contains("tail99"));

        // Restore follow mode and grow: the original screen's rows are retained.
        snapshot.set_scrollback(0);
        let expanded = Rect::new(0, 0, 80, 29);
        let expanded_body = body_layout(expanded)[2];
        let expanded_inner = panel(String::new()).inner(expanded_body);
        let mut terminal =
            Terminal::new(TestBackend::new(expanded.width, expanded.height)).unwrap();
        terminal
            .draw(|frame| {
                console(
                    frame,
                    &snapshot,
                    scrollback_len,
                    Some(0),
                    true,
                    expanded_body,
                    " Saved ",
                )
            })
            .unwrap();
        let visible = (expanded_inner.y..expanded_inner.bottom())
            .map(|y| row_text(terminal.backend().buffer(), expanded_inner, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(visible.contains("tail81"));
        assert!(visible.contains("tail99"));
        assert_eq!(snapshot.size(), original.size());
        assert_eq!(snapshot.contents(), original.contents());
    }
}
