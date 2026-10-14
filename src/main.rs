mod terminal_input;
mod ui;

use anyhow::{Context, Result, bail};
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{DisableLineWrap, EnableLineWrap},
};
use flutter_run_tui::{
    config::{LaunchConfig, LaunchFile},
    device::{Device, Discovery, DiscoveryEvent, merge_devices},
    session::Session,
    state::{self, State},
    tool::{FlutterTool, has_ownit_helpers},
};
use std::{
    io::IsTerminal,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Persistent Flutter launcher using .vscode/launch.json"
)]
struct Options {
    /// Flutter workspace (defaults to current directory; searches parent directories).
    #[arg(value_name = "PROJECT")]
    project: Option<PathBuf>,
    /// Explicit launch.json path (resolved relative to PROJECT).
    #[arg(long)]
    launch: Option<PathBuf>,
    /// Flutter executable or wrapper. Automatically uses the project's FVM SDK otherwise.
    #[arg(long, conflicts_with = "fvm")]
    flutter: Option<PathBuf>,
    /// Use FVM even without a project FVM configuration.
    #[arg(long)]
    fvm: bool,
    /// Device refresh interval in seconds (minimum 2).
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u64).range(2..=3600))]
    refresh_interval: u64,
    /// Override the favorites/settings file.
    #[arg(long)]
    state: Option<PathBuf>,
    /// Validate launch.json and SDK resolution without running Flutter or entering the TUI.
    #[arg(long)]
    check: bool,
    /// Disable automatic use of OWNIT's secrets guard and Designed-for-iPad helper.
    #[arg(long)]
    no_project_helpers: bool,
    /// Leave mouse input to your terminal (for text selection/copying).
    #[arg(long)]
    no_mouse: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Devices,
    Configs,
    Console,
    LastLog,
}

struct RestoreTerminal;

#[derive(Clone, Copy)]
struct ScrollDrag {
    start_row: u16,
    start_position: usize,
    max_position: usize,
    travel: u16,
}

impl Drop for RestoreTerminal {
    fn drop(&mut self) {
        let _ = execute!(
            std::io::stdout(),
            DisableBracketedPaste,
            DisableMouseCapture,
            EnableLineWrap
        );
        // A vanished terminal cannot receive restoration escapes. Never print
        // an error to that same closed stderr (restore() would panic there).
        let _ = ratatui::try_restore();
    }
}

pub(crate) struct App {
    launch: LaunchFile,
    tool: FlutterTool,
    discovery: Discovery,
    saved: State,
    state_path: PathBuf,
    persistence_enabled: bool,
    discovered: Vec<Device>,
    devices: Vec<Device>,
    device_index: usize,
    config_index: usize,
    device_list_offset: usize,
    config_list_offset: usize,
    selected_device: Option<Device>,
    page: Page,
    search: Option<String>,
    searching: bool,
    busy: bool,
    refreshed: Option<Instant>,
    status: String,
    ownit: bool,
    session: Option<Session>,
    last_log: Option<vt100::Parser>,
    last_log_scrollback: usize,
    last_log_offset: usize,
    last_log_view_rows: u16,
    active_config: String,
    help: bool,
    quit_confirmation: bool,
    dragging_scrollbar: Option<ScrollDrag>,
}

fn paths(options: &Options) -> Result<(PathBuf, PathBuf)> {
    let start = std::fs::canonicalize(options.project.clone().unwrap_or(std::env::current_dir()?))
        .context("Project directory not found")?;
    if !start.is_dir() {
        bail!("PROJECT must be a directory");
    }
    if let Some(path) = &options.launch {
        let launch = if path.is_absolute() {
            path.clone()
        } else {
            start.join(path)
        };
        return Ok((start, launch));
    }
    for workspace in start.ancestors() {
        let launch = workspace.join(".vscode/launch.json");
        if launch.is_file() {
            return Ok((workspace.to_path_buf(), launch));
        }
    }
    bail!(
        "No .vscode/launch.json found in {} or its parents. Pass PROJECT or --launch <path>.",
        start.display()
    )
}

