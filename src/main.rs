mod console_style;
mod runs;
mod shortcuts;
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
use flimru::{
    config::{LaunchConfig, LaunchFile},
    device::{Device, Discovery, DiscoveryEvent, merge_devices},
    session::Session,
    state::{self, State},
    tool::{FlutterTool, has_ownit_helpers},
};
use shortcuts::Shortcut;
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
    /// Color output policy. Auto follows NO_COLOR.
    #[arg(long, value_enum, default_value_t = console_style::ColorMode::Auto)]
    color: console_style::ColorMode,
    /// Start in multi-device selection mode (up to eight devices per batch).
    #[arg(long)]
    multi: bool,
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
    colors: bool,
    actions_open: bool,
    action_index: usize,
    action_offset: usize,
    action_scroll_drag: bool,
    top_focus: Option<ui::TopFocus>,
    toolbar_offset: usize,
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
    multi: bool,
    marked_devices: Vec<Device>,
    page: Page,
    search: Option<String>,
    searching: bool,
    busy: bool,
    status: String,
    ownit: bool,
    runs: Vec<runs::Run>,
    active_run: usize,
    retain_consoles: bool,
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
        Ok(saved) => (saved, true, "Choose a device to continue.".into()),
        Err(error) => (
            State::default(),
            false,
            format!("{error:#}. Favorites will not be saved until this file is repaired."),
        ),
    };
    let ownit = !options.no_project_helpers && has_ownit_helpers(&workspace);
    let discovery = Discovery::start(tool.clone(), Duration::from_secs(options.refresh_interval));
    let mut app = App {
        colors: options.color.enabled(),
        actions_open: false,
        action_index: 0,
        action_offset: 0,
        action_scroll_drag: false,
        top_focus: None,
        toolbar_offset: 0,
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
        multi: options.multi,
        marked_devices: vec![],
        page: Page::Devices,
        search: None,
        searching: false,
        busy: false,
        status,
        ownit,
        runs: vec![],
        active_run: 0,
        retain_consoles: false,
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
    crossterm::style::force_color_output(app.colors);
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
        let mut previous_colors = app.colors;
        while !quit.load(Ordering::Relaxed) {
            let frame_started = Instant::now();
            if !terminal_input::connected()? {
                break;
            }
            redraw |= app.tick()?;
            let area = terminal.size()?;
            if app.colors != previous_colors {
                execute!(std::io::stdout(), crossterm::style::ResetColor)?;
                crossterm::style::force_color_output(app.colors);
                // Repaint without clear()'s cursor-position query: terminals
                // which cannot answer it must still support the color toggle.
                terminal.resize(ratatui::layout::Rect::new(0, 0, area.width, area.height))?;
                previous_colors = app.colors;
                redraw = true;
            }
            redraw |= area != previous_size;
            previous_size = area;
            for run in &mut app.runs {
                run.prepare(area.height.saturating_sub(9).max(1));
                if let Some(session) = &mut run.session {
                    session.resize(
                        area.height.saturating_sub(9).max(1),
                        area.width.saturating_sub(3).max(1),
                    )?;
                }
            }
            let animations = app
                .active_session()
                .map(|session| session.animations.state());
            redraw |= animations != previous_animations;
            previous_animations = animations;
            if redraw || last_draw.elapsed() >= Duration::from_secs(1) {
                app.prepare_last_log(area.height.saturating_sub(9).max(1));
                ui::update_picker_viewport(
                    &mut app,
                    ratatui::layout::Rect::new(0, 0, area.width, area.height),
                );
                ui::update_top_viewport(
                    &mut app,
                    ratatui::layout::Rect::new(0, 0, area.width, area.height),
                );
                if app.actions_open {
                    ui::update_action_viewport(
                        &mut app,
                        ratatui::layout::Rect::new(0, 0, area.width, area.height),
                    );
                }
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
                    Event::Paste(text) => {
                        redraw = true;
                        app.paste(&text);
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
    app.runs.clear();
    let disconnected = !terminal_input::connected().unwrap_or(false);
    if disconnected {
        let _ = terminal_input::detach_output();
    }
    drop(restore_terminal);
    // A hangup can race with drawing/resizing and produce an I/O error after
    // the health check. It is still a normal terminal-loss shutdown.
    if disconnected { Ok(()) } else { result }
}

impl App {
    fn active_session(&self) -> Option<&Session> {
        self.runs.get(self.active_run)?.session.as_ref()
    }

    fn active_session_mut(&mut self) -> Option<&mut Session> {
        self.runs.get_mut(self.active_run)?.session.as_mut()
    }

    fn running_count(&self) -> usize {
        self.runs.iter().filter(|r| r.session.is_some()).count()
    }

    fn targets(&self) -> Vec<&Device> {
        if self.multi {
            self.marked_devices.iter().collect()
        } else {
            self.selected_device.iter().collect()
        }
    }

    fn toggle_mark(&mut self) {
        if let Some(device) = self
            .visible_devices()
            .get(self.device_index)
            .cloned()
            .cloned()
        {
            if let Some(index) = self.marked_devices.iter().position(|d| d.id == device.id) {
                self.marked_devices.remove(index);
            } else if self.marked_devices.len() < runs::MAX_RUNS {
                self.marked_devices.push(device);
            } else {
                self.status = format!("Select at most {} devices per batch.", runs::MAX_RUNS);
                return;
            }
            self.status = format!(
                "{} devices selected · Space checks · Enter chooses configuration.",
                self.marked_devices.len()
            );
        }
    }

    fn switch_run(&mut self, index: usize) {
        if index < self.runs.len() {
            self.active_run = index;
            self.top_focus = None;
            self.toolbar_offset = 0;
            self.page = Page::Console;
            self.actions_open = false;
            self.help = false;
            self.search = None;
            self.searching = false;
            self.dragging_scrollbar = None;
            self.action_scroll_drag = false;
        }
    }

    fn begin_add(&mut self) {
        if self.running_count() == runs::MAX_RUNS {
            self.status =
                "All eight session slots are in use. Stop one before adding another.".into();
            return;
        }
        self.retain_consoles |= !self.runs.is_empty();
        self.back();
        self.marked_devices.clear();
        self.config_index = 0;
        self.merge();
        self.status = format!(
            "Add session · {} running · choose device(s), then configuration.",
            self.running_count()
        );
    }

    fn close_ended_run(&mut self, index: usize) {
        let Some(run) = self.runs.get(index) else {
            return;
        };
        if run.session.is_some() {
            self.status = "Session is running. Stop it with q/Esc before closing its tab.".into();
            return;
        }
        self.runs.remove(index);
        self.active_run = self
            .active_run
            .saturating_sub(usize::from(index < self.active_run))
            .min(self.runs.len().saturating_sub(1));
        self.top_focus = None;
        self.toolbar_offset = 0;
        self.actions_open = false;
        self.action_index = 0;
        self.action_offset = 0;
        self.dragging_scrollbar = None;
        self.action_scroll_drag = false;
        if self.runs.is_empty() {
            self.retain_consoles = false;
            if self.page == Page::Console {
                self.back();
            }
        }
        self.status = format!("Closed ended session · {} running.", self.running_count());
    }

    fn close_last_log(&mut self) {
        self.last_log = None;
        self.last_log_offset = 0;
        self.last_log_scrollback = 0;
        self.last_log_view_rows = 0;
        self.top_focus = None;
        self.toolbar_offset = 0;
        self.actions_open = false;
        self.action_index = 0;
        self.action_offset = 0;
        self.dragging_scrollbar = None;
        self.action_scroll_drag = false;
        if self.page == Page::LastLog {
            self.back();
        }
        self.status = "Closed saved console.".into();
    }

    fn start_batch(&mut self, cols: u16, rows: u16) {
        let Some(config) = self
            .visible_configs()
            .get(self.config_index)
            .cloned()
            .cloned()
        else {
            return;
        };
        let devices: Vec<_> = self.targets().into_iter().cloned().collect();
        if devices.is_empty() {
            return;
        }
        let running = self.running_count();
        if running + devices.len() > runs::MAX_RUNS {
            self.status = format!(
                "Only {} session slot(s) free. Select fewer devices or stop a session.",
                runs::MAX_RUNS - running
            );
            return;
        }
        if let Some(device) = devices.iter().find(|device| {
            self.runs.iter().any(|run| {
                run.session.is_some() && run.device.id == device.id && run.config == config.name
            })
        }) {
            self.status = format!(
                "Already running: {} on {}. Choose another device or configuration.",
                config.name, device.name
            );
            return;
        }
        // Validate every command before starting any child. Device IDs from
        // launch.json remain constraints, and are never silently overridden.
        let plans: Result<Vec<_>> = devices
            .iter()
            .map(|d| self.tool.plan(&config, &d.id, self.ownit))
            .collect();
        let plans = match plans {
            Ok(plans) => plans,
            Err(error) => {
                self.status = format!("Cannot start batch: {error:#}");
                return;
            }
        };
        let mut next_runs = Vec::with_capacity(devices.len());
        for (device, plan) in devices.into_iter().zip(plans) {
            let run = match Session::start_with_color(
                &plan,
                rows.saturating_sub(9),
                cols.saturating_sub(3),
                Some(self.colors),
            ) {
                Ok(mut session) => {
                    if config.is_test() {
                        session.animations = flimru::animation::AnimationControl::new(false);
                    }
                    runs::Run::running(device, config.name.clone(), session)
                }
                Err(error) => runs::Run::failed(
                    device,
                    config.name.clone(),
                    format!("{error:#}"),
                    rows.saturating_sub(9),
                    cols.saturating_sub(3),
                ),
            };
            next_runs.push(run);
        }
        if !self.multi
            && let Some(error) = next_runs.first().and_then(|r| r.outcome.as_ref())
        {
            self.status = format!("Cannot start: {error}");
            return;
        }
        let added_device = next_runs.first().map(|r| r.device.id.clone());
        if running == 0 {
            self.runs.clear();
        } else {
            // Completed histories are the only eviction candidates. Never
            // replace a running child, and keep total retained consoles bounded.
            while self.runs.len() + next_runs.len() > runs::MAX_RUNS {
                if let Some(index) = self.runs.iter().position(|r| r.session.is_none()) {
                    self.runs.remove(index);
                } else {
                    // Capacity was checked before spawning. New children are
                    // still owned locally and are cleaned up on this return.
                    self.status = "No console slots available.".into();
                    return;
                }
            }
        }
        let first_new = self.runs.len();
        self.runs.extend(next_runs);
        self.retain_consoles = self.multi || running > 0;
        self.last_log.take();
        self.last_log_scrollback = 0;
        self.last_log_offset = 0;
        self.saved.last_device = added_device;
        self.saved.last_config = Some(config.name.clone());
        self.active_config = config.name;
        self.save();
        self.status = format!("{} session(s) running.", self.running_count());
        self.switch_run(first_new);
    }

    fn pinned(&self) -> Vec<String> {
        let mut ids: Vec<_> = self
            .launch
            .configurations
            .iter()
            .filter_map(|c| c.device_id.clone())
            .collect();
        ids.extend(self.marked_devices.iter().map(|d| d.id.clone()));
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
            if !device.available
                && !device.favorite
                && let Some(marked) = self.marked_devices.iter().find(|d| d.id == device.id)
            {
                device.name = marked.name.clone();
                device.platform = marked.platform.clone();
            }
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
        for marked in &mut self.marked_devices {
            if let Some(updated) = self.devices.iter().find(|d| d.id == marked.id) {
                *marked = updated.clone();
            }
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
        let targets = self.targets();
        self.launch
            .configurations
            .iter()
            .filter(|c| {
                !targets.is_empty()
                    && targets.iter().all(|d| self.compatible(c, d))
                    && (targets.len() == 1 || c.needs_device())
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
        let mut ended = false;
        for run in &mut self.runs {
            let was_running = run.session.is_some();
            changed |= run.tick();
            ended |= was_running && run.session.is_none();
        }
        if ended {
            let running = self.running_count();
            self.status = format!("{running} session(s) running.");
            if !self.retain_consoles && running == 0 && self.runs.len() == 1 {
                let mut run = self.runs.remove(0);
                self.last_log = run.console.take();
                self.last_log_scrollback = run.scrollback_len;
                self.last_log_offset = 0;
                self.active_config = run.config;
                self.status = format!(
                    "{} ended ({}).",
                    self.active_config,
                    run.outcome.as_deref().unwrap_or("finished")
                );
                self.back();
                self.merge();
            }
        }
        Ok(changed)
    }

    fn back(&mut self) {
        self.top_focus = None;
        self.toolbar_offset = 0;
        self.actions_open = false;
        self.page = Page::Devices;
        self.search = None;
        self.searching = false;
        self.selected_device = None;
        self.help = false;
        self.dragging_scrollbar = None;
        self.action_scroll_drag = false;
    }

    fn key(&mut self, key: KeyEvent, cols: u16, rows: u16) -> Result<bool> {
        // Leave function keys to the terminal/macOS; they are never launcher
        // commands or an implicit dismissal of a saved console or modal.
        if matches!(key.code, KeyCode::F(_)) {
            return Ok(false);
        }
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
            self.actions_open = false;
            self.quit_confirmation = true;
            return Ok(false);
        }
        let shortcut = Shortcut::from_key(key);
        if shortcut == Some(Shortcut::Actions) {
            self.actions_open = !self.actions_open;
            self.searching = false;
            self.help = false;
            self.dragging_scrollbar = None;
            self.action_scroll_drag = false;
            self.action_index = 0;
            self.action_offset = 0;
            return Ok(false);
        }
        if self.actions_open {
            let actions = ui::available_actions(self);
            match key.code {
                KeyCode::BackTab => {
                    self.action_index = self
                        .action_index
                        .checked_sub(1)
                        .unwrap_or(actions.len().saturating_sub(1))
                }
                KeyCode::Tab => self.action_index = (self.action_index + 1) % actions.len().max(1),
                KeyCode::Up | KeyCode::Char('k') => {
                    self.action_index = self.action_index.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.action_index =
                        (self.action_index + 1).min(actions.len().saturating_sub(1));
                }
                KeyCode::PageUp => {
                    self.action_index = self
                        .action_index
                        .saturating_sub(rows.saturating_sub(5).max(1) as usize)
                }
                KeyCode::PageDown => {
                    self.action_index = self
                        .action_index
                        .saturating_add(rows.saturating_sub(5).max(1) as usize)
                        .min(actions.len().saturating_sub(1))
                }
                KeyCode::Home => self.action_index = 0,
                KeyCode::End => self.action_index = actions.len().saturating_sub(1),
                KeyCode::Esc => self.actions_open = false,
                KeyCode::Enter | KeyCode::Char(' ') => {
                    let action = actions.get(self.action_index).map(|item| item.action);
                    self.actions_open = false;
                    if let Some(action) = action {
                        return self.activate(action, ratatui::layout::Rect::new(0, 0, cols, rows));
                    }
                }
                _ => {
                    self.actions_open = false;
                    return self.key(key, cols, rows);
                }
            }
            return Ok(false);
        }
        if self.help {
            self.help = false;
            return Ok(false);
        }
        if shortcut == Some(Shortcut::FocusActions) {
            if self.top_focus.is_some() {
                self.top_focus = None;
            } else {
                ui::focus_top(self, false);
            }
            return Ok(false);
        }
        if self.top_focus.is_some() {
            match key.code {
                KeyCode::Left => ui::step_top(self, -1),
                KeyCode::Right => ui::step_top(self, 1),
                KeyCode::Home => ui::focus_top(self, false),
                KeyCode::End => ui::focus_top(self, true),
                KeyCode::Enter | KeyCode::Char(' ') => return self.activate_top(cols, rows),
                KeyCode::Down | KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => {
                    self.top_focus = None
                }
                KeyCode::Up => {}
                _ if shortcut.is_some() => {}
                _ => return Ok(false),
            }
            if shortcut.is_none() {
                return Ok(false);
            }
        } else if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) && self.page != Page::Console
        {
            ui::focus_top(self, key.code == KeyCode::BackTab);
            return Ok(false);
        }
        if shortcut == Some(Shortcut::Colors) {
            self.colors = !self.colors;
            self.status = if self.colors {
                "Console colors enabled (Ctrl+l toggles)."
            } else {
                "Console colors disabled (Ctrl+l toggles)."
            }
            .into();
            return Ok(false);
        }
        if matches!(
            shortcut,
            Some(Shortcut::PreviousConsole | Shortcut::NextConsole)
        ) {
            if !self.runs.is_empty() {
                let count = self.runs.len();
                let index = if shortcut == Some(Shortcut::PreviousConsole) {
                    (self.active_run + count - 1) % count
                } else if self.page == Page::Console {
                    (self.active_run + 1) % count
                } else {
                    self.active_run
                };
                self.switch_run(index);
            }
            return Ok(false);
        }
        if shortcut == Some(Shortcut::Devices) {
            self.retain_consoles |= !self.runs.is_empty();
            self.back();
            return Ok(false);
        }
        if shortcut == Some(Shortcut::AddSession) {
            self.begin_add();
            return Ok(false);
        }
        if shortcut == Some(Shortcut::CloseSession) {
            match self.page {
                Page::Console => self.close_ended_run(self.active_run),
                Page::LastLog => self.close_last_log(),
                _ => self.status = "Select an ended session tab to close it.".into(),
            }
            return Ok(false);
        }
        if shortcut == Some(Shortcut::StopAll) {
            let mut errors = vec![];
            for run in &mut self.runs {
                if let Some(session) = &mut run.session
                    && let Err(error) = session.request_stop()
                {
                    errors.push(format!("{}: {error:#}", run.device.name));
                }
            }
            self.status = if errors.is_empty() {
                "Stopping all sessions. Ctrl+x again forces any remaining sessions.".into()
            } else {
                errors.join("; ")
            };
            return Ok(false);
        }
        if shortcut == Some(Shortcut::Animations) {
            if self.page == Page::Console
                && let Some(session) = self.active_session_mut()
                && let Err(error) = session.animations.toggle()
            {
                self.status = format!("Console input: {error:#}");
            }
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
            if matches!(
                key.code,
                KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End
            ) {
                if let Some(run) = self.runs.get_mut(self.active_run) {
                    match key.code {
                        KeyCode::PageUp => run.scroll(rows.saturating_sub(9).max(1) as isize),
                        KeyCode::PageDown => run.scroll(-(rows.saturating_sub(9).max(1) as isize)),
                        KeyCode::Home => run.set_offset(usize::MAX),
                        _ => run.set_offset(0),
                    }
                }
                return Ok(false);
            }
            if self.active_session().is_none() {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                    self.back();
                }
                return Ok(false);
            }
            if let Some(session) = self.active_session_mut() {
                let result = match key.code {
                    KeyCode::Esc | KeyCode::Char('q')
                        if !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        session.request_stop()
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
                KeyCode::Home => self.last_log_offset = self.last_log_max_offset(),
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
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && query.chars().count() < 512 =>
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
            KeyCode::Char('m') if self.page == Page::Devices => {
                self.retain_consoles |= !self.runs.is_empty();
                self.multi = !self.multi;
                self.marked_devices.clear();
                self.status = if self.multi {
                    "Multi-device mode · Space checks devices · Enter chooses configuration."
                } else {
                    "Single-device mode · Enter chooses configuration."
                }
                .into();
            }
            KeyCode::Char(' ') if self.page == Page::Devices && self.multi => self.toggle_mark(),
            KeyCode::Up if *index == 0 => ui::focus_top(self, false),
            KeyCode::Up | KeyCode::Char('k') => *index = index.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                *index = (*index + 1).min(count.saturating_sub(1))
            }
            KeyCode::Home => *index = 0,
            KeyCode::End => *index = count.saturating_sub(1),
            KeyCode::Char('/') => {
                self.search.get_or_insert_default();
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
                if self.multi && self.marked_devices.is_empty() {
                    self.status =
                        "Check at least one device with Space or the checkbox first.".into();
                    return Ok(false);
                }
                if let Some(device) = self
                    .visible_devices()
                    .get(self.device_index)
                    .map(|d| (*d).clone())
                {
                    self.selected_device = Some(device);
                    self.top_focus = None;
                    self.toolbar_offset = 0;
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
                self.start_batch(cols, rows);
            }
            _ => {}
        }
        Ok(false)
    }
}

impl App {
    fn paste(&mut self, text: &str) {
        if self.quit_confirmation || self.help || self.actions_open || self.top_focus.is_some() {
            return;
        }
        if self.page == Page::Console {
            if let Some(session) = self.active_session_mut()
                && let Err(error) = session.send(text.as_bytes())
            {
                self.status = error.to_string();
            }
        } else if self.searching {
            let query = self.search.get_or_insert_default();
            let remaining = 512_usize.saturating_sub(query.chars().count());
            query.extend(
                text.chars()
                    .filter(|character| !character.is_control())
                    .take(remaining),
            );
            self.device_index = 0;
            self.config_index = 0;
        }
    }

    fn mouse(&mut self, event: MouseEvent, area: ratatui::layout::Rect) -> Result<bool> {
        self.prepare_last_log(area.height.saturating_sub(9).max(1));
        ui::update_picker_viewport(self, area);
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
            && !self.actions_open
            && !self.help
            && !self.quit_confirmation
            && ui::top_hit(self, area, event).is_none()
        {
            self.top_focus = None;
        }
        // Menu clicks use the viewport actually drawn, not a pending keyboard
        // or wheel selection which has not reached the next frame yet.
        if matches!(event.kind, MouseEventKind::Up(MouseButton::Left)) {
            self.dragging_scrollbar = None;
            self.action_scroll_drag = false;
            return Ok(false);
        }
        if !self.quit_confirmation && !self.help {
            if self.actions_open
                && self.action_scroll_drag
                && matches!(event.kind, MouseEventKind::Drag(MouseButton::Left))
            {
                self.action_index = ui::action_scroll_index(self, area, event.row);
                return Ok(false);
            }
            let wheel = match event.kind {
                MouseEventKind::ScrollUp => Some(-1),
                MouseEventKind::ScrollDown => Some(1),
                _ => None,
            };
            if let Some(direction) = wheel {
                if self.actions_open {
                    self.action_index = self
                        .action_index
                        .saturating_add_signed(direction)
                        .min(ui::available_actions(self).len().saturating_sub(1));
                    return Ok(false);
                }
                if event.row <= area.y + 2 {
                    if self.top_focus.is_none() {
                        ui::focus_top(self, false);
                    } else {
                        ui::step_top(self, direction);
                    }
                    return Ok(false);
                }
                self.top_focus = None;
                match self.page {
                    Page::Console => {
                        if let Some(run) = self.runs.get_mut(self.active_run) {
                            run.scroll(-direction * 3);
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
        // Pointer commands return to content. Keyboard focus is explicitly
        // entered with Tab/Ctrl+g and restored when a keyboard-opened modal closes.
        self.top_focus = None;
        self.activate(action, area)
    }

    fn activate_top(&mut self, cols: u16, rows: u16) -> Result<bool> {
        if let Some(action) = ui::focused_top_action(self) {
            return self.activate(action, ratatui::layout::Rect::new(0, 0, cols, rows));
        }
        Ok(false)
    }

    fn activate(&mut self, action: ui::MouseAction, area: ratatui::layout::Rect) -> Result<bool> {
        let focus = self.top_focus.take();
        let page = self.page;
        let result = self.dispatch(action, area);
        if self.top_focus.is_none()
            && self.page == page
            && !self.searching
            && !matches!(
                action,
                ui::MouseAction::CloseConsole(_) | ui::MouseAction::CloseLastLog
            )
        {
            self.top_focus = focus;
        }
        result
    }

    fn dispatch(&mut self, action: ui::MouseAction, area: ratatui::layout::Rect) -> Result<bool> {
        use ui::MouseAction;
        let key = match action {
            MouseAction::FocusToolbar(index) => {
                ui::focus_toolbar_item(self, index);
                return Ok(false);
            }
            MouseAction::ToggleMulti => KeyCode::Char('m'),
            MouseAction::ToggleDevice(index) => {
                self.device_index = index;
                self.toggle_mark();
                return Ok(false);
            }
            MouseAction::PreviousConsole => {
                return self.key(Shortcut::PreviousConsole.key(), area.width, area.height);
            }
            MouseAction::NextConsole => {
                return self.key(Shortcut::NextConsole.key(), area.width, area.height);
            }
            MouseAction::Devices => {
                return self.key(Shortcut::Devices.key(), area.width, area.height);
            }
            MouseAction::AddSession => {
                return self.key(Shortcut::AddSession.key(), area.width, area.height);
            }
            MouseAction::StopAll => {
                return self.key(Shortcut::StopAll.key(), area.width, area.height);
            }
            MouseAction::SwitchConsole(index) => {
                self.switch_run(index);
                return Ok(false);
            }
            MouseAction::CloseConsole(index) => {
                self.close_ended_run(index);
                return Ok(false);
            }
            MouseAction::CloseLastLog => {
                self.close_last_log();
                return Ok(false);
            }
            MouseAction::ScrollActions(index) => {
                self.action_index = index;
                self.action_scroll_drag = true;
                return Ok(false);
            }
            MouseAction::Actions => {
                return self.key(Shortcut::Actions.key(), area.width, area.height);
            }
            MouseAction::DismissActions => {
                self.actions_open = false;
                return Ok(false);
            }
            MouseAction::SelectAction(index) => {
                let action = ui::available_actions(self)
                    .get(index)
                    .map(|item| item.action);
                self.actions_open = false;
                if let Some(action) = action {
                    return self.activate(action, area);
                }
                return Ok(false);
            }
            MouseAction::Filter => {
                self.search.get_or_insert_default();
                self.searching = true;
                return Ok(false);
            }
            MouseAction::ApplyFilter => {
                self.searching = false;
                return Ok(false);
            }
            MouseAction::ClearFilter => {
                self.search = None;
                self.searching = false;
                self.device_index = 0;
                self.config_index = 0;
                return Ok(false);
            }
            MouseAction::ToggleColors => {
                return self.key(Shortcut::Colors.key(), area.width, area.height);
            }
            MouseAction::FlutterKey(key) => KeyCode::Char(key),
            MouseAction::OldestOutput => KeyCode::Home,
            MouseAction::FollowOutput => KeyCode::End,
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
                if self.last_log.is_some() {
                    self.page = Page::LastLog;
                    self.actions_open = false;
                    self.searching = false;
                    self.search = None;
                    self.dragging_scrollbar = None;
                    self.action_scroll_drag = false;
                }
                return Ok(false);
            }
            MouseAction::Quit => {
                self.actions_open = false;
                self.quit_confirmation = true;
                return Ok(false);
            }
            MouseAction::ConfirmQuit => return Ok(true),
            MouseAction::CancelQuit => {
                self.quit_confirmation = false;
                return Ok(false);
            }
            MouseAction::Help | MouseAction::DismissHelp => {
                self.help = !self.help;
                return Ok(false);
            }
            MouseAction::ToggleAnimations => {
                return self.key(Shortcut::Animations.key(), area.width, area.height);
            }
            MouseAction::ConsoleScroll(row) => {
                let track = ui::console_scrollbar_area(area);
                self.begin_scrollbar_drag(track.y.saturating_add(row), track);
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
            Page::Console => match self
                .runs
                .get(self.active_run)
                .and_then(|r| r.screen().map(|s| (r, s)))
            {
                Some((run, screen)) => (
                    run.max_offset(),
                    run.max_offset().saturating_sub(run.offset()),
                    (screen.size().0 as usize).min(track.height as usize),
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
                if let Some(run) = self.runs.get_mut(self.active_run) {
                    run.set_offset(run.max_offset().saturating_sub(position));
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
            executable: workspace.join("flimru-nonexistent-test-tool"),
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
            colors: true,
            actions_open: false,
            action_index: 0,
            action_offset: 0,
            action_scroll_drag: false,
            top_focus: None,
            toolbar_offset: 0,
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
            multi: false,
            marked_devices: vec![],
            page: Page::Devices,
            search: None,
            searching: false,
            busy: false,
            status: String::new(),
            ownit: true,
            runs: vec![],
            active_run: 0,
            retain_consoles: false,
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
    fn top_actions_use_arrows_without_activating_or_losing_modal_focus() {
        let mut app = app();
        let press = |app: &mut App, code| {
            app.key(KeyEvent::new(code, KeyModifiers::NONE), 34, 12)
                .unwrap()
        };
        press(&mut app, KeyCode::Tab);
        assert!(!app.actions_open);
        assert_eq!(
            ui::focused_top_action(&app),
            Some(ui::MouseAction::NextOrRun)
        );
        press(&mut app, KeyCode::Right);
        assert_eq!(
            ui::focused_top_action(&app),
            Some(ui::MouseAction::ToggleMulti)
        );
        assert!(!app.multi);
        press(&mut app, KeyCode::Enter);
        assert!(app.multi);
        press(&mut app, KeyCode::Char(' '));
        assert!(!app.multi);
        press(&mut app, KeyCode::Down);
        assert!(app.top_focus.is_none());
        app.device_index = 0;
        press(&mut app, KeyCode::Up);
        assert!(app.top_focus.is_some());
        for _ in 0..12 {
            if ui::focused_top_action(&app) == Some(ui::MouseAction::Actions) {
                break;
            }
            press(&mut app, KeyCode::Right);
        }
        let origin = app.top_focus;
        assert_eq!(ui::focused_top_action(&app), Some(ui::MouseAction::Actions));
        press(&mut app, KeyCode::Enter);
        assert!(app.actions_open);
        press(&mut app, KeyCode::Esc);
        assert!(!app.actions_open);
        assert_eq!(app.top_focus, origin);
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::End);
        press(&mut app, KeyCode::Enter);
        assert!(app.quit_confirmation);
        assert!(!press(&mut app, KeyCode::Enter));
        assert!(!app.quit_confirmation);
        assert_eq!(app.top_focus, origin);
        press(&mut app, KeyCode::Esc);
        assert!(app.top_focus.is_none());
        assert_eq!(app.page, Page::Devices);
        press(&mut app, KeyCode::Tab);
        app.paste("ignored");
        assert!(app.search.is_none());
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.page, Page::Configs);
        assert!(app.top_focus.is_none());
    }

    #[test]
    fn header_navigation_and_disappearing_actions_are_safe() {
        let mut app = app();
        app.top_focus = Some(ui::TopFocus::Navigation(1));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 80, 24)
            .unwrap();
        assert_eq!(app.page, Page::Configs);
        app.top_focus = Some(ui::TopFocus::Navigation(0));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 80, 24)
            .unwrap();
        assert_eq!(app.page, Page::Devices);
        app.top_focus = Some(ui::TopFocus::Toolbar(ui::MouseAction::LastLog));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 80, 24)
            .unwrap();
        assert_eq!(app.page, Page::Devices);
        assert!(!app.quit_confirmation && !app.actions_open);
        ui::update_top_viewport(&mut app, ratatui::layout::Rect::new(0, 0, 34, 12));
        assert!(app.top_focus.is_none());
    }

    #[test]
    #[cfg(unix)]
    fn adding_sessions_evicts_only_completed_histories_and_enforces_capacity() {
        let mut app = app();
        app.multi = true;
        app.ownit = false;
        let workspace = app.launch.workspace.clone();
        app.launch = LaunchFile::parse(
            r#"{"configurations":[{"name":"Shared"}]}"#,
            &workspace.join("launch.json"),
            &workspace,
        )
        .unwrap();
        let device = app.devices[0].clone();
        let sleeper = |device: Device, name: &str| {
            runs::Run::running(
                device,
                name.into(),
                Session::start(
                    &flimru::tool::RunCommand {
                        executable: "/bin/sh".into(),
                        args: vec!["-c".into(), "exec sleep 30".into()],
                        cwd: workspace.clone(),
                        env: Default::default(),
                    },
                    5,
                    70,
                )
                .unwrap(),
            )
        };
        app.runs = (0..7)
            .map(|i| {
                runs::Run::failed(device.clone(), format!("old{i}"), "completed".into(), 5, 70)
            })
            .collect();
        let mut live = sleeper(device.clone(), "Live");
        live.session
            .as_mut()
            .unwrap()
            .parser
            .process(b"preserved live output");
        app.runs.push(live);
        let mut added_device = device.clone();
        added_device.id = "additional-device".into();
        app.marked_devices = vec![added_device.clone()];
        for _ in 0..3 {
            app.start_batch(80, 24); // A failed additional spawn also retains the live process.
            assert_eq!(app.runs.len(), runs::MAX_RUNS);
            assert_eq!(app.running_count(), 1);
            assert_eq!(app.active_run, runs::MAX_RUNS - 1);
            let live = app.runs.iter_mut().find(|r| r.config == "Live").unwrap();
            assert!(live.session.as_mut().unwrap().tick().unwrap().is_none());
            assert!(
                live.screen()
                    .unwrap()
                    .contents()
                    .contains("preserved live output")
            );
        }
        assert!(!app.runs.iter().any(|r| r.config == "old0"));
        assert_eq!(app.saved.last_device.as_deref(), Some("additional-device"));
        app.runs = (0..8)
            .map(|i| {
                let mut d = device.clone();
                d.id = format!("running-{i}");
                sleeper(d, "Shared")
            })
            .collect();
        app.page = Page::Console;
        app.begin_add();
        assert_eq!(app.page, Page::Console);
        assert!(app.status.contains("eight session slots"));
        app.start_batch(80, 24);
        assert!(app.status.contains("0 session slot(s) free"));
        assert_eq!(app.running_count(), 8);
        app.runs.pop();
        app.marked_devices.push(device);
        app.start_batch(80, 24);
        assert!(app.status.contains("1 session slot(s) free"));
        assert_eq!(app.running_count(), 7);
        app.marked_devices = vec![app.runs[0].device.clone()];
        app.start_batch(80, 24);
        assert!(app.status.contains("Already running"));
        assert_eq!(app.running_count(), 7);
    }

    #[test]
    #[cfg(unix)]
    fn last_session_ending_during_add_does_not_interrupt_configuration_selection() {
        let mut app = app();
        let device = app.devices[0].clone();
        let session = Session::start(
            &flimru::tool::RunCommand {
                executable: "/bin/sh".into(),
                args: vec!["-c".into(), "exit 0".into()],
                cwd: app.launch.workspace.clone(),
                env: Default::default(),
            },
            5,
            70,
        )
        .unwrap();
        app.runs
            .push(runs::Run::running(device, "Original".into(), session));
        app.switch_run(0);
        app.begin_add();
        app.key(
            KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
            80,
            24,
        )
        .unwrap();
        assert!(app.multi);
        app.page = Page::Configs;
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.running_count() > 0 {
            assert!(Instant::now() < deadline);
            app.tick().unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(app.page, Page::Configs);
        assert_eq!(app.runs.len(), 1);
        assert!(app.runs[0].outcome.is_some());
    }

    #[test]
    fn multi_selection_is_bounded_and_survives_filtering_and_discovery() {
        let mut app = app();
        app.multi = true;
        app.discovered = (0..10)
            .map(|i| Device {
                id: format!("device-{i}"),
                name: format!("Phone {i}"),
                platform: "ios".into(),
                available: true,
                favorite: false,
            })
            .collect();
        app.merge();
        for i in 0..10 {
            app.device_index = app
                .visible_devices()
                .iter()
                .position(|d| d.id == format!("device-{i}"))
                .unwrap();
            app.toggle_mark();
        }
        assert_eq!(app.marked_devices.len(), runs::MAX_RUNS);
        assert!(app.status.contains("at most"));
        app.search = Some("Phone 1".into());
        app.discovered.clear();
        app.merge();
        assert_eq!(app.marked_devices.len(), runs::MAX_RUNS);
        assert!(app.devices.iter().any(|d| d.id == "device-0"));
        app.search = None;
        app.device_index = app
            .visible_devices()
            .iter()
            .position(|d| d.id == "device-1")
            .unwrap();
        app.toggle_mark();
        assert_eq!(app.marked_devices.len(), runs::MAX_RUNS - 1);
        assert!(!app.marked_devices.iter().any(|d| d.id == "device-1"));
    }

    #[test]
    fn batch_validation_precedes_start_and_spawn_errors_have_separate_consoles() {
        let mut app = app();
        app.multi = true;
        app.ownit = false;
        let device = app
            .devices
            .iter()
            .find(|d| d.id == "android-device")
            .unwrap()
            .clone();
        let mut other = device.clone();
        other.id = "second".into();
        app.marked_devices = vec![device.clone(), other];
        app.page = Page::Configs;
        let workspace = app.launch.workspace.clone();
        app.launch = LaunchFile::parse(
            r#"{"configurations":[{"name":"Invalid task","preLaunchTask":"unsupported"}]}"#,
            &workspace.join("launch.json"),
            &workspace,
        )
        .unwrap();
        app.runs.push(runs::Run::failed(
            device,
            "Previous".into(),
            "retained".into(),
            5,
            70,
        ));
        app.start_batch(80, 24);
        assert!(app.status.contains("Cannot start batch"));
        assert_eq!(app.runs.len(), 1);
        assert_eq!(app.runs[0].config, "Previous");
        app.launch.configurations[0].pre_launch_task = None;
        app.start_batch(80, 24); // The fixture SDK executable deliberately does not exist.
        assert_eq!(app.runs.len(), 2);
        assert_eq!(app.running_count(), 0);
        assert_eq!(app.page, Page::Console);
        assert!(
            app.runs
                .iter()
                .all(|r| r.outcome.as_ref().unwrap().starts_with("failed:"))
        );
        assert!(
            app.runs
                .iter()
                .all(|r| r.screen().unwrap().contents().contains("Cannot start:"))
        );
    }

    #[test]
    fn multi_configs_must_support_every_device_and_device_less_tests_run_once() {
        let mut app = app();
        app.multi = true;
        let device = app
            .devices
            .iter()
            .find(|d| d.id == "android-device")
            .unwrap()
            .clone();
        let mut other = device.clone();
        other.id = "another-device".into();
        app.marked_devices = vec![device, other];
        let names: Vec<_> = app
            .visible_configs()
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, vec!["Integration tests"]);
        app.marked_devices.pop();
        assert!(app.visible_configs().iter().any(|c| c.name == "Ordinary"));
        app.marked_devices.clear();
        assert!(app.visible_configs().is_empty());
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 80, 24)
            .unwrap();
        assert_eq!(app.page, Page::Devices);
        assert!(app.status.contains("at least one"));
    }

    #[test]
    fn closing_ended_tabs_preserves_selection_and_last_close_returns_to_devices() {
        let mut app = app();
        let device = app.devices[0].clone();
        app.runs = (0..3)
            .map(|index| {
                runs::Run::failed(
                    device.clone(),
                    format!("Tab {index}"),
                    format!("history {index}"),
                    10,
                    70,
                )
            })
            .collect();
        app.retain_consoles = true;
        app.switch_run(1);
        app.runs[1].set_offset(0);
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        app.activate(ui::MouseAction::CloseConsole(0), area)
            .unwrap();
        assert_eq!(app.active_run, 0);
        assert_eq!(app.runs[0].config, "Tab 1");
        assert!(
            app.runs[0]
                .screen()
                .unwrap()
                .contents()
                .contains("history 1")
        );
        app.top_focus = Some(ui::TopFocus::Toolbar(ui::MouseAction::CloseConsole(0)));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 80, 24)
            .unwrap();
        assert!(app.top_focus.is_none());
        assert_eq!(app.runs[0].config, "Tab 2");
        app.key(Shortcut::CloseSession.key(), 80, 24).unwrap();
        assert!(app.runs.is_empty());
        assert_eq!(app.active_run, 0);
        assert_eq!(app.page, Page::Devices);
        assert!(!app.retain_consoles);
        app.close_ended_run(9); // Stale tab targets cannot remove another session.
    }

    #[test]
    fn close_saved_console_releases_history_and_preserves_picker_flow() {
        let mut app = app();
        app.last_log = Some(vt100::Parser::new(10, 70, 100));
        app.page = Page::Configs;
        let selected = app.selected_device.clone();
        app.close_last_log();
        assert_eq!(app.page, Page::Configs);
        assert_eq!(app.selected_device, selected);
        assert!(app.last_log.is_none());
        app.last_log = Some(vt100::Parser::new(10, 70, 100));
        app.last_log_offset = 10;
        app.last_log_scrollback = 20;
        app.page = Page::LastLog;
        app.key(Shortcut::CloseSession.key(), 80, 24).unwrap();
        assert_eq!(app.page, Page::Devices);
        assert!(app.last_log.is_none());
        assert_eq!(app.last_log_scrollback, 0);
        assert_eq!(app.last_log_offset, 0);
    }

    #[test]
    fn completed_console_switching_and_pointer_actions_preserve_history() {
        let mut app = app();
        let device = app.devices[0].clone();
        app.runs = vec![
            runs::Run::failed(device.clone(), "First".into(), "one".into(), 20, 70),
            runs::Run::failed(device, "Second".into(), "two".into(), 20, 70),
        ];
        let area = ratatui::layout::Rect::new(0, 0, 34, 12);
        app.switch_run(0);
        app.activate(ui::MouseAction::NextConsole, area).unwrap();
        assert_eq!(app.active_run, 1);
        assert!(app.runs[1].screen().unwrap().contents().contains("two"));
        app.key(Shortcut::PreviousConsole.key(), 34, 12).unwrap();
        assert_eq!(app.active_run, 0);
        app.activate(ui::MouseAction::Devices, area).unwrap();
        assert_eq!(app.page, Page::Devices);
        app.activate(ui::MouseAction::SwitchConsole(1), area)
            .unwrap();
        assert_eq!(app.page, Page::Console);
        assert_eq!(app.active_run, 1);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), 34, 12)
            .unwrap();
        assert_eq!(app.page, Page::Devices);
        assert_eq!(app.runs.len(), 2);
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
    fn function_keys_are_inert_and_console_shortcuts_do_not_edit_picker_state() {
        let mut app = app();
        let favorites = app.saved.favorites.clone();
        for page in [Page::Devices, Page::Configs, Page::Console, Page::LastLog] {
            app.page = page;
            for number in 1..=12 {
                app.key(
                    KeyEvent::new(KeyCode::F(number), KeyModifiers::NONE),
                    80,
                    24,
                )
                .unwrap();
                assert_eq!(app.page, page);
                assert!(app.colors && !app.help && !app.actions_open);
                assert!(app.top_focus.is_none());
            }
        }
        app.page = Page::Devices;
        for shortcut in [
            Shortcut::PreviousConsole,
            Shortcut::NextConsole,
            Shortcut::Animations,
        ] {
            app.key(shortcut.key(), 80, 24).unwrap();
        }
        assert_eq!(app.page, Page::Devices);
        assert_eq!(app.saved.favorites, favorites);
        assert!(!app.searching && !app.multi);
    }

    #[test]
    fn filter_and_color_commands_agree_between_keyboard_and_pointer_actions() {
        let mut keyboard = app();
        let mut pointer = app();
        let area = ratatui::layout::Rect::new(0, 0, 34, 12);
        keyboard
            .key(
                KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
                34,
                12,
            )
            .unwrap();
        pointer.activate(ui::MouseAction::Filter, area).unwrap();
        assert_eq!(keyboard.searching, pointer.searching);
        keyboard.paste("phone\r\n");
        pointer.paste("phone\r\n");
        assert_eq!(keyboard.search.as_deref(), Some("phone"));
        assert_eq!(keyboard.search, pointer.search);
        keyboard
            .key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 34, 12)
            .unwrap();
        pointer
            .activate(ui::MouseAction::ApplyFilter, area)
            .unwrap();
        assert!(!keyboard.searching && !pointer.searching);
        keyboard
            .key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), 34, 12)
            .unwrap();
        pointer
            .activate(ui::MouseAction::ClearFilter, area)
            .unwrap();
        assert_eq!(keyboard.search, pointer.search);
        keyboard.key(Shortcut::Colors.key(), 34, 12).unwrap();
        pointer
            .activate(ui::MouseAction::ToggleColors, area)
            .unwrap();
        assert_eq!(keyboard.colors, pointer.colors);
    }

    #[test]
    fn action_menu_is_modal_and_filters_bound_pasted_input() {
        let mut app = app();
        app.searching = true;
        app.paste(&"é".repeat(600));
        assert_eq!(app.search.as_ref().unwrap().chars().count(), 512);
        app.key(Shortcut::Actions.key(), 34, 12).unwrap();
        app.paste("ignored");
        assert_eq!(app.search.as_ref().unwrap().chars().count(), 512);
        assert!(app.actions_open);
        app.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE), 34, 12)
            .unwrap();
        assert!(
            !app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 34, 12)
                .unwrap()
        );
        assert!(app.quit_confirmation && !app.actions_open);
        assert!(
            !app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 34, 12)
                .unwrap()
        );
        assert!(!app.quit_confirmation);
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