fn main() -> Result<()> {
    let options = Options::parse();
    let (workspace, path) = paths(&options)?;
    let launch = LaunchFile::load(&path, &workspace)?;
    if launch.configurations.is_empty() && !options.check {
        bail!(
            "No supported launch configurations in {}. {}",
            path.display(),
            launch.warnings.join("\n")
        );
    }
    let tool = FlutterTool::resolve(&workspace, options.flutter.as_deref(), options.fvm)?;
    if options.check {
        println!(
            "Project: {}\nLaunch: {}\nFlutter: {}{}\n{} launch configurations",
            workspace.display(),
            path.display(),
            tool.executable.display(),
            if tool.prefix.is_empty() {
                ""
            } else {
                " flutter"
            },
            launch.configurations.len()
        );
        for config in &launch.configurations {
            println!(
                "  {}  [{}]{}",
                config.name,
                config.cwd.display(),
                config
                    .device_id
                    .as_ref()
                    .map(|id| format!(" · {id}"))
                    .unwrap_or_default()
            );
            if let Err(error) =
                config.flutter_args(config.device_id.as_deref().unwrap_or("selected-device"))
            {
                println!("    Cannot launch: {error:#}");
            }
        }
        for warning in &launch.warnings {
            println!("Warning: {warning}");
        }
        return Ok(());
    }
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("An interactive terminal is required. Use --check to validate configuration.");
    }
    let state_path = options.state.unwrap_or(state::default_path()?);
    let (saved, persistence_enabled, status) = match state::load(&state_path) {
        Ok(saved) => (
            saved,
            true,
            "Select a device, then a launch configuration.".into(),
        ),
        Err(error) => (
            State::default(),
            false,
            format!("{error:#}. Favorites will not be saved until this file is repaired."),
        ),
    };
    let ownit = !options.no_project_helpers && has_ownit_helpers(&workspace);
    let discovery = Discovery::start(tool.clone(), Duration::from_secs(options.refresh_interval));
    let mut app = App {
        launch,
        tool,
        discovery,
        saved,
        state_path,
        persistence_enabled,
        discovered: vec![],
        devices: vec![],
        device_index: 0,
        config_index: 0,
        device_list_offset: 0,
        config_list_offset: 0,
        selected_device: None,
        page: Page::Devices,
        search: None,
        searching: false,
        busy: false,
        refreshed: None,
        status,
        ownit,
        session: None,
        last_log: None,
        last_log_scrollback: 0,
        last_log_offset: 0,
        last_log_view_rows: 0,
        active_config: String::new(),
        help: false,
        quit_confirmation: false,
        dragging_scrollbar: None,
    };
    app.merge();
    let quit = Arc::new(AtomicBool::new(false));
    let signal_quit = quit.clone();
    ctrlc::set_handler(move || signal_quit.store(true, Ordering::Relaxed))?;
    let restore_terminal = RestoreTerminal;
    let mut terminal = ratatui::try_init()?;
    // ratatui's panic hook restores raw mode and the alternate screen.
    let result = (|| -> Result<()> {
        execute!(std::io::stdout(), EnableBracketedPaste, DisableLineWrap)?;
        if !options.no_mouse {
            execute!(std::io::stdout(), EnableMouseCapture)?;
            // Track button clicks, wheel and button drags, not every hover.
            execute!(
                std::io::stdout(),
                crossterm::style::Print("\x1b[?1003l\x1b[?1002h")
            )?;
        }
        let frame_interval = Duration::from_millis(33);
        let mut redraw = true;
        let mut last_draw = Instant::now();
        let mut previous_size = terminal.size()?;
        let mut previous_animations = None;
        while !quit.load(Ordering::Relaxed) {
            let frame_started = Instant::now();
            if !terminal_input::connected()? {
                break;
            }
            redraw |= app.tick()?;
            let area = terminal.size()?;
            redraw |= area != previous_size;
            previous_size = area;
            if app.page == Page::Console
                && let Some(session) = &mut app.session
            {
                session.resize(
                    area.height.saturating_sub(9).max(1),
                    area.width.saturating_sub(3).max(1),
                )?;
            }
            let animations = app
                .session
                .as_ref()
                .map(|session| session.animations.state());
            redraw |= animations != previous_animations;
            previous_animations = animations;
            if redraw || last_draw.elapsed() >= Duration::from_secs(1) {
                app.prepare_last_log(area.height.saturating_sub(9).max(1));
                ui::update_picker_viewport(
                    &mut app,
                    ratatui::layout::Rect::new(0, 0, area.width, area.height),
                );
                terminal.draw(|frame| ui::draw(frame, &app))?;
                last_draw = Instant::now();
                redraw = false;
            }
            let mut exit_requested = false;
            let input_started = Instant::now();
            for _ in 0..128 {
                if !terminal_input::connected()? {
                    exit_requested = true;
                    break;
                }
                if !event::poll(Duration::from_millis(1))? {
                    break;
                }
                match event::read()? {
                    Event::Key(key) if key.kind != KeyEventKind::Release => {
                        redraw = true;
                        if app.key(key, terminal.size()?.width, terminal.size()?.height)? {
                            exit_requested = true;
                            break;
                        }
                    }
                    Event::Paste(text) if app.page == Page::Console => {
                        redraw = true;
                        if !app.quit_confirmation
                            && !app.help
                            && let Some(session) = &mut app.session
                            && let Err(error) = session.send(text.as_bytes())
                        {
                            app.status = error.to_string();
                        }
                    }
                    Event::Mouse(event)
                        if !options.no_mouse && event.kind != MouseEventKind::Moved =>
                    {
                        redraw = true;
                        let area = terminal.size()?;
                        if app.mouse(
                            event,
                            ratatui::layout::Rect::new(0, 0, area.width, area.height),
                        )? {
                            exit_requested = true;
                            break;
                        }
                    }
                    Event::Resize(..) => redraw = true,
                    _ => {}
                }
                if input_started.elapsed() >= Duration::from_millis(5) {
                    break;
                }
            }
            if exit_requested {
                break;
            }
            // Ready input must never bypass the frame limit (e.g. mouse floods).
            if let Some(remaining) = frame_interval.checked_sub(frame_started.elapsed()) {
                std::thread::sleep(remaining);
            }
        }
        Ok(())
    })();
    // Stop owned children before restoring the user's terminal, even on errors.
    app.session.take();
    if !terminal_input::connected().unwrap_or(false) {
        let _ = terminal_input::detach_output();
    }
    drop(restore_terminal);
    result
}

impl App {
    fn pinned(&self) -> Vec<String> {
        let mut ids: Vec<_> = self
            .launch
            .configurations
            .iter()
            .filter_map(|c| c.device_id.clone())
            .collect();
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        if self.ownit {
            ids.push("mac-designed-for-ipad".into());
        }
        if self.launch.configurations.iter().any(|c| !c.needs_device()) {
            ids.push("flutter-tester".into());
        }
        ids
    }

    fn merge(&mut self) {
        let config_name = self
            .visible_configs()
            .get(self.config_index)
            .map(|c| c.name.clone());
        let selected = self
            .selected_device
            .as_ref()
            .map(|d| d.id.clone())
            .or_else(|| {
                self.visible_devices()
                    .get(self.device_index)
                    .map(|d| d.id.clone())
            })
            .or(self.saved.last_device.clone());
        self.devices = merge_devices(&self.discovered, &self.saved.favorites, &self.pinned());
        for device in &mut self.devices {
            if device.id == "mac-designed-for-ipad" {
                device.name = "My Mac (Designed for iPad)".into();
                device.platform = "ios".into();
            }
            if device.id == "flutter-tester" {
                device.name = "Flutter tests (no device)".into();
                device.platform = "tester".into();
            }
        }
        if let Some(selected) = &self.selected_device
            && let Some(updated) = self.devices.iter().find(|device| device.id == selected.id)
        {
            self.selected_device = Some(updated.clone());
        }
        let visible = self.visible_devices();
        self.device_index = selected
            .and_then(|id| visible.iter().position(|d| d.id == id))
            .unwrap_or(self.device_index.min(visible.len().saturating_sub(1)));
        let configs = self.visible_configs();
        self.config_index = config_name
            .and_then(|name| configs.iter().position(|c| c.name == name))
            .unwrap_or(self.config_index.min(configs.len().saturating_sub(1)));
    }

    fn visible_devices(&self) -> Vec<&Device> {
        let query = self.search.as_deref().unwrap_or("").to_lowercase();
        self.devices
            .iter()
            .filter(|d| {
                format!("{} {} {}", d.name, d.id, d.platform)
                    .to_lowercase()
                    .contains(&query)
            })
            .collect()
    }

    fn compatible(&self, config: &LaunchConfig, device: &Device) -> bool {
        if device.id == "flutter-tester" {
            return !config.needs_device();
        }
        if config.device_id.as_ref().is_some_and(|id| id != &device.id) {
            return false;
        }
        if self.ownit {
            let flags: Vec<_> = config.tool_args.iter().chain(&config.args).collect();
            let solana = flags.iter().enumerate().any(|(i, flag)| {
                (flag.as_str() == "--flavor"
                    && flags.get(i + 1).is_some_and(|f| f.contains("Solana")))
                    || (flag.starts_with("--flavor=") && flag.contains("Solana"))
            });
            if solana && !device.platform.starts_with("android") {
                return false;
            }
            if device.id == "macos" && config.device_id.as_deref() != Some("macos") {
                return false;
            }
        }
        true
    }

    fn visible_configs(&self) -> Vec<&LaunchConfig> {
        let query = self.search.as_deref().unwrap_or("").to_lowercase();
        self.launch
            .configurations
            .iter()
            .filter(|c| {
                self.selected_device
                    .as_ref()
                    .is_some_and(|d| self.compatible(c, d))
                    && c.name.to_lowercase().contains(&query)
            })
            .collect()
    }

    fn save(&mut self) {
        if self.persistence_enabled
            && let Err(error) = state::save(&self.state_path, &self.saved)
        {
            self.status = format!("Could not save favorites: {error:#}");
        }
    }

    fn tick(&mut self) -> Result<bool> {
        let mut changed = false;
        while let Ok(event) = self.discovery.events.try_recv() {
            changed = true;
            match event {
                DiscoveryEvent::Started => self.busy = true,
                DiscoveryEvent::Finished(result) => {
                    self.busy = false;
                    match result {
                        Ok(devices) => {
                            self.discovered = devices;
                            self.refreshed = Some(Instant::now());
                            // Update labels of saved favorites without dropping absent favorites.
                            for favorite in &mut self.saved.favorites {
                                if let Some(device) =
                                    self.discovered.iter().find(|d| d.id == favorite.id)
                                {
                                    *favorite = device.saved();
                                }
                            }
                            self.merge();
                        }
                        Err(error) => {
                            self.status =
                                format!("Device discovery failed: {error}. Press r to retry.")
                        }
                    }
                }
            }
        }
        if let Some(session) = &mut self.session {
            let revision = session.visual_revision;
            let status = session.tick()?;
            changed |= revision != session.visual_revision;
            if let Some(status) = status {
                changed = true;
                self.last_log = Some(session.take_console());
                self.last_log_scrollback = session.scrollback_len;
                self.last_log_offset = 0;
                self.status = format!(
                    "{} ended ({status}). Press l to view its console, or choose another device.",
                    self.active_config
                );
                self.session.take();
                self.page = Page::Devices;
                self.selected_device = None;
                self.search = None;
                self.merge();
            }
        }
        Ok(changed)
    }

    fn back(&mut self) {
        self.page = Page::Devices;
        self.search = None;
        self.searching = false;
        self.selected_device = None;
        self.help = false;
    }

    fn key(&mut self, key: KeyEvent, cols: u16, rows: u16) -> Result<bool> {
        if self.quit_confirmation {
            match key.code {
                KeyCode::Char('y' | 'Y') => return Ok(true),
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('n' | 'N') => {
                    self.quit_confirmation = false
                }
                _ => {}
            }
            return Ok(false);
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
            self.quit_confirmation = true;
            return Ok(false);
        }
        if key.code == KeyCode::F(1) {
            self.help = !self.help;
            return Ok(false);
        }
        if self.help {
            self.help = false;
            return Ok(false);
        }
        if self.page != Page::Console
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == KeyCode::Char('c')
        {
            self.quit_confirmation = true;
            return Ok(false);
        }
        if self.page == Page::Console {
            if let Some(session) = &mut self.session {
                let result = match key.code {
                    KeyCode::F(2) => session.animations.toggle(),
                    KeyCode::Esc | KeyCode::Char('q')
                        if !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        session.request_stop()
                    }
                    KeyCode::PageUp => {
                        session.scroll(rows.saturating_sub(9).max(1) as isize);
                        Ok(())
                    }
                    KeyCode::PageDown => {
                        session.scroll(-(rows.saturating_sub(9).max(1) as isize));
                        Ok(())
                    }
                    KeyCode::End => {
                        session.parser.screen_mut().set_scrollback(0);
                        Ok(())
                    }
                    _ => key_bytes(key)
                        .map(|bytes| session.send(&bytes))
                        .unwrap_or(Ok(())),
                };
                if let Err(error) = result {
                    self.status = format!("Console input: {error:#}");
                }
            }
            return Ok(false);
        }
        if self.page == Page::LastLog {
            let page = rows.saturating_sub(9).max(1) as usize;
            match key.code {
                KeyCode::PageUp => self.last_log_offset = self.last_log_offset.saturating_add(page),
                KeyCode::Up | KeyCode::Char('k') => {
                    self.last_log_offset = self.last_log_offset.saturating_add(1)
                }
                KeyCode::PageDown => {
                    self.last_log_offset = self.last_log_offset.saturating_sub(page)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.last_log_offset = self.last_log_offset.saturating_sub(1)
                }
                KeyCode::End => self.last_log_offset = 0,
                _ => self.back(),
            }
            self.prepare_last_log(page as u16);
            return Ok(false);
        }
        if self.searching {
            let query = self.search.get_or_insert_default();
            match key.code {
                KeyCode::Esc => {
                    self.search = None;
                    self.searching = false;
                }
                KeyCode::Enter => self.searching = false,
                KeyCode::Backspace => {
                    query.pop();
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    query.push(c);
                }
                _ => {}
            }
            self.device_index = 0;
            self.config_index = 0;
            return Ok(false);
        }
        let count = if self.page == Page::Devices {
            self.visible_devices().len()
        } else {
            self.visible_configs().len()
        };
        let index = if self.page == Page::Devices {
            &mut self.device_index
        } else {
            &mut self.config_index
        };
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => *index = index.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                *index = (*index + 1).min(count.saturating_sub(1))
            }
            KeyCode::Home => *index = 0,
            KeyCode::End => *index = count.saturating_sub(1),
            KeyCode::Char('/') => {
                self.search = Some(String::new());
                self.searching = true;
            }
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('r') => self.discovery.refresh(),
            KeyCode::Char('c') => match LaunchFile::load(&self.launch.path, &self.launch.workspace)
            {
                Ok(launch) => {
                    self.launch = launch;
                    self.config_index = 0;
                    self.merge();
                    self.status = "Launch configurations reloaded.".into();
                }
                Err(error) => self.status = format!("Reload failed: {error:#}"),
            },
            KeyCode::Char('l') if self.last_log.is_some() => self.page = Page::LastLog,
            KeyCode::Char('f') if self.page == Page::Devices => {
                if let Some(device) = self
                    .visible_devices()
                    .get(self.device_index)
                    .map(|d| (*d).clone())
                {
                    let favorite = self.saved.toggle_favorite(device.saved());
                    self.status = format!(
                        "{} {} favorites.",
                        device.name,
                        if favorite { "added to" } else { "removed from" }
                    );
                    self.merge();
                    self.save();
                }
            }
            KeyCode::Esc if self.search.is_some() => {
                self.search = None;
                self.device_index = 0;
                self.config_index = 0;
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                if self.page == Page::Configs {
                    self.back();
                } else {
                    self.quit_confirmation = true;
                }
            }
            KeyCode::Enter if self.page == Page::Devices => {
                if let Some(device) = self
                    .visible_devices()
                    .get(self.device_index)
                    .map(|d| (*d).clone())
                {
                    self.selected_device = Some(device);
                    self.search = None;
                    self.page = Page::Configs;
                    let configs = self.visible_configs();
                    self.config_index = self
                        .saved
                        .last_config
                        .as_ref()
                        .and_then(|name| configs.iter().position(|c| &c.name == name))
                        .unwrap_or(0);
                }
            }
            KeyCode::Enter if self.page == Page::Configs => {
                if let (Some(config), Some(device)) = (
                    self.visible_configs()
                        .get(self.config_index)
                        .map(|c| (*c).clone()),
                    self.selected_device.clone(),
                ) {
                    match self
                        .tool
                        .plan(&config, &device.id, self.ownit)
                        .and_then(|plan| {
                            Session::start(&plan, rows.saturating_sub(9), cols.saturating_sub(3))
                        }) {
                        Ok(mut session) => {
                            if config.is_test() {
                                session.animations =
                                    flutter_run_tui::animation::AnimationControl::new(false);
                            }
                            self.saved.last_device = Some(device.id);
                            self.saved.last_config = Some(config.name.clone());
                            self.save();
                            self.active_config = config.name;
                            // Only the latest run is retained. Release the old
                            // history when a new console successfully starts.
                            self.last_log.take();
                            self.last_log_scrollback = 0;
                            self.last_log_offset = 0;
                            self.session = Some(session);
                            self.status = "Flutter shortcuts go directly to the session. F2 slows animations; Esc stops; Ctrl+q asks to quit.".into();
                            self.page = Page::Console;
                            self.search = None;
                        }
                        Err(error) => self.status = format!("Cannot start: {error:#}"),
                    }
                }
            }
            _ => {}
        }
        Ok(false)
    }
}

impl App {
    fn mouse(&mut self, event: MouseEvent, area: ratatui::layout::Rect) -> Result<bool> {
        self.prepare_last_log(area.height.saturating_sub(9).max(1));
        ui::update_picker_viewport(self, area);
        if matches!(event.kind, MouseEventKind::Up(MouseButton::Left)) {
            self.dragging_scrollbar = None;
            return Ok(false);
        }
        if !self.quit_confirmation && !self.help {
            let wheel = match event.kind {
                MouseEventKind::ScrollUp => Some(-1),
                MouseEventKind::ScrollDown => Some(1),
                _ => None,
            };
            if let Some(direction) = wheel {
                match self.page {
                    Page::Console => {
                        if let Some(session) = &mut self.session {
                            session.scroll(-direction * 3);
                        }
                    }
                    Page::LastLog => {
                        self.last_log_offset =
                            self.last_log_offset.saturating_add_signed(-direction * 3);
                        self.prepare_last_log(area.height.saturating_sub(9).max(1));
                    }
                    Page::Devices | Page::Configs => {
                        self.searching = false;
                        return self.key(
                            KeyEvent::new(
                                if direction < 0 {
                                    KeyCode::Up
                                } else {
                                    KeyCode::Down
                                },
                                KeyModifiers::NONE,
                            ),
                            area.width,
                            area.height,
                        );
                    }
                }
                return Ok(false);
            }
            if let Some(drag) = self.dragging_scrollbar
                && matches!(event.kind, MouseEventKind::Drag(MouseButton::Left))
            {
                let delta = i32::from(event.row) - i32::from(drag.start_row);
                let position = (drag.start_position as f64
                    + f64::from(delta) * drag.max_position as f64 / f64::from(drag.travel.max(1)))
                .round()
                .clamp(0.0, drag.max_position as f64) as usize;
                self.set_console_position(position);
                return Ok(false);
            }
        }
        let Some(action) = ui::mouse_action(self, area, event) else {
            return Ok(false);
        };
        use ui::MouseAction;
        let key = match action {
            MouseAction::SelectDevice(index) => {
                self.searching = false;
                self.device_index = index;
                return Ok(false);
            }
            MouseAction::SelectConfig(index) => {
                self.searching = false;
                self.config_index = index;
                return Ok(false);
            }
            MouseAction::ToggleFavorite(index) => {
                self.searching = false;
                self.device_index = index;
                KeyCode::Char('f')
            }
            MouseAction::NextOrRun => {
                self.searching = false;
                KeyCode::Enter
            }
            MouseAction::Back => {
                self.search = None;
                self.searching = false;
                KeyCode::Esc
            }
            MouseAction::Refresh => {
                self.searching = false;
                KeyCode::Char('r')
            }
            MouseAction::ReloadConfigs => {
                self.searching = false;
                KeyCode::Char('c')
            }
            MouseAction::LastLog => {
                self.searching = false;
                KeyCode::Char('l')
            }
            MouseAction::Quit => {
                self.quit_confirmation = true;
                return Ok(false);
            }
            MouseAction::ConfirmQuit => return Ok(true),
            MouseAction::CancelQuit => {
                self.quit_confirmation = false;
                return Ok(false);
            }
            MouseAction::Help | MouseAction::DismissHelp => KeyCode::F(1),
            MouseAction::ToggleAnimations => KeyCode::F(2),
            MouseAction::ConsoleScroll(_) => {
                self.begin_scrollbar_drag(event.row, ui::console_scrollbar_area(area));
                return Ok(false);
            }
        };
        self.key(
            KeyEvent::new(key, KeyModifiers::NONE),
            area.width,
            area.height,
        )
    }

    fn begin_scrollbar_drag(&mut self, row: u16, track: ratatui::layout::Rect) {
        if track.height == 0 {
            return;
        }
        let (max, position, rows) = match self.page {
            Page::Console => match &self.session {
                Some(session) => (
                    session.scrollback_len,
                    session
                        .scrollback_len
                        .saturating_sub(session.parser.screen().scrollback()),
                    session.parser.screen().size().0 as usize,
                ),
                None => return,
            },
            Page::LastLog => match &self.last_log {
                Some(parser) => (
                    self.last_log_max_offset(),
                    self.last_log_max_offset()
                        .saturating_sub(self.last_log_offset),
                    (self.last_log_view_rows.max(1) as usize)
                        .min(parser.screen().size().0 as usize),
                ),
                None => return,
            },
            _ => return,
        };
        let total = max.saturating_add(rows).max(1);
        let height = track.height as usize;
        // Match Ratatui's thumb ratio; grabbing the thumb doesn't jump.
        let thumb = ((rows * height + total / 2) / total).clamp(1, height);
        let travel = height - thumb;
        let thumb_start = ((position * height + total / 2) / total).min(travel);
        let pointer = row.saturating_sub(track.y) as usize;
        let position = if (thumb_start..thumb_start + thumb).contains(&pointer) || travel == 0 {
            position
        } else {
            (pointer.saturating_sub(thumb / 2).min(travel) as f64 * max as f64 / travel as f64)
                .round() as usize
        };
        self.set_console_position(position);
        self.dragging_scrollbar = Some(ScrollDrag {
            start_row: row,
            start_position: position,
            max_position: max,
            travel: travel as u16,
        });
    }

    fn set_console_position(&mut self, position: usize) {
        match self.page {
            Page::Console => {
                if let Some(session) = &mut self.session {
                    session
                        .parser
                        .screen_mut()
                        .set_scrollback(session.scrollback_len.saturating_sub(position));
                }
            }
            Page::LastLog => {
                self.last_log_offset = self.last_log_max_offset().saturating_sub(position);
                self.prepare_last_log(self.last_log_view_rows);
            }
            _ => {}
        }
    }

    fn last_log_max_offset(&self) -> usize {
        self.last_log_scrollback
            + self
                .last_log
                .as_ref()
                .map(|s| {
                    s.screen()
                        .size()
                        .0
                        .saturating_sub(self.last_log_view_rows.max(1)) as usize
                })
                .unwrap_or(0)
    }

    fn prepare_last_log(&mut self, rows: u16) {
        self.last_log_view_rows = rows.max(1);
        self.last_log_offset = self.last_log_offset.min(self.last_log_max_offset());
        if let Some(parser) = &mut self.last_log {
            let screen = parser.screen_mut();
            let hidden = screen.size().0.saturating_sub(self.last_log_view_rows) as usize;
            screen.set_scrollback(self.last_log_offset.saturating_sub(hidden));
        }
    }
}

fn key_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    let mut bytes = match key.code {
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) && c.is_ascii() => {
            vec![(c.to_ascii_lowercase() as u8) & 0x1f]
        }
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        _ => return None,
    };
    if key.modifiers.contains(KeyModifiers::ALT) {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn app() -> App {
        let workspace = std::env::temp_dir();
        let tool = FlutterTool {
            executable: workspace.join("flutter-run-tui-nonexistent-test-tool"),
            prefix: vec![],
            workspace: workspace.clone(),
        };
        let launch = LaunchFile::parse(r#"{"configurations":[
            {"name":"Ordinary","deviceId":"android-device"},
            {"name":"Solana","toolArgs":["--flavor"],"args":["devSolana"],"deviceId":"android-device"},
            {"name":"Unit tests","program":"test"},
            {"name":"Integration tests","program":"integration_test/app_test.dart"}
        ]}"#, &workspace.join("launch.json"), &workspace).unwrap();
        let discovery = Discovery::start(tool.clone(), Duration::from_secs(3600));
        let mut app = App {
            launch,
            tool,
            discovery,
            saved: State::default(),
            state_path: workspace.join("unused-state.json"),
            persistence_enabled: false,
            discovered: vec![],
            devices: vec![],
            device_index: 0,
            config_index: 0,
            device_list_offset: 0,
            config_list_offset: 0,
            selected_device: None,
            page: Page::Devices,
            search: None,
            searching: false,
            busy: false,
            refreshed: None,
            status: String::new(),
            ownit: true,
            session: None,
            last_log: None,
            last_log_scrollback: 0,
            last_log_offset: 0,
            last_log_view_rows: 0,
            active_config: String::new(),
            help: false,
            quit_confirmation: false,
            dragging_scrollbar: None,
        };
        app.merge();
        app
    }

    #[test]
    fn discovery_updates_selected_platform_and_preserves_config() {
        let mut app = app();
        app.selected_device = Some(
            app.devices
                .iter()
                .find(|d| d.id == "android-device")
                .unwrap()
                .clone(),
        );
        app.page = Page::Configs;
        assert!(!app.visible_configs().iter().any(|c| c.name == "Solana"));
        app.discovered = vec![Device {
            id: "android-device".into(),
            name: "Pixel".into(),
            platform: "android-arm64".into(),
            available: true,
            favorite: false,
        }];
        app.merge();
        assert_eq!(
            app.selected_device.as_ref().unwrap().platform,
            "android-arm64"
        );
        assert!(app.visible_configs().iter().any(|c| c.name == "Solana"));
        assert_eq!(app.visible_configs()[app.config_index].name, "Ordinary");
    }

    #[test]
    fn tester_never_offers_integration_tests() {
        let mut app = app();
        app.selected_device = Some(
            app.devices
                .iter()
                .find(|d| d.id == "flutter-tester")
                .unwrap()
                .clone(),
        );
        app.page = Page::Configs;
        assert_eq!(
            app.visible_configs()
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Unit tests"]
        );
    }

    #[test]
    fn filtered_selection_survives_apply_and_moves_to_configs() {
        let mut app = app();
        for c in "/android".chars() {
            assert!(
                !app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE), 100, 30)
                    .unwrap()
            );
        }
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 100, 30)
            .unwrap();
        assert_eq!(app.visible_devices().len(), 1);
        assert!(!app.searching);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 100, 30)
            .unwrap();
        assert_eq!(app.page, Page::Configs);
        assert!(app.search.is_none());
        assert_eq!(app.selected_device.as_ref().unwrap().id, "android-device");
    }

    #[test]
    fn layouts_render_at_small_and_large_sizes() {
        let mut app = app();
        for (width, height) in [(20, 8), (34, 12), (80, 24), (120, 40)] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            for page in [Page::Devices, Page::Configs] {
                app.page = page;
                terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            }
        }
    }

    #[test]
    fn quit_requires_explicit_confirmation_and_enter_cancels() {
        let mut app = app();
        assert!(
            !app.key(
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
                100,
                30
            )
            .unwrap()
        );
        assert!(app.quit_confirmation);
        assert!(
            !app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 100, 30)
                .unwrap()
        );
        assert!(!app.quit_confirmation);
        app.key(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            100,
            30,
        )
        .unwrap();
        assert!(
            app.key(
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
                100,
                30
            )
            .unwrap()
        );
    }

    #[test]
    fn grabbing_thumb_does_not_jump_and_drag_reaches_endpoints() {
        let mut app = app();
        let mut parser = vt100::Parser::new(23, 109, 1000);
        for index in 0..100 {
            parser.process(format!("line:{index}\r\n").as_bytes());
        }
        let screen = parser.screen_mut();
        screen.set_scrollback(usize::MAX);
        app.last_log_scrollback = screen.scrollback();
        screen.set_scrollback(0);
        app.last_log = Some(parser);
        app.page = Page::LastLog;
        app.prepare_last_log(23);
        let area = ratatui::layout::Rect::new(0, 0, 112, 32);
        let track = ui::console_scrollbar_area(area);
        let bottom = track.bottom() - 1;
        app.begin_scrollbar_drag(bottom, track);
        assert_eq!(app.last_log.as_ref().unwrap().screen().scrollback(), 0);
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: track.x,
                row: track.y,
                modifiers: KeyModifiers::NONE,
            },
            area,
        )
        .unwrap();
        assert_eq!(
            app.last_log.as_ref().unwrap().screen().scrollback(),
            app.last_log_scrollback
        );
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: track.x,
                row: bottom,
                modifiers: KeyModifiers::NONE,
            },
            area,
        )
        .unwrap();
        assert_eq!(app.last_log.as_ref().unwrap().screen().scrollback(), 0);
    }
}
