mod availability_worker;
mod batch_report;
mod clipboard;
pub mod console_search;
mod console_style;
pub mod export_dialog;
mod runs;
mod selection;
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
    availability::{self, Assessment},
    batch_commands::{self, BatchReport, BatchTarget, HotCommand, SendFailure},
    config::{LaunchFile, SourceEntry},
    config_reload::{self, ConfigReloader, ReloadError, ReloadResult, ReloadSource},
    console_review::{ReviewTicket, ReviewWorker, SeverityFilter},
    console_snapshot,
    device::{Device, Discovery, DiscoveryEvent, merge_devices},
    log_export::{self, ExportError, ExportMetadata, ExportRequest, ExportWorker, SavedExport},
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

struct Update {
    redraw: bool,
    output_received: bool,
}

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

const DIAGNOSTIC_TEXT_CHARS: usize = 512;
const CONFIG_LABEL_CHARS: usize = 256;
const DISPLAYED_WARNING_LIMIT: usize = 16;

// Bound work before wrapping/rendering; retain full source strings for --check.
fn diagnostic_text(text: &str, limit: usize) -> String {
    let mut characters = text.chars();
    let mut shown: String = characters.by_ref().take(limit).collect();
    if characters.next().is_some() {
        shown.push_str("… [shortened; --check for full text]");
    }
    shown
}

// Keep one extra character so SearchView can detect and mark shortened metadata.
fn source_label(text: &str, limit: usize) -> String {
    text.chars()
        .filter(|ch| !ch.is_control())
        .take(limit + 1)
        .collect()
}

fn append_display_warnings(lines: &mut Vec<String>, title: &str, warnings: &[String]) {
    if warnings.is_empty() {
        return;
    }
    lines.push(title.into());
    lines.extend(
        warnings
            .iter()
            .take(DISPLAYED_WARNING_LIMIT)
            .map(|warning| diagnostic_text(warning, DIAGNOSTIC_TEXT_CHARS)),
    );
    let remaining = warnings.len().saturating_sub(DISPLAYED_WARNING_LIMIT);
    if remaining > 0 {
        lines.push(format!(
            "{remaining} more warnings; use --check for full output."
        ));
    }
}

#[derive(Default)]
struct AvailabilityView {
    catalog: Option<Arc<LaunchFile>>,
    generation: u64,
    worker: Option<availability_worker::Worker>,
    targets: Vec<(String, String, String)>,
    ownit: bool,
    assessments: std::collections::HashMap<usize, Assessment>,
    synchronous_overrides: std::collections::HashSet<usize>,
    desired_source: Option<usize>,
    ready: usize,
    query: String,
    show_unavailable: bool,
    visible: Vec<usize>,
    visible_dirty: bool,
}

// Hold the cached index list rather than allocating/scanning the catalog for
// each toolbar, footer, count and list read in a frame.
struct VisibleEntries<'a> {
    launch: &'a LaunchFile,
    indexes: std::cell::Ref<'a, Vec<usize>>,
}

impl<'a> VisibleEntries<'a> {
    fn len(&self) -> usize {
        self.indexes.len()
    }
    fn is_empty(&self) -> bool {
        self.indexes.is_empty()
    }
    fn get(&self, index: usize) -> Option<&'a SourceEntry> {
        self.indexes
            .get(index)
            .and_then(|i| self.launch.entries.get(*i))
    }
    fn iter(&self) -> impl DoubleEndedIterator<Item = &'a SourceEntry> + ExactSizeIterator + '_ {
        self.indexes.iter().map(|i| &self.launch.entries[*i])
    }
}
impl std::ops::Index<usize> for VisibleEntries<'_> {
    type Output = SourceEntry;
    fn index(&self, index: usize) -> &Self::Output {
        &self.launch.entries[self.indexes[index]]
    }
}

pub(crate) struct App {
    selection: Option<selection::Selection>,
    clipboard: clipboard::Clipboard,
    actions_open: bool,
    action_index: usize,
    action_offset: usize,
    action_scroll_drag: bool,
    top_focus: Option<ui::TopFocus>,
    toolbar_offset: usize,
    launch: Arc<LaunchFile>,
    config_reloader: Option<ConfigReloader>,
    reload_generation: u64,
    manual_reload_pending: Option<u64>,
    reload_input_drain_pending: bool,
    reload_error: Option<ReloadError>,
    availability_view: std::cell::RefCell<AvailabilityView>,
    tool: FlutterTool,
    discovery: Discovery,
    saved: State,
    state_path: PathBuf,
    persistence_enabled: bool,
    discovered: Vec<Device>,
    devices: Vec<Device>,
    device_index: usize,
    config_index: usize,
    show_unavailable: bool,
    reveal_unavailable_after_check: bool,
    diagnostics_open: bool,
    diagnostics_offset: u16,
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
    review: Option<console_search::SearchView>,
    review_notice: Option<String>,
    batch_report: Option<batch_report::BatchReportView>,
    pending_export: Option<PendingExport>,
    last_operation: Option<String>,
    last_log: Option<vt100::Parser>,
    last_log_source: Option<console_search::SearchSource>,
    last_log_max_columns: u16,
    last_log_history_capacity: usize,
    last_log_scrollback: usize,
    last_log_offset: usize,
    last_log_view_rows: u16,
    active_config: String,
    help: bool,
    quit_confirmation: bool,
    dragging_scrollbar: Option<ScrollDrag>,
}

/// At most one export capture/write and its optional editable prompt. Closing
/// the prompt while writing retains this owner until the actual result arrives.
struct PendingExport {
    source_id: runs::RunId,
    workspace: PathBuf,
    dialog: Option<export_dialog::ExportDialog>,
    capture: Option<(ReviewWorker, ReviewTicket)>,
    writer: ExportWorker,
}

/// Catalog positions are not identities. A unique name can survive edits; a
/// repeated name needs one identical entry in BOTH catalogs to prove identity.
fn preserve_config_source(
    old: &LaunchFile,
    new: &LaunchFile,
    source: Option<usize>,
) -> Option<usize> {
    let selected = old.entries.get(source?)?;
    let named = |entry: &&SourceEntry| entry.name == selected.name;
    let old_name_count = old.entries.iter().filter(named).take(2).count();
    let mut new_named_entries = new.entries.iter().filter(named);
    let first_named_entry = new_named_entries.next()?;
    if old_name_count == 1 && new_named_entries.next().is_none() {
        return Some(first_named_entry.source_index);
    }
    let selected_config = selected
        .configuration
        .and_then(|index| old.configurations.get(index));
    let same_entry = |catalog: &LaunchFile, entry: &SourceEntry| {
        entry.name == selected.name
            && entry.diagnostics == selected.diagnostics
            && entry.warnings == selected.warnings
            && entry
                .configuration
                .and_then(|index| catalog.configurations.get(index))
                == selected_config
    };
    if old
        .entries
        .iter()
        .filter(|entry| same_entry(old, entry))
        .take(2)
        .count()
        != 1
    {
        return None;
    }
    let mut matching = new.entries.iter().filter(|entry| same_entry(new, entry));
    let matched = matching.next()?;
    matching.next().is_none().then_some(matched.source_index)
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
    let launch = config_reload::load_bounded(&path, &workspace)?;
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
        for entry in &launch.entries {
            println!("  {}", entry.name);
            let assessment = availability::assess(
                &launch,
                entry,
                &tool,
                &[],
                !options.no_project_helpers && has_ownit_helpers(&workspace),
            );
            for reason in assessment.reasons {
                println!("    Cannot launch: {}", reason.display());
            }
            if assessment.target_required {
                println!("    Select a target to assess device compatibility.");
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
        selection: None,
        clipboard: clipboard::Clipboard::default(),
        actions_open: false,
        action_index: 0,
        action_offset: 0,
        action_scroll_drag: false,
        top_focus: None,
        toolbar_offset: 0,
        config_reloader: Some(ConfigReloader::new(path, workspace)?),
        reload_generation: 0,
        manual_reload_pending: None,
        reload_input_drain_pending: false,
        reload_error: None,
        launch: Arc::new(launch),
        availability_view: Default::default(),
        tool,
        discovery,
        saved,
        state_path,
        persistence_enabled,
        discovered: vec![],
        devices: vec![],
        device_index: 0,
        config_index: 0,
        show_unavailable: false,
        reveal_unavailable_after_check: false,
        diagnostics_open: false,
        diagnostics_offset: 0,
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
        review: None,
        review_notice: None,
        batch_report: None,
        pending_export: None,
        last_operation: None,
        last_log: None,
        last_log_source: None,
        last_log_max_columns: 1,
        last_log_history_capacity: Session::SCROLLBACK_CAPACITY,
        last_log_scrollback: 0,
        last_log_offset: 0,
        last_log_view_rows: 0,
        active_config: String::new(),
        help: false,
        quit_confirmation: false,
        dragging_scrollbar: None,
    };
    app.merge();
    app.advance_availability();
    if app.launch.entries.is_empty()
        || (app.availability_counts().2 == 0
            && !app
                .availability_view
                .borrow()
                .assessments
                .values()
                .any(|assessment| assessment.reasons.is_empty()))
    {
        app.open_diagnostics_picker();
    }
    let mut startup_assessment = app.availability_counts().2 != 0;
    let quit = Arc::new(AtomicBool::new(false));
    let signal_quit = quit.clone();
    ctrlc::set_handler(move || signal_quit.store(true, Ordering::Relaxed))?;
    let restore_terminal = RestoreTerminal;
    crossterm::style::force_color_output(true);
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
        let mut previous_size = terminal.size()?;
        let mut previous_animations = None;
        while !quit.load(Ordering::Relaxed) {
            let frame_started = Instant::now();
            if !terminal_input::connected()? {
                break;
            }
            let update = app.tick()?;
            redraw |= update.redraw;
            if startup_assessment && app.availability_counts().2 == 0 {
                startup_assessment = false;
                if app.page == Page::Devices
                    && app.selected_device.is_none()
                    && !app
                        .availability_view
                        .borrow()
                        .assessments
                        .values()
                        .any(|assessment| assessment.reasons.is_empty())
                {
                    app.open_diagnostics_picker();
                    redraw = true;
                }
            }
            let area = terminal.size()?;
            if area != previous_size {
                app.selection = None;
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
            let animations = app.active_session().map(|session| {
                (
                    session.animations.state(),
                    session.animations.connection_error(),
                )
            });
            redraw |= animations != previous_animations;
            previous_animations = animations;
            // Input wakes poll immediately; quiet background workers only need
            // a bounded check. Keep the normal frame rate while output changes.
            let wait_interval = if redraw || update.output_received {
                frame_interval
            } else {
                Duration::from_millis(100)
            };
            if redraw {
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
                terminal.draw(|frame| app.draw(frame))?;
                redraw = false;
            }
            let mut exit_requested = false;
            let mut input_started = None;
            for input_index in 0..128 {
                if !terminal_input::connected()? {
                    exit_requested = true;
                    break;
                }
                let timeout = if input_index == 0 {
                    wait_interval.saturating_sub(frame_started.elapsed())
                } else {
                    Duration::from_millis(1)
                };
                // The tty backend needs a positive timeout to inspect its
                // parser. Waiting here avoids a separate idle polling sleep.
                if !event::poll(timeout.max(Duration::from_millis(1)))? {
                    // Only an observed idle queue re-arms Run after reload.
                    // Reaching either input budget leaves queued intent blocked.
                    redraw |= app.input_queue_idle();
                    break;
                }
                // The input budget measures ready-event work, excluding the
                // blocking wait for the first event. Drain bursts together.
                let input_started = input_started.get_or_insert_with(Instant::now);
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
                    Event::Resize(width, height) => {
                        app.batch_report_event(&Event::Resize(width, height));
                        redraw = true;
                    }
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
    app.review.take();
    let final_export = app.shutdown_export();
    let disconnected = !terminal_input::connected().unwrap_or(false);
    if disconnected {
        let _ = terminal_input::detach_output();
    }
    drop(restore_terminal);
    if !disconnected && let Some(message) = final_export {
        eprintln!("{message}");
    }
    // A hangup can race with drawing/resizing and produce an I/O error after
    // the health check. It is still a normal terminal-loss shutdown.
    if disconnected { Ok(()) } else { result }
}

impl App {
    fn reviewing(&self) -> bool {
        self.review
            .as_ref()
            .is_some_and(console_search::SearchView::is_active)
    }

    fn reviewed_source(&self) -> Option<runs::RunId> {
        self.review
            .as_ref()
            .and_then(|view| view.source())
            .map(|source| source.id)
    }

    fn showing_operation_details(&self) -> bool {
        self.review
            .as_ref()
            .is_some_and(console_search::SearchView::showing_operation_details)
    }

    fn export_dialog_open(&self) -> bool {
        self.pending_export
            .as_ref()
            .is_some_and(|pending| pending.dialog.is_some())
    }

    /// Covered console UI owns input/cursor even without a frozen review source.
    /// Capture identity remains separate from presentation ownership.
    fn console_ui_owned(&self) -> bool {
        self.reviewing()
            || self.showing_operation_details()
            || self.export_dialog_open()
            || self.batch_report.is_some()
    }

    fn batch_targets(&self) -> Vec<BatchTarget> {
        self.runs.iter().map(runs::Run::batch_target).collect()
    }

    fn hot_eligible_count(&self) -> usize {
        self.runs
            .iter()
            .map(runs::Run::batch_target)
            .filter(|target| target.exclusion().is_none())
            .count()
    }

    fn dispatch_hot_command(
        &mut self,
        command: HotCommand,
        targets: &[BatchTarget],
    ) -> BatchReport {
        batch_commands::dispatch(command, targets, |id, bytes| {
            let session = self
                .runs
                .iter_mut()
                .find(|run| run.id == id)
                .and_then(|run| run.session.as_mut())
                .filter(|session| session.stop_requested.is_none())
                .ok_or(SendFailure::SessionUnavailable)?;
            // Keep the existing animation restart and nonblocking PTY path.
            session.send(bytes).map_err(|_| SendFailure::InputRejected)
        })
    }

    fn request_hot_command(&mut self, command: HotCommand) {
        if self.batch_report.is_some() || self.export_dialog_open() || self.quit_confirmation {
            return;
        }
        let targets = self.batch_targets();
        let report = self.dispatch_hot_command(command, &targets);
        // The delivery modal carries the result without changing review notice
        // geometry or the underlying query/filter/snapshot/scroll position.
        self.status = report.summary(command);
        self.batch_report = Some(batch_report::BatchReportView::new(command, report));
        self.actions_open = false;
        self.help = false;
        self.top_focus = None;
        self.dragging_scrollbar = None;
        self.action_scroll_drag = false;
    }

    fn batch_report_event(&mut self, event: &Event) -> bool {
        let Some(view) = &mut self.batch_report else {
            return false;
        };
        if view.handle_event(event) == batch_report::ReportAction::Closed {
            self.batch_report = None;
        }
        true
    }

    fn close_review(&mut self) {
        self.review_notice = None;
        if let Some(view) = &mut self.review {
            view.close();
        }
    }

    fn draw(&mut self, frame: &mut ratatui::Frame<'_>) {
        // Tiny layouts render global quit controls in the content layer. Do
        // not let a modal's enlarge message clear those active controls.
        if self.quit_confirmation && (frame.area().width < 34 || frame.area().height < 12) {
            ui::draw(frame, self);
            return;
        }
        if self.export_dialog_open() || self.batch_report.is_some() {
            ui::draw_content(frame, self);
            let mut area = ui::review_area(frame.area());
            if self.reviewing() && self.review_notice.is_some() {
                area.height = area.height.saturating_sub(1);
            }
            if let Some(view) = &mut self.review
                && (view.is_active() || view.showing_operation_details())
            {
                view.render_with_focus(frame, area, false);
            }
            if let Some(dialog) = self
                .pending_export
                .as_mut()
                .and_then(|pending| pending.dialog.as_mut())
            {
                dialog.render_with_focus(frame, !self.quit_confirmation);
            }
            if let Some(report) = &mut self.batch_report {
                report.render(frame);
            }
            ui::draw_overlays(frame, self);
            return;
        }
        if !self.reviewing() {
            if self.showing_operation_details() {
                ui::draw_content(frame, self);
                if let Some(view) = &mut self.review {
                    view.render_with_focus(frame, ui::review_area(frame.area()), false);
                }
                ui::draw_overlays(frame, self);
            } else {
                ui::draw(frame, self);
            }
            return;
        }
        ui::draw_content(frame, self);
        if frame.area().width >= 34 && frame.area().height >= 12 {
            let mut area = ui::review_area(frame.area());
            if self.review_notice.is_some() {
                area.height = area.height.saturating_sub(1);
            }
            let focused = !self.actions_open
                && !self.help
                && !self.quit_confirmation
                && !self.diagnostics_open
                && self.top_focus.is_none();
            if let Some(view) = &mut self.review {
                view.render_with_focus(frame, area, focused);
            }
            if let Some(notice) = &self.review_notice {
                ui::review_operation_notice(frame, notice);
            }
        }
        ui::draw_overlays(frame, self);
    }

    fn run_source(run: &runs::Run) -> console_search::SearchSource {
        let mut source = console_search::SearchSource::new(
            run.id,
            format!(
                "{} · {}",
                source_label(&run.device.name, 96),
                source_label(&run.config, 96)
            ),
            std::time::SystemTime::now(),
        );
        source.device = Some(source_label(&run.device.name, 128));
        source.configuration = Some(source_label(&run.config, 128));
        let outcome = run.outcome.as_deref().unwrap_or(
            if run
                .session
                .as_ref()
                .is_some_and(|session| session.stop_requested.is_some())
            {
                "stopping"
            } else {
                "running"
            },
        );
        source.outcome = Some(source_label(outcome, 128));
        source
    }

    /// Shared invocation-time identity and preclone bound for review/export.
    fn capture_console(
        &mut self,
    ) -> Option<(
        console_search::SearchSource,
        Result<vt100::Screen, console_snapshot::SnapshotError>,
    )> {
        let source = match self.page {
            Page::Console => self.runs.get(self.active_run).map(Self::run_source),
            Page::LastLog if self.last_log.is_some() => {
                let source = self.last_log_source.get_or_insert_with(|| {
                    let mut source = console_search::SearchSource::new(
                        runs::next_id(),
                        source_label(&self.active_config, 96),
                        std::time::SystemTime::now(),
                    );
                    source.configuration = Some(source_label(&self.active_config, 128));
                    source
                });
                let mut source = source.clone();
                source.captured_at = std::time::SystemTime::now();
                Some(source)
            }
            _ => None,
        };
        let source = source?;
        // A live drag can own its own full history clone; release it before
        // allocating the review screen, even when preflight later rejects it.
        self.selection = None;
        self.dragging_scrollbar = None;
        // This must precede the Screen clone: retained rows keep historical
        // widths, and an inactive primary grid can retain history in alt mode.
        let captured = match self.page {
            Page::Console => self.runs.get(self.active_run).and_then(|run| {
                run.screen().map(|screen| {
                    (
                        screen,
                        if screen.alternate_screen() {
                            run.history_capacity
                        } else {
                            run.scrollback()
                        },
                        run.historical_columns(),
                    )
                })
            }),
            Page::LastLog => self.last_log.as_ref().map(|parser| {
                (
                    parser.screen(),
                    if parser.screen().alternate_screen() {
                        self.last_log_history_capacity
                    } else {
                        self.last_log_scrollback
                    },
                    self.last_log_max_columns.max(parser.screen().size().1),
                )
            }),
            _ => None,
        }
        .map(|(screen, history, columns)| {
            flimru::console_snapshot::estimate_screen_capture_with_max_columns(
                screen, history, columns,
            )
            .map(|_| screen.clone())
        });
        Some((source, captured?))
    }

    fn capture_review(&mut self, refresh: bool) {
        let Some((source, captured)) = self.capture_console() else {
            self.status = "Select a console to search.".into();
            return;
        };
        if self.review.is_none() {
            match console_search::SearchView::new(console_style::review_severity) {
                Ok(view) => self.review = Some(view),
                Err(error) => {
                    self.status = format!("Cannot start console review: {error}");
                    return;
                }
            }
        }
        self.review_notice = None;
        self.top_focus = None;
        self.actions_open = false;
        self.help = false;
        if let Some(view) = &mut self.review {
            match captured {
                Ok(screen) => {
                    let _ = if refresh {
                        view.refresh(source, screen)
                    } else {
                        view.capture(source, screen)
                    };
                }
                Err(error) => view.show_error(source, Self::capture_error_message(error)),
            }
        }
    }

    fn capture_error_message(error: console_snapshot::SnapshotError) -> String {
        match error {
            console_snapshot::SnapshotError::CaptureLimit => "128 MiB capture limit. Start a new session in a narrower terminal; history stays wide.".into(),
            _ => error.to_string(),
        }
    }

    fn search_console(&mut self) {
        if self.reviewing() {
            self.top_focus = None;
            if let Some(view) = &mut self.review {
                view.edit_query();
            }
        } else {
            self.capture_review(false);
        }
    }

    fn open_export(&mut self) {
        if self.pending_export.is_some() {
            self.report_operation(
                "A console export is already pending; wait for its result.".into(),
            );
            return;
        }
        // Capture metadata and the exact shared review Arc before any later tab
        // change. A pending review must finish, never become a different capture.
        let (source, snapshot, captured) = if self.reviewing() {
            let Some(view) = &self.review else {
                return;
            };
            let (Some(source), Some(snapshot)) = (view.source().cloned(), view.snapshot()) else {
                let message = match view.error() {
                    Some(error) => format!("Cannot export this snapshot: {error}"),
                    None if view.is_pending() => "Console snapshot is not ready. Wait for capture to finish before exporting.".into(),
                    None => "No console snapshot is available to export. Capture or refresh a snapshot first.".into(),
                };
                self.report_operation(message);
                return;
            };
            self.selection = None;
            (source, Some(snapshot), None)
        } else {
            let Some((source, captured)) = self.capture_console() else {
                self.report_operation("Select a retained console to export.".into());
                return;
            };
            (source, None, Some(captured))
        };
        let metadata = ExportMetadata::new(
            source
                .captured_at
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            source.device.as_deref(),
            source.configuration.as_deref(),
            source.outcome.as_deref(),
        );
        let workspace = self.launch.workspace.clone();
        let destination = match log_export::default_destination(&workspace, &metadata) {
            Ok(destination) => destination,
            Err(error) => {
                self.report_operation(error.to_string());
                return;
            }
        };
        let mut dialog =
            export_dialog::ExportDialog::new(&source.label, metadata, &destination, snapshot);
        let mut capture = None;
        if let Some(captured) = captured {
            let started = captured
                .map_err(Self::capture_error_message)
                .and_then(|screen| {
                    let mut worker = ReviewWorker::new(console_style::review_severity)
                        .map_err(|_| "Could not start console export capture worker.".to_owned())?;
                    let ticket = worker
                        .capture(source.id, screen, String::new(), SeverityFilter::All)
                        .map_err(|error| error.to_string())?;
                    Ok((worker, ticket))
                });
            match started {
                Ok(worker) => capture = Some(worker),
                Err(message) => {
                    dialog.set_error(&message);
                    self.report_operation(message);
                }
            }
        }
        self.top_focus = None;
        self.dragging_scrollbar = None;
        self.actions_open = false;
        self.help = false;
        self.diagnostics_open = false;
        self.pending_export = Some(PendingExport {
            source_id: source.id,
            workspace,
            dialog: Some(dialog),
            capture,
            writer: ExportWorker::new(),
        });
    }

    /// Modal routing consumes every event, before selection/review/PTY fallback.
    fn export_event(&mut self, event: &Event) -> bool {
        let Some(pending) = &mut self.pending_export else {
            return false;
        };
        let Some(dialog) = &mut pending.dialog else {
            return false;
        };
        match dialog.handle_event(event) {
            export_dialog::DialogAction::Submit(destination) => {
                let result = dialog
                    .snapshot()
                    .ok_or(ExportError::InvalidInput("snapshot is not ready"))
                    .and_then(|snapshot| {
                        ExportRequest::new(
                            &pending.workspace,
                            &destination,
                            Arc::clone(snapshot),
                            dialog.metadata().clone(),
                        )
                    })
                    .and_then(|request| pending.writer.start(request));
                if let Err(error) = result {
                    let message = error.to_string();
                    dialog.set_error(&message);
                    self.report_operation(message);
                }
            }
            export_dialog::DialogAction::Cancel => self.cancel_export(),
            export_dialog::DialogAction::Consumed => {}
        }
        true
    }

    fn cancel_export(&mut self) {
        let Some(mut pending) = self.pending_export.take() else {
            return;
        };
        pending.dialog = None;
        // Capture has bounded cancellation checkpoints; its owned worker joins.
        pending.capture.take();
        if pending.writer.is_running() {
            pending.writer.cancel();
            self.pending_export = Some(pending);
            self.report_operation("Cancelling console export; waiting for its result.".into());
        } else {
            self.report_operation("Console export cancelled.".into());
        }
    }

    fn export_result_message(result: &Result<SavedExport, ExportError>) -> String {
        match result {
            Ok(saved) => format!(
                "Saved console log to {} ({} bytes).",
                saved.path.display(),
                saved.bytes
            ),
            Err(error) => error.to_string(),
        }
    }

    fn advance_export(&mut self) -> bool {
        let Some(mut pending) = self.pending_export.take() else {
            return false;
        };
        let mut changed = false;
        if let Some((worker, ticket)) = &mut pending.capture
            && let Some(update) = worker.take_update()
            && update.ticket == *ticket
        {
            changed = true;
            if let Some(dialog) = &mut pending.dialog {
                match update.result {
                    Ok(data) => dialog.set_snapshot(data.snapshot),
                    Err(error) => {
                        let message = error.to_string();
                        dialog.set_error(&message);
                        self.report_operation(message);
                    }
                }
            }
            pending.capture.take();
        }
        if let Some(result) = pending.writer.poll() {
            self.report_operation(Self::export_result_message(&result));
            if let Err(error) = &result
                && *error != ExportError::Cancelled
                && let Some(dialog) = &mut pending.dialog
            {
                dialog.set_error(&error.to_string());
                self.pending_export = Some(pending);
            }
            return true;
        }
        self.pending_export = Some(pending);
        changed
    }

    fn shutdown_export(&mut self) -> Option<String> {
        let mut pending = self.pending_export.take()?;
        pending.capture.take();
        let result = pending.writer.shutdown()?;
        let message = Self::export_result_message(&result);
        self.report_operation(message);
        // The final terminal print must use the same sanitized, bounded outcome
        // as Operation details, including canonical paths reached via symlinks.
        self.last_operation.clone()
    }

    fn review_event(&mut self, event: &Event) -> bool {
        let effect = self
            .review
            .as_mut()
            .map(|view| view.handle_event(event))
            .unwrap_or(console_search::SearchEffect::Inactive);
        match effect {
            console_search::SearchEffect::Inactive => false,
            console_search::SearchEffect::RefreshSnapshot => {
                self.capture_review(true);
                true
            }
            console_search::SearchEffect::ResumeLive => {
                self.status = "Resumed live console.".into();
                true
            }
            console_search::SearchEffect::Consumed => true,
        }
    }

    fn report_operation(&mut self, message: String) {
        let mut message: String = message
            .chars()
            .filter(|ch| !ch.is_control())
            .take(console_search::MAX_OPERATION_CHARS + 1)
            .collect();
        if message.chars().count() > console_search::MAX_OPERATION_CHARS {
            message = message
                .chars()
                .take(console_search::MAX_OPERATION_CHARS - 16)
                .collect();
            message.push_str("… [shortened]");
        }
        self.last_operation = Some(message.clone());
        self.status = message;
        if self.showing_operation_details()
            && let Some(view) = &mut self.review
        {
            view.show_operation_details(&self.status);
        }
        if self.reviewing() {
            self.review_notice = Some(self.status.clone());
        }
    }

    fn send_flutter_key(&mut self, key: char) {
        let operation = match key {
            'r' => "Hot reload",
            'R' => "Hot restart",
            'h' => "Flutter help",
            'v' => "DevTools",
            _ => "Console input",
        };
        if let Some(session) = self.active_session_mut() {
            let result = session.send(key.to_string().as_bytes());
            self.report_operation(match result {
                Ok(()) => format!("{operation} requested"),
                Err(error) => format!("{operation}: {error:#}"),
            });
        }
    }
    fn toggle_animations(&mut self) {
        if let Some(session) = self.active_session_mut() {
            let result = session.animations.toggle();
            self.report_operation(match result {
                Ok(()) => "Animation change requested".into(),
                Err(error) => format!("Animation control: {error:#}"),
            });
        }
    }
    fn stop_all(&mut self) {
        let mut errors = vec![];
        for run in &mut self.runs {
            if let Some(session) = &mut run.session
                && let Err(error) = session.request_stop()
            {
                errors.push(format!("{}: {error:#}", run.device.name));
            }
        }
        self.report_operation(if errors.is_empty() {
            if self.console_ui_owned() {
                "Stopping all sessions. Use Actions to force any remaining sessions."
            } else {
                "Stopping all sessions. Ctrl+x again forces any remaining sessions."
            }
            .into()
        } else {
            errors.join("; ")
        });
    }

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
        self.selection = None;
        if index < self.runs.len() {
            if self
                .reviewed_source()
                .is_some_and(|source| source != self.runs[index].id)
            {
                self.close_review();
            }
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

    fn cycle_run(&mut self, previous: bool) {
        if !self.runs.is_empty() {
            let count = self.runs.len();
            let index = if previous {
                (self.active_run + count - 1) % count
            } else if self.page == Page::Console {
                (self.active_run + 1) % count
            } else {
                self.active_run
            };
            self.switch_run(index);
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
        self.set_config_selection(0);
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
        let source_id = run.id;
        if self
            .pending_export
            .as_ref()
            .is_some_and(|pending| pending.source_id == source_id)
        {
            self.cancel_export();
        }
        if self.reviewed_source() == Some(source_id) {
            self.close_review();
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
        if self.pending_export.as_ref().is_some_and(|pending| {
            Some(pending.source_id) == self.last_log_source.as_ref().map(|source| source.id)
        }) {
            self.cancel_export();
        }
        if self.reviewed_source() == self.last_log_source.as_ref().map(|source| source.id) {
            self.close_review();
        }
        self.last_log = None;
        self.last_log_source = None;
        self.last_log_max_columns = 1;
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
        if let Some(message) = self.reload_run_guard() {
            self.status = message.into();
            return;
        }
        let Some(entry) = self.visible_entries().get(self.config_index) else {
            self.status = "Select a configuration first.".into();
            return;
        };
        let presented_runnable = self.assessment(entry).runnable();
        let assessment =
            availability::assess(&self.launch, entry, &self.tool, &self.targets(), self.ownit);
        self.cache_assessment(entry, assessment.clone(), true);
        if !presented_runnable || !assessment.runnable() {
            let source_index = entry.source_index;
            self.show_unavailable = true;
            let index = self
                .visible_entries()
                .iter()
                .position(|entry| entry.source_index == source_index)
                .unwrap_or(0);
            self.set_config_selection(index);
            self.diagnostics_open = true;
            self.diagnostics_offset = 0;
            return;
        }
        let Some(config) = entry
            .configuration
            .and_then(|i| self.launch.configurations.get(i))
            .cloned()
        else {
            return;
        };
        let devices: Vec<_> = self.targets().into_iter().cloned().collect();
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
        // Availability and launch share this all-target preflight and its plans.
        let plans = assessment.plans;
        let mut next_runs = Vec::with_capacity(devices.len());
        for (device, plan) in devices.into_iter().zip(plans) {
            let run = match Session::start(&plan, rows.saturating_sub(9), cols.saturating_sub(3)) {
                Ok(mut session) => {
                    if config.is_test() {
                        session.animations = flimru::animation::AnimationControl::new(false);
                    }
                    runs::Run::running(device, config.name.clone(), session, plan.hot_capabilities)
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
        self.last_log_source = None;
        self.last_log_max_columns = 1;
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

    // A source can be deliberately retained while absent from a partial visible
    // list. That deferred source is the logical selection during catalog edits.
    fn selected_config_source(&self) -> Option<usize> {
        let visible = self.visible_entries();
        self.availability_view.borrow().desired_source.or_else(|| {
            visible
                .get(self.config_index)
                .map(|entry| entry.source_index)
        })
    }

    fn reload_run_guard(&self) -> Option<&'static str> {
        if self.manual_reload_pending.is_some() {
            Some("Reloading configurations. Wait, then choose Run explicitly.")
        } else if self.reload_input_drain_pending {
            Some("Finishing reload… Choose Run when it appears.")
        } else {
            None
        }
    }

    fn input_queue_idle(&mut self) -> bool {
        if self.manual_reload_pending.is_none() && self.reload_input_drain_pending {
            if self
                .reload_run_guard()
                .is_some_and(|message| self.status == message)
            {
                self.status = match self.reload_error {
                    Some(error) => format!("Reload failed: {error}"),
                    None => "Launch configurations reloaded.".into(),
                };
            }
            self.reload_input_drain_pending = false;
            // The event loop redraws a ready frame before processing more input.
            true
        } else {
            false
        }
    }

    fn request_config_reload(&mut self) {
        // Even a worker-start failure must not reinterpret the request's queued
        // Enter as a fresh Run. Completion re-arms this fence after idle waits
        // observed while the worker was still pending.
        self.reload_input_drain_pending = true;
        if self.config_reloader.is_none() {
            match ConfigReloader::new(self.launch.path.clone(), self.launch.workspace.clone()) {
                Ok(reloader) => self.config_reloader = Some(reloader),
                Err(_) => {
                    self.reload_error = Some(ReloadError::ReadFailed);
                    self.status =
                        "Reload failed: Could not start the configuration reload worker.".into();
                    return;
                }
            }
        }
        if let Some(reloader) = &self.config_reloader {
            let generation = reloader.request_reload();
            self.reload_generation = generation;
            self.manual_reload_pending = Some(generation);
            self.status = "Reloading launch configurations…".into();
        }
    }

    fn advance_config_reload(&mut self) -> bool {
        let result = self.config_reloader.as_mut().and_then(ConfigReloader::poll);
        result.is_some_and(|result| self.apply_config_reload(result))
    }

    fn apply_config_reload(&mut self, result: ReloadResult) -> bool {
        if result.generation < self.reload_generation {
            return false;
        }
        self.reload_generation = result.generation;
        // A newer automatic version may coalesce an unread manual completion.
        // Its completed read also supersedes that explicit request's version.
        let completing_manual = self
            .manual_reload_pending
            .is_some_and(|generation| result.generation >= generation);
        let manual = result.source == ReloadSource::Manual || completing_manual;
        if completing_manual {
            self.manual_reload_pending = None;
        }
        if manual {
            self.reload_input_drain_pending = true;
        }
        match result.result {
            Err(error) => {
                let changed = self.reload_error.replace(error) != Some(error);
                if manual {
                    self.status = format!("Reload failed: {error}");
                }
                changed || manual
            }
            Ok(launch) => {
                let previous_error = self.reload_error.take();
                let resolving_status = previous_error
                    .is_some_and(|error| self.status == format!("Reload failed: {error}"));
                if !manual && previous_error.is_none() && self.launch.as_ref() == &launch {
                    return false;
                }
                let source =
                    preserve_config_source(&self.launch, &launch, self.selected_config_source());
                self.launch = Arc::new(launch);
                self.set_config_source_selection(source);
                self.merge();
                if manual || resolving_status {
                    self.status = "Launch configurations reloaded.".into();
                }
                true
            }
        }
    }

    fn merge(&mut self) {
        let config_source = self.selected_config_source();
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
        if let Some(source) = config_source {
            self.set_config_source_selection(Some(source));
        } else if self.config_index != usize::MAX {
            let count = self.visible_entries().len();
            self.config_index = self.config_index.min(count.saturating_sub(1));
        }
    }

    // Catalog application can set its chosen source (or clear selection) without
    // reaching into the view. A hidden/pending source has no actionable fallback.
    fn set_config_source_selection(&mut self, source: Option<usize>) {
        self.ensure_availability_context();
        self.availability_view.get_mut().desired_source = source;
        self.config_index = usize::MAX;
        self.restore_deferred_config_selection();
    }

    // Every deliberate picker-index choice discards a deferred source anchor.
    fn set_config_selection(&mut self, index: usize) {
        // Establish the selected view even for a fresh catalog whose first
        // draw has not happened yet (for example P2's explicit apply policy).
        drop(self.visible_entries());
        self.config_index = index;
        self.availability_view.get_mut().desired_source = None;
    }

    fn restore_deferred_config_selection(&mut self) {
        let desired = self.availability_view.borrow().desired_source;
        if let Some(source) = desired {
            let index = self.visible_entries().indexes.binary_search(&source).ok();
            self.config_index = index.unwrap_or(usize::MAX);
            if index.is_some() && self.availability_counts().2 == 0 {
                self.availability_view.get_mut().desired_source = None;
            }
        }
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

    fn ensure_availability_context(&self) {
        let targets: Vec<_> = self
            .targets()
            .iter()
            .map(|target| {
                (
                    target.id.clone(),
                    target.platform.clone(),
                    target.name.clone(),
                )
            })
            .collect();
        let identity = self.launch.clone();
        let view = self.availability_view.borrow();
        if view
            .catalog
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, &identity))
            || view.targets != targets
            || view.ownit != self.ownit
        {
            drop(view);
            let mut view = self.availability_view.borrow_mut();
            let desired_source = view
                .catalog
                .as_ref()
                .filter(|old| Arc::ptr_eq(old, &identity))
                .and_then(|_| {
                    view.desired_source
                        .or_else(|| view.visible.get(self.config_index).copied())
                });
            let generation = view.generation.wrapping_add(1);
            let worker = view.worker.take().unwrap_or_default();
            worker.request(
                generation,
                self.launch.clone(),
                self.tool.clone(),
                self.targets().into_iter().cloned().collect(),
                self.ownit,
            );
            *view = AvailabilityView {
                catalog: Some(identity),
                generation,
                worker: Some(worker),
                targets,
                ownit: self.ownit,
                desired_source,
                visible_dirty: true,
                ..Default::default()
            };
        }
    }

    fn cache_assessment(&self, entry: &SourceEntry, mut assessment: Assessment, synchronous: bool) {
        assessment.plans.clear();
        let mut view = self.availability_view.borrow_mut();
        if synchronous {
            view.synchronous_overrides.insert(entry.source_index);
        } else if view.synchronous_overrides.contains(&entry.source_index) {
            return;
        }
        if let Some(old) = view
            .assessments
            .insert(entry.source_index, assessment.clone())
        {
            view.ready -= usize::from(old.runnable());
        }
        view.ready += usize::from(assessment.runnable());
        if !view.visible_dirty {
            let included = (view.query.is_empty()
                || entry
                    .name
                    .to_lowercase()
                    .contains(&view.query.to_lowercase()))
                && (view.show_unavailable || assessment.runnable());
            match (view.visible.binary_search(&entry.source_index), included) {
                (Ok(index), false) => {
                    view.visible.remove(index);
                }
                (Err(index), true) => view.visible.insert(index, entry.source_index),
                _ => {}
            }
        }
    }

    fn assessment(&self, entry: &SourceEntry) -> Assessment {
        self.ensure_availability_context();
        self.availability_view
            .borrow()
            .assessments
            .get(&entry.source_index)
            .cloned()
            .unwrap_or_else(|| Assessment {
                pending: true,
                // Parser diagnostics are already known without filesystem work.
                reasons: entry.diagnostics.clone(),
                target_required: self.targets().is_empty(),
                plans: Vec::new(),
            })
    }

    // Only bounded, already assessed metadata crosses onto the UI thread.
    // All filesystem/planner work runs in the one owned presentation worker.
    fn advance_availability(&mut self) -> bool {
        self.ensure_availability_context();
        let results = {
            let view = self.availability_view.borrow();
            view.worker
                .as_ref()
                .map(|worker| worker.drain(view.generation))
                .unwrap_or_default()
        };
        self.apply_availability_results(results)
    }

    fn apply_availability_results(&mut self, results: Vec<(usize, Assessment)>) -> bool {
        let changed = !results.is_empty();
        for (index, assessment) in results {
            self.cache_assessment(&self.launch.entries[index], assessment, false);
        }
        self.restore_deferred_config_selection();
        changed
    }

    #[cfg(test)]
    fn settle_availability(&mut self) {
        self.ensure_availability_context();
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.availability_counts().2 != 0 {
            assert!(
                Instant::now() < deadline,
                "presentation worker did not settle"
            );
            self.advance_availability();
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn visible_entries(&self) -> VisibleEntries<'_> {
        self.ensure_availability_context();
        let query = self.search.as_deref().unwrap_or("");
        let mut view = self.availability_view.borrow_mut();
        if view.visible_dirty
            || view.query != query
            || view.show_unavailable != self.show_unavailable
        {
            view.query = query.to_owned();
            view.show_unavailable = self.show_unavailable;
            let query = query.to_lowercase();
            view.visible = self
                .launch
                .entries
                .iter()
                .enumerate()
                .filter_map(|(i, entry)| {
                    ((query.is_empty() || entry.name.to_lowercase().contains(&query))
                        && (self.show_unavailable
                            || view
                                .assessments
                                .get(&entry.source_index)
                                .is_some_and(Assessment::runnable)))
                    .then_some(i)
                })
                .collect();
            view.visible_dirty = false;
        }
        drop(view);
        VisibleEntries {
            launch: &self.launch,
            indexes: std::cell::Ref::map(self.availability_view.borrow(), |view| &view.visible),
        }
    }

    fn availability_counts(&self) -> (usize, usize, usize) {
        self.ensure_availability_context();
        let view = self.availability_view.borrow();
        (
            view.ready,
            view.assessments.len() - view.ready,
            self.launch
                .entries
                .len()
                .saturating_sub(view.assessments.len()),
        )
    }

    fn open_diagnostics_picker(&mut self) {
        self.page = Page::Configs;
        self.show_unavailable = true;
        self.set_config_selection(0);
        self.config_list_offset = 0;
        self.search = None;
        self.searching = false;
        self.top_focus = None;
        self.status = "Configuration diagnostics · i explains · u hides unavailable.".into();
    }

    fn diagnostics_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(error) = self.reload_error {
            lines.push(format!("Configuration reload error: {error}"));
            lines.push(
                "Using the last good catalog. Correct the file; reload retries automatically."
                    .into(),
            );
            lines.push(String::new());
        }
        if let Some(message) = self.reload_run_guard() {
            lines.push(message.into());
        }
        let entry = self.visible_entries().get(self.config_index).or_else(|| {
            self.availability_view
                .borrow()
                .desired_source
                .and_then(|source| self.launch.entries.get(source))
        });
        if let Some(entry) = entry {
            lines.push(diagnostic_text(&entry.name, CONFIG_LABEL_CHARS));
            let assessment = self.assessment(entry);
            if assessment.pending {
                lines
                    .push("Checking availability. Please wait, then choose Run explicitly.".into());
            } else if assessment.runnable() {
                lines.push("Ready to run on the selected target(s).".into());
            }
            lines.extend(assessment.reasons.iter().map(|reason| {
                let message = diagnostic_text(&reason.message, DIAGNOSTIC_TEXT_CHARS);
                match &reason.target {
                    Some(target) => {
                        format!("{}: {message}", diagnostic_text(target, CONFIG_LABEL_CHARS))
                    }
                    None => message,
                }
            }));
            append_display_warnings(&mut lines, "Configuration warnings:", &entry.warnings);
            if assessment.target_required {
                lines.push("Select a target to assess device compatibility and enable Run.".into());
            }
        } else if self.config_index == usize::MAX && !self.visible_entries().is_empty() {
            lines.push("Select a configuration to see details.".into());
        } else if self.launch.entries.is_empty() {
            lines.push(
                "This configurations array is empty. Add a Dart launch configuration, then reload."
                    .into(),
            );
        } else {
            lines.push(
                "No visible configurations. Clear the filter or show unavailable entries.".into(),
            );
        }
        if entry.is_none() && !self.launch.entries.is_empty() {
            let (ready, blocked, checking) = self.availability_counts();
            lines.push(format!("Catalog: {ready} ready / {blocked} blocked / {checking} checking. Select an entry to inspect its reasons and warnings."));
            if !self.launch.warnings.is_empty() {
                lines.push(format!(
                    "{} catalog warning(s); use --check for full output.",
                    self.launch.warnings.len()
                ));
            }
        }
        append_display_warnings(&mut lines, "File warnings:", &self.launch.file_warnings);
        lines
    }

    fn catalog_warning_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if !self.launch.warnings.is_empty() {
            lines.push(format!("{} catalog warning(s). Select a configuration to inspect its reasons and warnings; --check prints full output.", self.launch.warnings.len()));
        }
        append_display_warnings(&mut lines, "File warnings:", &self.launch.file_warnings);
        lines
    }

    fn save(&mut self) {
        if self.persistence_enabled
            && let Err(error) = state::save(&self.state_path, &self.saved)
        {
            self.status = format!("Could not save favorites: {error:#}");
        }
    }

    fn tick(&mut self) -> Result<Update> {
        let mut changed = self.advance_config_reload();
        changed |= self
            .review
            .as_mut()
            .is_some_and(console_search::SearchView::poll);
        let console_ui_owned = self.console_ui_owned();
        let reviewed_source = self.reviewed_source();
        let mut reviewed_outcome = None;
        let mut output_received = false;
        if let Some(selection) = &mut self.selection {
            changed |= selection.tick(Instant::now());
        }
        if let Some(result) = self.clipboard.tick() {
            changed = true;
            self.status = match result {
                Ok(()) => "Selection copied to clipboard. Esc or click clears selection.".into(),
                Err(error) => format!("Copy failed: {error:#}"),
            };
        }
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
        for (index, run) in self.runs.iter_mut().enumerate() {
            let was_running = run.session.is_some();
            let output_changed = run.tick();
            output_received |= output_changed;
            let just_ended = was_running && run.session.is_none();
            changed |= just_ended
                || (output_changed
                    && self.page == Page::Console
                    && index == self.active_run
                    && !console_ui_owned);
            if just_ended && reviewed_source == Some(run.id) {
                reviewed_outcome = run
                    .outcome
                    .as_ref()
                    .map(|outcome| source_label(outcome, 128));
            }
            ended |= just_ended;
        }
        if let Some(outcome) = reviewed_outcome {
            self.report_operation(format!("Console ended: {outcome}"));
        }
        if ended {
            let running = self.running_count();
            self.status = format!("{running} session(s) running.");
            if !self.retain_consoles
                && running == 0
                && self.runs.len() == 1
                && self.reviewed_source() != self.runs.first().map(|run| run.id)
            {
                let mut run = self.runs.remove(0);
                self.last_log_source = Some(Self::run_source(&run));
                self.last_log_max_columns = run.historical_columns();
                self.last_log_history_capacity = run.history_capacity;
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
        changed |= self.advance_availability();
        // Acknowledged export outcomes follow lifecycle/discovery notices.
        changed |= self.advance_export();
        let (ready, _, pending) = self.availability_counts();
        if self.reveal_unavailable_after_check && pending == 0 {
            self.reveal_unavailable_after_check = false;
            if ready == 0 && self.page == Page::Configs {
                self.show_unavailable = true;
                changed = true;
            }
        }
        Ok(Update {
            redraw: changed,
            output_received,
        })
    }

    fn back(&mut self) {
        self.close_review();
        self.selection = None;
        self.top_focus = None;
        self.toolbar_offset = 0;
        self.actions_open = false;
        self.page = Page::Devices;
        self.search = None;
        self.searching = false;
        self.selected_device = None;
        self.help = false;
        self.diagnostics_open = false;
        self.reveal_unavailable_after_check = false;
        self.dragging_scrollbar = None;
        self.action_scroll_drag = false;
    }

    fn key(&mut self, key: KeyEvent, cols: u16, rows: u16) -> Result<bool> {
        // Leave function keys to the terminal/macOS; they are never launcher
        // commands or an implicit dismissal of a saved console or modal.
        if matches!(key.code, KeyCode::F(_)) {
            return Ok(false);
        }
        // Global quit owns input before every modal and text selection.
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
        if self.batch_report.is_some() {
            self.batch_report_event(&Event::Key(key));
            return Ok(false);
        }
        if self.export_dialog_open() {
            self.export_event(&Event::Key(key));
            return Ok(false);
        }
        if self.diagnostics_open {
            let area = ratatui::layout::Rect::new(0, 0, cols, rows);
            ui::clamp_diagnostics_scroll(self, area);
            match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('i' | 'q') => {
                    self.diagnostics_open = false
                }
                KeyCode::Up => self.diagnostics_offset = self.diagnostics_offset.saturating_sub(1),
                KeyCode::Down => {
                    self.diagnostics_offset = self.diagnostics_offset.saturating_add(1)
                }
                KeyCode::Home => self.diagnostics_offset = 0,
                _ => {}
            }
            ui::clamp_diagnostics_scroll(self, area);
            return Ok(false);
        }
        if !self.console_ui_owned()
            && self
                .selection
                .as_ref()
                .is_some_and(|selection| selection.keyboard)
        {
            let delta = match key.code {
                KeyCode::Left => Some((-1, 0)),
                KeyCode::Right => Some((1, 0)),
                KeyCode::Up => Some((0, -1)),
                KeyCode::Down => Some((0, 1)),
                KeyCode::Home => Some((-i16::MAX, 0)),
                KeyCode::End => Some((i16::MAX, 0)),
                _ => None,
            };
            if let Some((dx, dy)) = delta {
                if let Some(selection) = &mut self.selection {
                    selection.move_end(dx, dy, key.modifiers.contains(KeyModifiers::SHIFT));
                }
                return Ok(false);
            }
            if key.code == KeyCode::Enter {
                let text = self
                    .selection
                    .as_mut()
                    .and_then(selection::Selection::selected_text);
                if let Some(text) = text {
                    self.copy_selection(&text)?;
                }
                return Ok(false);
            }
        }
        if !self.console_ui_owned() && self.selection.take().is_some() && key.code == KeyCode::Esc {
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
        if shortcut == Some(Shortcut::SelectText)
            && !self.actions_open
            && !self.help
            && !self.console_ui_owned()
        {
            self.begin_text_selection(ratatui::layout::Rect::new(0, 0, cols, rows));
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
                    let reading_binding = self.review.as_ref().is_some_and(|view| {
                        !view.editing()
                            && !view.showing_source_details()
                            && matches!(key.code, KeyCode::Char('n' | 'N' | 'f' | 'r' | 'i' | '/'))
                    });
                    if self.console_ui_owned() && shortcut.is_none() && !reading_binding {
                        return Ok(false);
                    }
                    return self.key(key, cols, rows);
                }
            }
            return Ok(false);
        }
        if self.help {
            self.help = false;
            return Ok(false);
        }
        if self.showing_operation_details() {
            self.review_event(&Event::Key(key));
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
        if shortcut == Some(Shortcut::SearchConsole) {
            self.search_console();
            return Ok(false);
        }
        if matches!(
            shortcut,
            Some(Shortcut::PreviousConsole | Shortcut::NextConsole)
        ) {
            self.cycle_run(shortcut == Some(Shortcut::PreviousConsole));
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
        // Explicit navigation above remains global. Everything else belongs
        // to snapshot review before process controls, selection or PTY fallback.
        if self.review_event(&Event::Key(key)) {
            return Ok(false);
        }
        if shortcut == Some(Shortcut::StopAll) {
            self.stop_all();
            return Ok(false);
        }
        if shortcut == Some(Shortcut::Animations) {
            if self.page == Page::Console {
                self.toggle_animations();
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
            self.set_config_selection(0);
            return Ok(false);
        }
        let count = if self.page == Page::Devices {
            self.visible_devices().len()
        } else {
            self.visible_entries().len()
        };
        if self.page == Page::Configs
            && matches!(
                key.code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Home
                    | KeyCode::End
                    | KeyCode::Char('j' | 'k')
            )
        {
            let absent = self.config_index >= count;
            let index = if absent {
                if matches!(key.code, KeyCode::Up | KeyCode::End | KeyCode::Char('k')) {
                    count.saturating_sub(1)
                } else {
                    0
                }
            } else {
                self.config_index
            };
            self.set_config_selection(index);
            if absent {
                return Ok(false);
            }
        }
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
            KeyCode::Char('d') if self.page == Page::Devices => self.open_diagnostics_picker(),
            KeyCode::Char('u') if self.page == Page::Configs => {
                self.show_unavailable = !self.show_unavailable;
                self.reveal_unavailable_after_check = false;
                self.set_config_selection(0);
                self.config_list_offset = 0;
            }
            KeyCode::Char('i') if self.page == Page::Configs => {
                self.diagnostics_open = true;
                self.diagnostics_offset = 0;
            }
            KeyCode::Char('r') if self.page == Page::Devices => self.discovery.refresh(),
            KeyCode::Char('r' | 'c') => self.request_config_reload(),
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
                self.set_config_selection(0);
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
                    self.advance_availability();
                    self.reveal_unavailable_after_check = true;
                    let (ready, _, pending) = self.availability_counts();
                    if ready == 0 && pending == 0 {
                        self.show_unavailable = true;
                        self.reveal_unavailable_after_check = false;
                    }
                    let configs = self.visible_entries();
                    let index = self
                        .saved
                        .last_config
                        .as_ref()
                        .and_then(|name| configs.iter().position(|c| &c.name == name))
                        .unwrap_or(0);
                    drop(configs);
                    self.set_config_selection(index);
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
        if self.batch_report.is_some() {
            return;
        }
        if self.export_dialog_open() {
            if !self.quit_confirmation {
                // Keep an oversize sentinel; never allocate an unbounded second
                // clipboard copy merely to reject it in the modal path editor.
                let bounded = text
                    .chars()
                    .take(log_export::MAX_DESTINATION_BYTES + 1)
                    .collect();
                self.export_event(&Event::Paste(bounded));
            }
            return;
        }
        self.selection = None;
        if self.quit_confirmation
            || self.help
            || self.diagnostics_open
            || self.actions_open
            || self.top_focus.is_some()
            || self.showing_operation_details()
        {
            return;
        }
        if self.reviewing() {
            let bounded: String = text
                .chars()
                .take(flimru::console_snapshot::MAX_QUERY_CHARS + 1)
                .collect();
            self.review_event(&Event::Paste(bounded));
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
            self.set_config_selection(0);
        }
    }

    fn copy_selection(&mut self, text: &str) -> Result<()> {
        match self.clipboard.copy(text) {
            Ok(None) => self.status = "Copying selection…".into(),
            Ok(Some(text)) => {
                execute!(
                    std::io::stdout(),
                    crossterm::clipboard::CopyToClipboard::to_clipboard_from(text)
                )?;
                self.status =
                    "Copy requested. Terminal clipboard permission may be required.".into();
            }
            Err(error) => self.status = format!("Copy failed: {error:#}"),
        }
        Ok(())
    }

    fn begin_text_selection(&mut self, area: ratatui::layout::Rect) {
        if self.console_ui_owned() {
            return;
        }
        self.prepare_last_log(area.height.saturating_sub(9).max(1));
        if area.width < 34 || area.height < 12 {
            return;
        }
        if let Some((screen, offset)) = self.selection_screen() {
            let content = ui::console_content_area(area);
            let start_row = ui::console_start_row(screen, offset, content.height);
            self.selection = selection::Selection::capture(
                screen,
                start_row,
                content,
                (content.x, content.y).into(),
            );
            if let Some(selection) = &mut self.selection {
                selection.enable_keyboard();
            }
            self.top_focus = None;
        }
    }

    fn selection_screen(&self) -> Option<(&vt100::Screen, Option<usize>)> {
        match self.page {
            Page::Console => self.runs.get(self.active_run).and_then(|run| {
                run.screen()
                    .map(|screen| (screen, run.outcome.as_ref().map(|_| run.offset)))
            }),
            Page::LastLog => self
                .last_log
                .as_ref()
                .map(|parser| (parser.screen(), Some(self.last_log_offset))),
            _ => None,
        }
    }

    fn mouse(&mut self, event: MouseEvent, area: ratatui::layout::Rect) -> Result<bool> {
        if self.quit_confirmation {
            if let Some(action @ (ui::MouseAction::ConfirmQuit | ui::MouseAction::CancelQuit)) =
                ui::mouse_action(self, area, event)
            {
                return self.activate(action, area);
            }
            return Ok(false);
        }
        if self.batch_report.is_some() {
            if ui::mouse_action(self, area, event) == Some(ui::MouseAction::Quit)
                && self
                    .batch_report
                    .as_ref()
                    .is_some_and(|report| !report.covers((event.column, event.row).into()))
            {
                self.top_focus = None;
                return self.activate(ui::MouseAction::Quit, area);
            }
            self.batch_report_event(&Event::Mouse(event));
            return Ok(false);
        }
        if self.export_dialog_open() {
            // Quit is application-wide, but an obscured toolbar cell still
            // belongs to the export modal. All other background actions stay
            // consumed, including ordinary toolbar controls.
            if ui::mouse_action(self, area, event) == Some(ui::MouseAction::Quit)
                && self
                    .pending_export
                    .as_ref()
                    .and_then(|pending| pending.dialog.as_ref())
                    .is_some_and(|dialog| !dialog.covers((event.column, event.row).into()))
            {
                self.top_focus = None;
                return self.activate(ui::MouseAction::Quit, area);
            }
            self.export_event(&Event::Mouse(event));
            return Ok(false);
        }
        if self.showing_operation_details()
            && !self.actions_open
            && !self.help
            && !self.quit_confirmation
        {
            // Details own the body, while the supported-size header Quit
            // remains visible. Other covered/background controls stay inert.
            if area.width >= 34
                && area.height >= 12
                && !ui::review_area(area).contains((event.column, event.row).into())
                && ui::mouse_action(self, area, event) == Some(ui::MouseAction::Quit)
            {
                self.top_focus = None;
                return self.activate(ui::MouseAction::Quit, area);
            }
            self.review_event(&Event::Mouse(event));
            return Ok(false);
        }
        if self.diagnostics_open {
            ui::clamp_diagnostics_scroll(self, area);
            match event.kind {
                MouseEventKind::ScrollDown => {
                    self.diagnostics_offset = self.diagnostics_offset.saturating_add(1)
                }
                MouseEventKind::ScrollUp => {
                    self.diagnostics_offset = self.diagnostics_offset.saturating_sub(1)
                }
                MouseEventKind::Down(MouseButton::Left) => self.diagnostics_open = false,
                _ => {}
            }
            ui::clamp_diagnostics_scroll(self, area);
            return Ok(false);
        }
        if self.reviewing() && !self.actions_open && !self.help && !self.quit_confirmation {
            let small = area.width < 34 || area.height < 12;
            if small || event.row < area.y.saturating_add(3) {
                if matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
                    && let Some(action) = ui::mouse_action(self, area, event)
                {
                    self.top_focus = None;
                    return self.activate(action, area);
                }
                if !small
                    && matches!(
                        event.kind,
                        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                    )
                {
                    let direction = if event.kind == MouseEventKind::ScrollUp {
                        -1
                    } else {
                        1
                    };
                    if self.top_focus.is_none() {
                        ui::focus_top(self, false);
                    } else {
                        ui::step_top(self, direction);
                    }
                }
            } else {
                if matches!(event.kind, MouseEventKind::Down(MouseButton::Left)) {
                    self.top_focus = None;
                }
                self.review_event(&Event::Mouse(event));
            }
            return Ok(false);
        }
        if self
            .selection
            .as_ref()
            .is_some_and(|selection| selection.viewport != ui::console_content_area(area))
        {
            self.selection = None;
        }
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
            let was_selecting = self
                .selection
                .as_ref()
                .is_some_and(|selection| selection.dragging);
            let text = self
                .selection
                .as_mut()
                .filter(|selection| selection.dragging)
                .and_then(|selection| selection.finish((event.column, event.row).into()));
            if let Some(text) = text {
                self.copy_selection(&text)?;
            }
            if was_selecting
                && self
                    .selection
                    .as_ref()
                    .is_some_and(|selection| !selection.has_range())
            {
                self.selection = None;
            }
            self.dragging_scrollbar = None;
            self.action_scroll_drag = false;
            return Ok(false);
        }
        if !self.quit_confirmation && !self.help {
            if !self.actions_open
                && matches!(event.kind, MouseEventKind::Drag(MouseButton::Left))
                && let Some(selection) = &mut self.selection
                && selection.dragging
            {
                selection.update((event.column, event.row).into());
                return Ok(false);
            }
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
                if !self.actions_open
                    && let Some(selection) = &mut self.selection
                    && selection.dragging
                {
                    selection.scroll(direction * 3);
                    return Ok(false);
                }
                self.selection = None;
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
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Left)) {
            self.selection = None;
            if !self.actions_open
                && !self.help
                && !self.quit_confirmation
                && area.width >= 34
                && area.height >= 12
                && let Some((screen, offset)) = self.selection_screen()
            {
                let content = ui::console_content_area(area);
                let start_row = ui::console_start_row(screen, offset, content.height);
                self.selection = selection::Selection::capture_history(
                    screen,
                    start_row,
                    content,
                    (event.column, event.row).into(),
                );
                if self.selection.is_some() {
                    self.dragging_scrollbar = None;
                    return Ok(false);
                }
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
        self.selection = None;
        let focus = self.top_focus.take();
        let page = self.page;
        let result = self.dispatch(action, area);
        if self.top_focus.is_none()
            && self.page == page
            && !self.searching
            && !matches!(
                action,
                ui::MouseAction::CloseConsole(_)
                    | ui::MouseAction::CloseLastLog
                    | ui::MouseAction::SelectText
                    | ui::MouseAction::SearchConsole
                    | ui::MouseAction::EditSearchQuery
                    | ui::MouseAction::NextSearchMatch
                    | ui::MouseAction::PreviousSearchMatch
                    | ui::MouseAction::CycleSearchFilter
                    | ui::MouseAction::SearchFilter(_)
                    | ui::MouseAction::RefreshSnapshot
                    | ui::MouseAction::ResumeLive
                    | ui::MouseAction::SnapshotSource
                    | ui::MouseAction::ExportLog
            )
        {
            self.top_focus = focus;
        }
        result
    }

    fn dispatch(&mut self, action: ui::MouseAction, area: ratatui::layout::Rect) -> Result<bool> {
        use ui::MouseAction;
        if matches!(
            action,
            MouseAction::PreviousConsole
                | MouseAction::NextConsole
                | MouseAction::Devices
                | MouseAction::AddSession
                | MouseAction::SwitchConsole(_)
                | MouseAction::CloseConsole(_)
                | MouseAction::CloseLastLog
                | MouseAction::LastLog
                | MouseAction::SelectDevice(_)
                | MouseAction::SelectConfig(_)
                | MouseAction::ToggleDevice(_)
                | MouseAction::ToggleFavorite(_)
                | MouseAction::ToggleMulti
                | MouseAction::NextOrRun
                | MouseAction::Back
                | MouseAction::Refresh
                | MouseAction::ReloadConfigs
                | MouseAction::ConfigurationDiagnostics
                | MouseAction::ToggleUnavailable
                | MouseAction::ExplainConfiguration
                | MouseAction::Filter
                | MouseAction::ApplyFilter
                | MouseAction::ClearFilter
        ) && let Some(view) = &mut self.review
        {
            view.dismiss_operation_details();
        }
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
                self.cycle_run(true);
                return Ok(false);
            }
            MouseAction::NextConsole => {
                self.cycle_run(false);
                return Ok(false);
            }
            MouseAction::Devices => {
                self.retain_consoles |= !self.runs.is_empty();
                self.back();
                return Ok(false);
            }
            MouseAction::AddSession => {
                self.begin_add();
                return Ok(false);
            }
            MouseAction::StopAll => {
                self.stop_all();
                return Ok(false);
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
                self.set_config_selection(0);
                return Ok(false);
            }
            MouseAction::FlutterKey(key) => {
                self.send_flutter_key(key);
                return Ok(false);
            }
            MouseAction::BatchHotCommand(command) => {
                self.request_hot_command(command);
                return Ok(false);
            }
            MouseAction::SearchConsole => {
                self.search_console();
                return Ok(false);
            }
            MouseAction::ExportLog => {
                self.open_export();
                return Ok(false);
            }
            MouseAction::RefreshSnapshot => {
                self.capture_review(true);
                return Ok(false);
            }
            MouseAction::ResumeLive => {
                self.close_review();
                return Ok(false);
            }
            MouseAction::EditSearchQuery
            | MouseAction::NextSearchMatch
            | MouseAction::PreviousSearchMatch
            | MouseAction::CycleSearchFilter
            | MouseAction::SearchFilter(_)
            | MouseAction::SnapshotSource => {
                if let Some(view) = &mut self.review {
                    match action {
                        MouseAction::EditSearchQuery => view.edit_query(),
                        MouseAction::NextSearchMatch => view.next_match(false),
                        MouseAction::PreviousSearchMatch => view.previous_match(),
                        MouseAction::CycleSearchFilter => view.cycle_filter(),
                        MouseAction::SearchFilter(filter) => view.set_filter(filter),
                        MouseAction::SnapshotSource => view.show_source_details(),
                        _ => {}
                    }
                }
                return Ok(false);
            }
            MouseAction::SelectText => {
                self.actions_open = false;
                self.top_focus = None;
                self.begin_text_selection(area);
                return Ok(false);
            }
            MouseAction::OldestOutput if !self.console_ui_owned() => KeyCode::Home,
            MouseAction::FollowOutput if !self.console_ui_owned() => KeyCode::End,
            MouseAction::OldestOutput | MouseAction::FollowOutput => return Ok(false),
            MouseAction::SelectDevice(index) => {
                self.searching = false;
                self.device_index = index;
                return Ok(false);
            }
            MouseAction::SelectConfig(index) => {
                self.searching = false;
                self.set_config_selection(index);
                return Ok(false);
            }
            MouseAction::ToggleFavorite(index) => {
                self.searching = false;
                self.device_index = index;
                KeyCode::Char('f')
            }
            MouseAction::ConfigurationDiagnostics => {
                self.actions_open = false;
                self.open_diagnostics_picker();
                return Ok(false);
            }
            MouseAction::ToggleUnavailable => KeyCode::Char('u'),
            MouseAction::ExplainConfiguration => KeyCode::Char('i'),
            MouseAction::NextOrRun => {
                self.searching = false;
                KeyCode::Enter
            }
            MouseAction::Back if self.page == Page::Console => {
                if let Some(session) = self.active_session_mut() {
                    let forced = session.stop_requested.is_some();
                    let result = session.request_stop();
                    self.report_operation(match result {
                        Ok(()) => if forced {
                            "Forced stop requested"
                        } else {
                            "Stop requested"
                        }
                        .into(),
                        Err(error) => format!("Stop: {error:#}"),
                    });
                } else {
                    self.back();
                }
                return Ok(false);
            }
            MouseAction::Back if self.page == Page::LastLog => {
                self.back();
                return Ok(false);
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
                self.request_config_reload();
                return Ok(false);
            }
            MouseAction::LastLog => {
                if self.last_log.is_some() {
                    if self.page != Page::LastLog {
                        self.close_review();
                    }
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
            MouseAction::OperationDetails => {
                self.actions_open = false;
                self.top_focus = None;
                self.selection = None;
                if let Some(message) = &self.last_operation {
                    if self.review.is_none() {
                        match console_search::SearchView::new(console_style::review_severity) {
                            Ok(view) => self.review = Some(view),
                            Err(error) => {
                                self.status = format!("Cannot show operation details: {error}");
                                return Ok(false);
                            }
                        }
                    }
                    if let Some(view) = &mut self.review {
                        view.show_operation_details(message);
                    }
                }
                return Ok(false);
            }
            MouseAction::ToggleAnimations => {
                self.toggle_animations();
                return Ok(false);
            }
            MouseAction::ReconnectAnimations => {
                if let Some(session) = self.active_session_mut() {
                    let message = match session.reconnect_animations() {
                        Ok(()) => "VM Service reconnect requested.".into(),
                        Err(error) => format!("Animation reconnect: {error:#}"),
                    };
                    self.report_operation(message);
                }
                return Ok(false);
            }
            MouseAction::ConsoleScroll(row) => {
                if self.console_ui_owned() {
                    return Ok(false);
                }
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
        if self.console_ui_owned() {
            return;
        }
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
            selection: None,
            clipboard: clipboard::Clipboard::default(),
            actions_open: false,
            action_index: 0,
            action_offset: 0,
            action_scroll_drag: false,
            top_focus: None,
            toolbar_offset: 0,
            launch: Arc::new(launch),
            config_reloader: None,
            reload_generation: 0,
            manual_reload_pending: None,
            reload_input_drain_pending: false,
            reload_error: None,
            availability_view: Default::default(),
            tool,
            discovery,
            saved: State::default(),
            state_path: workspace.join("unused-state.json"),
            persistence_enabled: false,
            discovered: vec![],
            devices: vec![],
            device_index: 0,
            config_index: 0,
            show_unavailable: false,
            reveal_unavailable_after_check: false,
            diagnostics_open: false,
            diagnostics_offset: 0,
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
            review: None,
            review_notice: None,
            batch_report: None,
            pending_export: None,
            last_operation: None,
            last_log: None,
            last_log_source: None,
            last_log_max_columns: 1,
            last_log_history_capacity: Session::SCROLLBACK_CAPACITY,
            last_log_scrollback: 0,
            last_log_offset: 0,
            last_log_view_rows: 0,
            active_config: String::new(),
            help: false,
            quit_confirmation: false,
            dragging_scrollbar: None,
        };
        app.merge();
        app.settle_availability();
        app
    }

    fn reload_test_app(raw: &str) -> App {
        let mut app = app();
        app.ownit = false;
        app.launch =
            Arc::new(LaunchFile::parse(raw, &app.launch.path, &app.launch.workspace).unwrap());
        app.selected_device = Some(app.devices[0].clone());
        app.page = Page::Configs;
        app.merge();
        app.settle_availability();
        app
    }

    fn catalog_result(app: &App, raw: &str, generation: u64, source: ReloadSource) -> ReloadResult {
        ReloadResult {
            generation,
            source,
            result: LaunchFile::parse(raw, &app.launch.path, &app.launch.workspace)
                .map_err(|_| ReloadError::InvalidConfiguration),
        }
    }

    #[test]
    fn reload_preserves_only_unambiguous_names_and_content_in_both_catalogs() {
        let cases = [
            (
                r#"[{"name":"Alpha"},{"name":"Beta"}]"#,
                1,
                r#"[{"name":"Beta","args":["--verbose"]},{"name":"Alpha"}]"#,
                Some(0),
            ),
            (
                r#"[{"name":"Same","args":["a"]},{"name":"Same","args":["b"]}]"#,
                1,
                r#"[{"name":"Same","args":["b"]},{"name":"Same","args":["a"]}]"#,
                Some(0),
            ),
            (
                r#"[{"name":"Same","args":["a"]},{"name":"Same","args":["b"]}]"#,
                1,
                r#"[{"name":"Same","args":["a"]}]"#,
                None,
            ),
            (
                r#"[{"name":"Same"},{"name":"Same"}]"#,
                1,
                r#"[{"name":"Same"}]"#,
                None,
            ),
            (
                r#"[{"name":"Same"}]"#,
                0,
                r#"[{"name":"Same"},{"name":"Same"}]"#,
                None,
            ),
            (r#"[{"name":"Gone"}]"#, 0, r#"[{"name":"Other"}]"#, None),
        ];
        for (old, selected, new, expected) in cases {
            let mut app = reload_test_app(&format!(r#"{{"configurations":{old}}}"#));
            app.set_config_source_selection(Some(selected));
            let result = catalog_result(
                &app,
                &format!(r#"{{"configurations":{new}}}"#),
                1,
                ReloadSource::Automatic,
            );
            assert!(app.apply_config_reload(result));
            app.settle_availability();
            assert_eq!(app.selected_config_source(), expected, "{old} -> {new}");
            app.merge();
            assert_eq!(app.selected_config_source(), expected);
            if expected.is_none() {
                assert_eq!(app.config_index, usize::MAX);
                app.start_batch(80, 24);
                assert!(app.runs.is_empty());
                assert_eq!(app.status, "Select a configuration first.");
            }
        }
    }

    #[test]
    fn reload_captures_deferred_selection_and_preserves_explicitly_cleared_selection() {
        let mut app =
            reload_test_app(r#"{"configurations":[{"name":"First"},{"name":"Selected"}]}"#);
        app.set_config_selection(1);
        app.selected_device
            .as_mut()
            .unwrap()
            .name
            .push_str(" updated");
        app.ensure_availability_context();
        app.restore_deferred_config_selection();
        assert_eq!(app.config_index, usize::MAX);
        assert_eq!(app.selected_config_source(), Some(1));
        let result = catalog_result(
            &app,
            r#"{"configurations":[{"name":"Selected"},{"name":"First"}]}"#,
            1,
            ReloadSource::Automatic,
        );
        app.apply_config_reload(result);
        assert_eq!(app.selected_config_source(), Some(0));
        app.settle_availability();
        assert_eq!(app.visible_entries()[app.config_index].name, "Selected");
        app.set_config_source_selection(None);
        let result = catalog_result(
            &app,
            r#"{"configurations":[{"name":"Selected","args":["--verbose"]},{"name":"First"}]}"#,
            2,
            ReloadSource::Automatic,
        );
        app.apply_config_reload(result);
        app.settle_availability();
        app.merge();
        assert_eq!(app.selected_config_source(), None);
        assert_eq!(app.config_index, usize::MAX);
    }

    #[test]
    fn reload_last_good_errors_stale_results_and_semantic_duplicate_recovery() {
        let raw = r#"{"configurations":[{"name":"Stable"}]}"#;
        let mut app = reload_test_app(raw);
        app.status = "Unrelated status".into();
        let original = app.launch.clone();
        let availability_generation = app.availability_view.borrow().generation;
        assert!(!app.apply_config_reload(catalog_result(&app, raw, 1, ReloadSource::Automatic)));
        assert!(Arc::ptr_eq(&original, &app.launch));
        assert_eq!(
            app.availability_view.borrow().generation,
            availability_generation
        );
        assert!(app.apply_config_reload(ReloadResult {
            generation: 2,
            source: ReloadSource::Automatic,
            result: Err(ReloadError::Missing)
        }));
        assert_eq!(app.status, "Unrelated status");
        assert!(Arc::ptr_eq(&original, &app.launch));
        assert!(!app.apply_config_reload(ReloadResult {
            generation: 3,
            source: ReloadSource::Automatic,
            result: Err(ReloadError::Missing)
        }));
        assert!(
            app.diagnostics_lines()
                .join("\n")
                .contains("last good catalog")
        );
        app.reload_generation = 5;
        app.manual_reload_pending = Some(5);
        assert!(!app.apply_config_reload(catalog_result(
            &app,
            r#"{"configurations":[]}"#,
            4,
            ReloadSource::Manual
        )));
        assert_eq!(app.manual_reload_pending, Some(5));
        assert_eq!(app.reload_error, Some(ReloadError::Missing));
        // An automatic completion may coalesce an unread, older manual result.
        assert!(app.apply_config_reload(catalog_result(&app, raw, 6, ReloadSource::Automatic)));
        assert_eq!(app.manual_reload_pending, None);
        assert_eq!(app.reload_error, None);
        assert!(!Arc::ptr_eq(&original, &app.launch));
        app.settle_availability();
        assert!(app.availability_view.borrow().generation > availability_generation);
        assert_eq!(app.selected_config_source(), Some(0));
        assert_eq!(app.status, "Launch configurations reloaded.");
        app.status = "Keep on automatic recovery".into();
        app.apply_config_reload(ReloadResult {
            generation: 7,
            source: ReloadSource::Automatic,
            result: Err(ReloadError::InvalidConfiguration),
        });
        let before_recovery = app.launch.clone();
        assert!(app.apply_config_reload(catalog_result(&app, raw, 8, ReloadSource::Automatic)));
        assert!(!Arc::ptr_eq(&before_recovery, &app.launch));
        assert_eq!(app.reload_error, None);
        assert_eq!(app.status, "Keep on automatic recovery");
        app.apply_config_reload(ReloadResult {
            generation: 9,
            source: ReloadSource::Manual,
            result: Err(ReloadError::Missing),
        });
        assert!(app.status.starts_with("Reload failed:"));
        app.apply_config_reload(catalog_result(&app, raw, 10, ReloadSource::Automatic));
        assert_eq!(app.status, "Launch configurations reloaded.");
    }

    #[test]
    fn unchanged_manual_reload_refreshes_external_metadata_and_gates_run_until_completion() {
        let fixture = tempfile::tempdir().unwrap();
        let cwd = fixture.path().join("created-later");
        let raw =
            serde_json::json!({"configurations":[{"name":"Metadata", "cwd":cwd}]}).to_string();
        let mut app = reload_test_app(&raw);
        app.show_unavailable = true;
        app.set_config_source_selection(Some(0));
        assert!(!app.assessment(&app.launch.entries[0]).runnable());
        std::fs::create_dir(&cwd).unwrap();
        app.manual_reload_pending = Some(10);
        app.reload_generation = 10;
        app.start_batch(80, 24);
        assert!(app.runs.is_empty());
        assert!(app.status.contains("Reloading configurations"));
        assert!(
            !ui::available_actions(&app)
                .iter()
                .any(|item| item.label == "Run configuration")
        );
        app.apply_config_reload(catalog_result(&app, &raw, 10, ReloadSource::Manual));
        app.settle_availability();
        assert!(app.assessment(&app.launch.entries[0]).runnable());
        assert!(app.runs.is_empty()); // Completion never queues an implicit Run.
        assert_eq!(app.selected_config_source(), Some(0));
        assert_eq!(app.status, "Launch configurations reloaded.");
        app.manual_reload_pending = Some(11);
        app.reload_generation = 11;
        app.apply_config_reload(ReloadResult {
            generation: 11,
            source: ReloadSource::Manual,
            result: Err(ReloadError::Missing),
        });
        assert_eq!(app.manual_reload_pending, None);
        assert_eq!(app.reload_error, Some(ReloadError::Missing));
    }

    #[test]
    fn manual_reload_requires_post_completion_input_idle_before_run_rearms() {
        let raw = r#"{"configurations":[{"name":"Ready one"},{"name":"Ready two"}]}"#;
        for fail in [false, true] {
            for source in [ReloadSource::Manual, ReloadSource::Automatic] {
                let mut app = reload_test_app(raw);
                app.manual_reload_pending = Some(10);
                app.reload_generation = 10;
                app.reload_input_drain_pending = true;
                assert!(!app.input_queue_idle()); // Idle while pending cannot re-arm.
                assert!(!app.apply_config_reload(catalog_result(&app, raw, 9, source)));
                assert!(app.reload_input_drain_pending);
                let result = if fail {
                    ReloadResult {
                        generation: 11,
                        source,
                        result: Err(ReloadError::InvalidConfiguration),
                    }
                } else {
                    catalog_result(&app, raw, 11, source)
                };
                assert!(app.apply_config_reload(result));
                app.settle_availability();
                assert_eq!(app.manual_reload_pending, None);
                assert!(app.reload_input_drain_pending);
                // Budget boundaries are deliberately not idle acknowledgements.
                for _ in 0..256 {
                    app.key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE), 80, 24)
                        .unwrap();
                }
                app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 80, 24)
                    .unwrap();
                assert!(app.runs.is_empty());
                assert!(app.status.contains("Finishing reload"));
                assert!(app.reload_input_drain_pending);

                // Other queued intent remains actionable while Run is guarded.
                app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 80, 24)
                    .unwrap();
                assert_eq!(app.selected_config_source(), Some(1));
                for code in [KeyCode::Char('/'), KeyCode::Char('R'), KeyCode::Enter] {
                    app.key(KeyEvent::new(code, KeyModifiers::NONE), 80, 24)
                        .unwrap();
                }
                assert_eq!(app.search.as_deref(), Some("R"));
                assert!(!app.searching);
                app.key(
                    KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
                    80,
                    24,
                )
                .unwrap();
                assert!(app.quit_confirmation);
                app.key(
                    KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
                    80,
                    24,
                )
                .unwrap();
                assert!(!app.quit_confirmation);
                assert!(app.reload_run_guard().is_some());
                assert!(
                    !ui::available_actions(&app)
                        .iter()
                        .any(|item| item.label == "Run configuration")
                );
                assert!(app.input_queue_idle()); // Explicit event-loop acknowledgement.
                assert_eq!(
                    app.status,
                    if fail {
                        format!("Reload failed: {}", ReloadError::InvalidConfiguration)
                    } else {
                        "Launch configurations reloaded.".into()
                    }
                );
                assert!(!app.input_queue_idle());
                assert!(app.reload_run_guard().is_none());
                assert!(
                    ui::available_actions(&app)
                        .iter()
                        .any(|item| item.label == "Run configuration")
                );
                assert!(app.runs.is_empty()); // Re-arming never schedules Run itself.
            }
        }
    }

    #[test]
    fn reload_accepts_empty_and_rejected_catalogs_without_changing_page_or_modal_state() {
        let mut app = reload_test_app(r#"{"configurations":[{"name":"Stable"}]}"#);
        app.page = Page::LastLog;
        app.search = Some("Sta".into());
        app.searching = true;
        app.actions_open = true;
        app.help = true;
        app.diagnostics_open = true;
        app.diagnostics_offset = 3;
        app.multi = true;
        app.marked_devices = vec![app.selected_device.clone().unwrap()];
        let marked_id = app.marked_devices[0].id.clone();
        app.saved.favorites.push(app.marked_devices[0].saved());
        app.status = "Keep this status".into();
        for (generation, raw) in [
            (1, r#"{"configurations":[]}"#),
            (
                2,
                r#"{"configurations":[{"name":"Rejected","type":"SECRET"}]}"#,
            ),
        ] {
            app.apply_config_reload(catalog_result(
                &app,
                raw,
                generation,
                ReloadSource::Automatic,
            ));
            app.settle_availability();
            assert_eq!(app.page, Page::LastLog);
            assert_eq!(app.search.as_deref(), Some("Sta"));
            assert!(app.searching && app.actions_open && app.help && app.diagnostics_open);
            assert_eq!(app.diagnostics_offset, 3);
            assert_eq!(app.marked_devices[0].id, marked_id);
            assert_eq!(app.saved.favorites.len(), 1);
            assert_eq!(app.status, "Keep this status");
            assert_eq!(app.selected_config_source(), None);
            assert_eq!(app.launch.entries.len(), generation as usize - 1);
        }
        assert_eq!(app.launch.configurations.len(), 0);
        assert_eq!(app.launch.entries[0].name, "Rejected");
        assert!(!app.diagnostics_lines().join("\n").contains("SECRET"));
    }

    #[test]
    fn manual_reload_keyboard_toolbar_and_menu_request_one_shared_async_policy() {
        let fixture = tempfile::tempdir().unwrap();
        let raw = r#"{"configurations":[{"name":"Ready"}]}"#;
        let path = fixture.path().join("launch.json");
        std::fs::write(&path, raw).unwrap();
        for route in ["r", "c", "device-c", "toolbar", "menu"] {
            let mut app = reload_test_app(raw);
            app.launch = Arc::new(LaunchFile::parse(raw, &path, fixture.path()).unwrap());
            app.merge();
            app.settle_availability();
            let area = ratatui::layout::Rect::new(0, 0, 80, 24);
            match route {
                "toolbar" => {
                    app.activate(ui::MouseAction::ReloadConfigs, area).unwrap();
                }
                "menu" => {
                    app.actions_open = true;
                    app.action_index = ui::available_actions(&app)
                        .iter()
                        .position(|item| item.action == ui::MouseAction::ReloadConfigs)
                        .unwrap();
                    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 80, 24)
                        .unwrap();
                }
                key => {
                    if key == "device-c" {
                        app.page = Page::Devices;
                    }
                    app.key(
                        KeyEvent::new(
                            KeyCode::Char(if key == "r" { 'r' } else { 'c' }),
                            KeyModifiers::NONE,
                        ),
                        80,
                        24,
                    )
                    .unwrap();
                }
            }
            assert!(app.manual_reload_pending.is_some(), "{route}");
            assert_eq!(app.status, "Reloading launch configurations…");
            if app.page == Page::Configs {
                app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 80, 24)
                    .unwrap();
                assert!(app.runs.is_empty(), "{route}");
            }
            let deadline = Instant::now() + Duration::from_secs(3);
            while app.manual_reload_pending.is_some() {
                assert!(Instant::now() < deadline, "{route}");
                app.advance_config_reload();
                std::thread::sleep(Duration::from_millis(1));
            }
            app.settle_availability();
            assert_eq!(app.reload_error, None, "{route}");
            assert_eq!(app.status, "Launch configurations reloaded.");
            assert!(app.runs.is_empty());
        }
    }

    #[test]
    #[cfg(unix)]
    fn reload_keeps_owned_live_session_and_selection_snapshot_while_output_drains() {
        let mut app = reload_test_app(r#"{"configurations":[{"name":"Live original"}]}"#);
        let session = Session::start(
            &flimru::tool::RunCommand {
                capture_vm_service: false,
                hot_capabilities: flimru::batch_commands::HotCapabilities::Supported,
                executable: "/bin/sh".into(),
                args: vec![
                    "-c".into(),
                    "while :; do printf 'reload-heartbeat\\r\\n'; sleep 0.01; done".into(),
                ],
                cwd: app.launch.workspace.clone(),
                env: Default::default(),
            },
            15,
            77,
        )
        .unwrap();
        app.runs.push(runs::Run::running(
            app.selected_device.clone().unwrap(),
            "Launch-time original".into(),
            session,
            flimru::batch_commands::HotCapabilities::Supported,
        ));
        app.page = Page::Console;
        app.active_config = "Launch-time original".into();
        let identity = std::ptr::from_ref(app.runs[0].session.as_ref().unwrap());
        let deadline = Instant::now() + Duration::from_secs(3);
        while !app.runs[0]
            .screen()
            .unwrap()
            .contents()
            .contains("reload-heartbeat")
        {
            assert!(Instant::now() < deadline);
            app.tick().unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
        app.begin_text_selection(ratatui::layout::Rect::new(0, 0, 80, 24));
        assert!(app.selection.is_some());
        app.apply_config_reload(catalog_result(
            &app,
            r#"{"configurations":[{"name":"Future","deviceId":"new-pinned"}]}"#,
            1,
            ReloadSource::Automatic,
        ));
        assert_eq!(app.page, Page::Console);
        assert_eq!(app.active_run, 0);
        assert_eq!(app.active_config, "Launch-time original");
        assert_eq!(app.runs[0].config, "Launch-time original");
        assert_eq!(
            std::ptr::from_ref(app.runs[0].session.as_ref().unwrap()),
            identity
        );
        assert!(app.selection.is_some());
        assert!(app.devices.iter().any(|device| device.id == "new-pinned"));
        let revision = app.runs[0].session.as_ref().unwrap().visual_revision;
        let deadline = Instant::now() + Duration::from_secs(3);
        while app.runs[0].session.as_ref().unwrap().visual_revision == revision {
            assert!(Instant::now() < deadline);
            app.tick().unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(app.running_count(), 1);
    }

    #[test]
    #[cfg(unix)]
    fn reload_large_catalog_apply_stays_within_the_interactive_budget() {
        let raw = format!(
            "{{\"configurations\":[{}]}}",
            (0..100_000)
                .map(|index| format!("{{\"name\":\"Config {index}\"}}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(raw.len() < config_reload::MAX_LAUNCH_FILE_BYTES);
        let mut app = reload_test_app(&raw);
        let session = Session::start(
            &flimru::tool::RunCommand {
                capture_vm_service: false,
                hot_capabilities: flimru::batch_commands::HotCapabilities::Supported,
                executable: "/bin/sh".into(),
                args: vec![
                    "-c".into(),
                    "while :; do printf 'apply-heartbeat\\r\\n'; sleep 0.01; done".into(),
                ],
                cwd: app.launch.workspace.clone(),
                env: Default::default(),
            },
            15,
            77,
        )
        .unwrap();
        app.runs.push(runs::Run::running(
            app.selected_device.clone().unwrap(),
            "Unchanged live child".into(),
            session,
            flimru::batch_commands::HotCapabilities::Supported,
        ));
        app.set_config_source_selection(Some(99_999));
        let duplicate = catalog_result(&app, &raw, 1, ReloadSource::Automatic);
        let started = Instant::now();
        assert!(!app.apply_config_reload(duplicate));
        let duplicate_time = started.elapsed();
        let reversed = format!(
            "{{\"configurations\":[{}]}}",
            (0..100_000)
                .rev()
                .map(|index| format!("{{\"name\":\"Config {index}\"}}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        let mut changed =
            LaunchFile::parse(&reversed, &app.launch.path, &app.launch.workspace).unwrap();
        changed.configurations[0].args.push("--verbose".into());
        let started = Instant::now();
        assert!(app.apply_config_reload(ReloadResult {
            generation: 2,
            source: ReloadSource::Automatic,
            result: Ok(changed)
        }));
        let changed_time = started.elapsed();
        assert_eq!(app.selected_config_source(), Some(0));
        let started = Instant::now();
        app.tick().unwrap();
        let drain_time = started.elapsed();
        assert!(
            app.runs[0]
                .screen()
                .unwrap()
                .contents()
                .contains("apply-heartbeat")
        );
        assert_eq!(app.runs[0].config, "Unchanged live child");
        assert_eq!(app.running_count(), 1);
        eprintln!(
            "P2 100000-entry apply: semantic duplicate {duplicate_time:?}; reversed/changed catalog + identity/merge {changed_time:?}; following heartbeat-draining tick {drain_time:?}"
        );
        assert!(duplicate_time < Duration::from_millis(100));
        assert!(changed_time < Duration::from_millis(100));
        assert!(drain_time < Duration::from_millis(100));
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
        app.launch = Arc::new(
            LaunchFile::parse(
                r#"{"configurations":[{"name":"Shared"}]}"#,
                &workspace.join("launch.json"),
                &workspace,
            )
            .unwrap(),
        );
        let device = app.devices[0].clone();
        let sleeper = |device: Device, name: &str| {
            runs::Run::running(
                device,
                name.into(),
                Session::start(
                    &flimru::tool::RunCommand {
                        capture_vm_service: false,
                        hot_capabilities: flimru::batch_commands::HotCapabilities::Supported,
                        executable: "/bin/sh".into(),
                        args: vec!["-c".into(), "exec sleep 30".into()],
                        cwd: workspace.clone(),
                        env: Default::default(),
                    },
                    5,
                    70,
                )
                .unwrap(),
                flimru::batch_commands::HotCapabilities::Supported,
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
            app.settle_availability();
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
        app.settle_availability();
        app.start_batch(80, 24);
        assert!(app.status.contains("0 session slot(s) free"));
        assert_eq!(app.running_count(), 8);
        app.runs.pop();
        app.marked_devices.push(device);
        app.settle_availability();
        app.start_batch(80, 24);
        assert!(app.status.contains("1 session slot(s) free"));
        assert_eq!(app.running_count(), 7);
        app.marked_devices = vec![app.runs[0].device.clone()];
        app.settle_availability();
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
                capture_vm_service: false,
                hot_capabilities: flimru::batch_commands::HotCapabilities::Supported,
                executable: "/bin/sh".into(),
                args: vec!["-c".into(), "exit 0".into()],
                cwd: app.launch.workspace.clone(),
                env: Default::default(),
            },
            5,
            70,
        )
        .unwrap();
        app.runs.push(runs::Run::running(
            device,
            "Original".into(),
            session,
            flimru::batch_commands::HotCapabilities::Supported,
        ));
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
        app.settle_availability();
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
        app.settle_availability();
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
        app.launch = Arc::new(
            LaunchFile::parse(
                r#"{"configurations":[{"name":"Invalid task","preLaunchTask":"unsupported"}]}"#,
                &workspace.join("launch.json"),
                &workspace,
            )
            .unwrap(),
        );
        app.runs.push(runs::Run::failed(
            device,
            "Previous".into(),
            "retained".into(),
            5,
            70,
        ));
        app.show_unavailable = true;
        app.settle_availability();
        app.start_batch(80, 24);
        assert!(app.diagnostics_open);
        assert_eq!(app.runs.len(), 1);
        assert_eq!(app.runs[0].config, "Previous");
        app.diagnostics_open = false;
        Arc::make_mut(&mut app.launch).configurations[0].pre_launch_task = None;
        app.settle_availability();
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
        app.settle_availability();
        let names: Vec<_> = app
            .visible_entries()
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, vec!["Integration tests"]);
        app.marked_devices.pop();
        app.settle_availability();
        assert!(app.visible_entries().iter().any(|c| c.name == "Ordinary"));
        app.marked_devices.clear();
        app.settle_availability();
        assert!(app.visible_entries().is_empty());
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
        app.settle_availability();
        assert!(!app.visible_entries().iter().any(|c| c.name == "Solana"));
        app.discovered = vec![Device {
            id: "android-device".into(),
            name: "Pixel".into(),
            platform: "android-arm64".into(),
            available: true,
            favorite: false,
        }];
        app.merge();
        app.settle_availability();
        assert_eq!(
            app.selected_device.as_ref().unwrap().platform,
            "android-arm64"
        );
        assert!(app.visible_entries().iter().any(|c| c.name == "Solana"));
        assert_eq!(app.visible_entries()[app.config_index].name, "Ordinary");
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
        app.settle_availability();
        assert_eq!(
            app.visible_entries()
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
                assert!(!app.help && !app.actions_open);
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
    fn filter_commands_agree_between_keyboard_and_pointer_actions() {
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
    #[test]
    fn text_selection_freezes_only_the_view_and_clears_without_leaving_console() {
        use ratatui::{Terminal, backend::TestBackend, layout::Rect};
        let mut app = app();
        let mut parser = vt100::Parser::new(23, 109, 1000);
        parser.process(b"alpha text\r\nbeta");
        app.last_log = Some(parser);
        app.page = Page::LastLog;
        let area = Rect::new(0, 0, 112, 32);
        let content = ui::console_content_area(area);
        let event = |kind, x, y| MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        };
        app.mouse(
            event(
                MouseEventKind::Down(MouseButton::Left),
                content.x,
                content.y,
            ),
            area,
        )
        .unwrap();
        app.mouse(
            event(
                MouseEventKind::Drag(MouseButton::Left),
                content.x + 4,
                content.y,
            ),
            area,
        )
        .unwrap();
        app.last_log.as_mut().unwrap().process(b"\x1b[Hnew output");
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
        assert_eq!(
            terminal.backend().buffer()[(content.x, content.y)].symbol(),
            "a"
        );
        assert_eq!(
            terminal.backend().buffer()[(content.x, content.y)].bg,
            ratatui::style::Color::Rgb(96, 210, 225)
        );
        app.key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            area.width,
            area.height,
        )
        .unwrap();
        assert!(app.selection.is_none());
        assert_eq!(app.page, Page::LastLog);
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
        assert_eq!(
            terminal.backend().buffer()[(content.x, content.y)].symbol(),
            "n"
        );
        app.mouse(
            event(
                MouseEventKind::Down(MouseButton::Left),
                content.x,
                content.y,
            ),
            area,
        )
        .unwrap();
        app.mouse(
            event(MouseEventKind::Up(MouseButton::Left), content.x, content.y),
            area,
        )
        .unwrap();
        assert!(app.selection.is_none()); // Plain clicks neither copy nor freeze.
        app.key(Shortcut::SelectText.key(), area.width, area.height)
            .unwrap();
        assert!(app.selection.as_ref().unwrap().keyboard);
        let track = ui::console_scrollbar_area(area);
        app.mouse(
            event(MouseEventKind::Down(MouseButton::Left), track.x, track.y),
            area,
        )
        .unwrap();
        assert!(app.selection.is_none());
        app.key(Shortcut::SelectText.key(), area.width, area.height)
            .unwrap();
        app.mouse(
            event(
                MouseEventKind::Drag(MouseButton::Left),
                content.x,
                content.y,
            ),
            Rect::new(0, 0, 80, 24),
        )
        .unwrap();
        assert!(app.selection.is_none()); // Resize invalidates the old text coordinates.
    }
    #[test]
    fn rejected_rows_block_keyboard_pointer_and_paste_and_diagnostics_need_no_device() {
        let mut app = app();
        app.ownit = false;
        let workspace = app.launch.workspace.clone();
        app.launch = Arc::new(
            LaunchFile::parse(
                r#"{"configurations":[
            {"name":"Rejected", "type":"SECRET_TYPE"},
            {"name":"Task", "preLaunchTask":"SECRET_TASK"},
            {"name":"Ready"}
        ]}"#,
                &workspace.join("launch.json"),
                &workspace,
            )
            .unwrap(),
        );
        app.selected_device = Some(app.devices[0].clone());
        app.page = Page::Configs;
        app.merge();
        app.settle_availability();
        assert_eq!(
            app.visible_entries()
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["Ready"]
        );
        app.key(
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE),
            34,
            12,
        )
        .unwrap();
        assert_eq!(app.visible_entries().len(), 3);
        let area = ratatui::layout::Rect::new(0, 0, 34, 12);
        for action in [None, Some(ui::MouseAction::NextOrRun)] {
            if let Some(action) = action {
                app.activate(action, area).unwrap();
            } else {
                app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 34, 12)
                    .unwrap();
            }
            assert!(app.diagnostics_open);
            assert!(app.runs.is_empty());
            assert!(
                !ui::available_actions(&app)
                    .iter()
                    .any(|item| item.label == "Run configuration")
            );
            app.paste("SHOULD_NOT_FILTER_OR_LAUNCH");
            assert!(app.search.is_none());
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 34, 12)
                .unwrap();
            assert!(!app.diagnostics_open);
            assert_eq!(app.page, Page::Configs);
            assert!(app.runs.is_empty());
        }
        app.back();
        app.discovered.clear();
        app.devices.clear();
        assert!(
            ui::available_actions(&app)
                .iter()
                .any(|item| item.action == ui::MouseAction::ConfigurationDiagnostics)
        );
        app.activate(ui::MouseAction::ConfigurationDiagnostics, area)
            .unwrap();
        assert_eq!(app.page, Page::Configs);
        assert!(app.targets().is_empty());
        app.key(
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
            34,
            12,
        )
        .unwrap();
        let lines = app.diagnostics_lines().join("\n");
        assert!(lines.contains("type must be dart"));
        assert!(lines.contains("Select a target"));
        assert!(!lines.contains("SECRET"));
    }
    #[test]
    fn availability_view_caches_previews_and_launch_rechecks_directory() {
        let directory = tempfile::tempdir().unwrap();
        let cwd = directory.path().join("launch-directory");
        std::fs::create_dir(&cwd).unwrap();
        let mut app = app();
        app.ownit = false;
        app.selected_device = Some(app.devices[0].clone());
        app.launch = Arc::new(
            LaunchFile::parse(
                &serde_json::json!({"configurations":[{"name":"Cached preview", "cwd":cwd}]})
                    .to_string(),
                &directory.path().join("launch.json"),
                directory.path(),
            )
            .unwrap(),
        );
        app.merge();
        app.settle_availability();
        assert_eq!(app.visible_entries().len(), 1);
        assert_eq!(app.availability_view.borrow().assessments.len(), 1);
        std::fs::remove_dir(&cwd).unwrap();
        // Repeated presentation reads use cached metadata, with no planner IO.
        assert_eq!(app.visible_entries().len(), 1);
        app.start_batch(34, 12);
        assert!(app.runs.is_empty());
        assert!(app.diagnostics_open);
        assert!(
            app.diagnostics_lines()
                .join("\n")
                .contains("cwd is not an existing directory")
        );
        assert_eq!(
            app.availability_view.borrow().assessments[&0].reasons[0].code,
            "cwd"
        );
        // An unchanged discovery scan retains cached previews. Explaining the
        // externally repaired row refreshes it, without starting a child.
        std::fs::create_dir(&cwd).unwrap();
        app.merge();
        app.settle_availability();
        app.diagnostics_open = false;
        app.start_batch(34, 12);
        assert!(app.runs.is_empty());
        assert!(app.diagnostics_open);
        assert_eq!(app.availability_counts(), (1, 0, 0));
        assert_eq!(app.visible_entries().len(), 1);

        // A catalog refresh can reuse the same source index and allocation.
        Arc::make_mut(&mut app.launch).configurations[0].pre_launch_task = Some("build".into());
        app.show_unavailable = false;
        app.merge();
        app.settle_availability();
        assert!(app.visible_entries().is_empty());
        app.show_unavailable = true;
        assert_eq!(
            app.assessment(&app.launch.entries[0]).reasons[0].code,
            "preLaunchTask"
        );
    }

    #[test]
    fn unchanged_discovery_preserves_rejected_duplicate_source_and_explanation() {
        let mut app = app();
        app.ownit = false;
        let workspace = app.launch.workspace.clone();
        app.launch = Arc::new(LaunchFile::parse(
            r#"{"configurations":[{"name":"Duplicate"},{"name":"Duplicate","type":"unsupported"}]}"#,
            &workspace.join("launch.json"), &workspace).unwrap());
        app.selected_device = Some(app.devices[0].clone());
        app.page = Page::Configs;
        app.show_unavailable = true;
        app.settle_availability();
        app.config_index = 1;
        app.diagnostics_open = true;
        let generation = app.availability_view.borrow().generation;
        app.merge(); // the device-discovery completion path
        assert_eq!(app.visible_entries()[app.config_index].source_index, 1);
        assert!(app.diagnostics_open);
        assert!(
            app.diagnostics_lines()
                .join("\n")
                .contains("type must be dart")
        );
        assert_eq!(app.availability_view.borrow().generation, generation);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 34, 12)
            .unwrap();
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 34, 12)
            .unwrap();
        assert!(app.diagnostics_open);
        assert_eq!(app.visible_entries()[app.config_index].source_index, 1);
        assert!(app.runs.is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn blocked_or_pending_enter_only_explains_then_a_later_ready_run_spawns_once() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let cwd = directory.path().join("created-later");
        let counter = directory.path().join("spawn-count");
        let executable = directory.path().join("fixture-tool");
        std::fs::write(
            &executable,
            "#!/bin/sh\nprintf 'spawn\\n' >> \"$SPAWN_COUNT\"\nexec sleep 30\n",
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        for pending in [false, true] {
            if cwd.exists() {
                std::fs::remove_dir(&cwd).unwrap();
            }
            let mut app = app();
            app.ownit = false;
            app.tool.executable = executable.clone();
            app.selected_device = Some(app.devices[0].clone());
            app.page = Page::Configs;
            app.show_unavailable = true;
            app.launch = Arc::new(
                LaunchFile::parse(
                    &serde_json::json!({"configurations":[{
                        "name":"External recovery", "cwd":cwd, "env":{"SPAWN_COUNT":counter}
                    }]})
                    .to_string(),
                    &directory.path().join("launch.json"),
                    directory.path(),
                )
                .unwrap(),
            );
            if !pending {
                app.settle_availability();
            } else {
                app.ensure_availability_context();
            }
            assert_eq!(app.assessment(&app.launch.entries[0]).pending, pending);
            assert!(
                !ui::available_actions(&app)
                    .iter()
                    .any(|item| item.label == "Run configuration")
            );
            std::fs::create_dir(&cwd).unwrap();
            let before = std::fs::read_to_string(&counter)
                .unwrap_or_default()
                .lines()
                .count();
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 80, 24)
                .unwrap();
            assert!(app.diagnostics_open);
            assert!(app.runs.is_empty());
            assert_eq!(
                std::fs::read_to_string(&counter)
                    .unwrap_or_default()
                    .lines()
                    .count(),
                before
            );
            assert!(app.diagnostics_lines().join("\n").contains("Ready to run"));
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 80, 24)
                .unwrap();
            assert!(!app.diagnostics_open);
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 80, 24)
                .unwrap();
            assert_eq!(app.running_count(), 1);
            let deadline = Instant::now() + Duration::from_secs(2);
            while std::fs::read_to_string(&counter)
                .unwrap_or_default()
                .lines()
                .count()
                == before
            {
                assert!(Instant::now() < deadline);
                app.tick().unwrap();
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(
                std::fs::read_to_string(&counter).unwrap().lines().count(),
                before + 1
            );
        }
    }

    #[test]
    fn diagnostics_keyboard_wheel_and_resize_bound_the_stored_wrapped_position() {
        let mut app = app();
        let workspace = app.launch.workspace.clone();
        app.launch = Arc::new(LaunchFile::parse(
            r#"{"configurations":[{"name":"Very long unsupported configuration 界 é","type":"unsupported"}],"compounds":[{}]}"#,
            &workspace.join("launch.json"), &workspace).unwrap());
        app.open_diagnostics_picker();
        app.settle_availability();
        app.diagnostics_open = true;
        let compact = ratatui::layout::Rect::new(0, 0, 34, 12);
        for _ in 0..50 {
            app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 34, 12)
                .unwrap();
        }
        let bottom = app.diagnostics_offset;
        assert!(bottom > 0 && bottom < 50);
        app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), 34, 12)
            .unwrap();
        assert_eq!(app.diagnostics_offset, bottom - 1);
        let wheel = |kind| MouseEvent {
            kind,
            column: 2,
            row: 2,
            modifiers: KeyModifiers::NONE,
        };
        for _ in 0..50 {
            app.mouse(wheel(MouseEventKind::ScrollDown), compact)
                .unwrap();
        }
        assert_eq!(app.diagnostics_offset, bottom);
        app.mouse(wheel(MouseEventKind::ScrollUp), compact).unwrap();
        assert_eq!(app.diagnostics_offset, bottom - 1);
        ui::update_picker_viewport(&mut app, ratatui::layout::Rect::new(0, 0, 120, 40));
        assert_eq!(app.diagnostics_offset, 0);
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 120, 40)
            .unwrap();
        assert_eq!(app.diagnostics_offset, 0); // all content fits
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 34, 12)
            .unwrap();
        app.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE), 34, 12)
            .unwrap();
        assert_eq!(app.diagnostics_offset, 0);
    }

    #[test]
    #[cfg(unix)]
    fn large_catalog_presentation_stays_responsive_while_live_pty_is_drained() {
        let mut app = app();
        app.ownit = false;
        app.show_unavailable = true;
        app.selected_device = Some(app.devices[0].clone());
        app.page = Page::Configs;
        let workspace = app.launch.workspace.clone();
        let raw = serde_json::json!({"configurations":(0..100_000).map(|i|
            serde_json::json!({"name":format!("Configuration {i}")})).collect::<Vec<_>>()})
        .to_string();
        assert!(raw.len() < 4 * 1024 * 1024);
        app.launch =
            Arc::new(LaunchFile::parse(&raw, &workspace.join("launch.json"), &workspace).unwrap());
        let session = Session::start(
            &flimru::tool::RunCommand {
                capture_vm_service: false,
                hot_capabilities: flimru::batch_commands::HotCapabilities::Supported,
                executable: "/bin/sh".into(),
                args: vec![
                    "-c".into(),
                    "while :; do printf 'heartbeat\\r\\n'; sleep 0.01; done".into(),
                ],
                cwd: workspace,
                env: Default::default(),
            },
            15,
            77,
        )
        .unwrap();
        app.runs.push(runs::Run::running(
            app.devices[0].clone(),
            "Live".into(),
            session,
            flimru::batch_commands::HotCapabilities::Supported,
        ));
        let started = Instant::now();
        app.merge();
        let initial = started.elapsed();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        let started = Instant::now();
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
        let pending_draw = started.elapsed();
        let started = Instant::now();
        app.selected_device.as_mut().unwrap().platform = "changed-platform".into();
        app.ensure_availability_context();
        let target_change = started.elapsed();
        let generation = app.availability_view.borrow().generation;
        let mut max_tick = Duration::ZERO;
        let mut max_draw = Duration::ZERO;
        let mut output_seen = false;
        let deadline = Instant::now() + Duration::from_secs(15);
        while app.availability_counts().2 != 0 {
            assert!(Instant::now() < deadline);
            let started = Instant::now();
            output_seen |= app.tick().unwrap().output_received;
            max_tick = max_tick.max(started.elapsed());
            let started = Instant::now();
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            max_draw = max_draw.max(started.elapsed());
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(app.availability_counts(), (100_000, 0, 0));
        assert_eq!(app.visible_entries().len(), 100_000);
        assert!(output_seen);
        // Also exercise target invalidation after the entire 100k cache exists,
        // including disposal of old metadata and stale-result rejection.
        let started = Instant::now();
        app.selected_device.as_mut().unwrap().platform = "final-platform".into();
        app.ensure_availability_context();
        let full_target_change = started.elapsed();
        assert!(app.availability_view.borrow().generation > generation);
        let generation = app.availability_view.borrow().generation;
        while app.availability_counts().2 != 0 {
            assert!(Instant::now() < deadline);
            let started = Instant::now();
            app.tick().unwrap();
            max_tick = max_tick.max(started.elapsed());
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(app.availability_counts(), (100_000, 0, 0));
        let started = Instant::now();
        app.merge();
        let unchanged_merge = started.elapsed();
        assert_eq!(app.availability_view.borrow().generation, generation);
        assert_eq!(app.availability_counts(), (100_000, 0, 0));
        let started = Instant::now();
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
        let cached_draw = started.elapsed();
        eprintln!(
            "100000 entries / {} bytes: initial merge {initial:?}; pending draw {pending_draw:?}; target change {target_change:?}; full-cache target change {full_target_change:?}; max tick {max_tick:?}; max draw {max_draw:?}; unchanged merge {unchanged_merge:?}; cached draw {cached_draw:?}",
            raw.len()
        );
        // A broad regression bound tolerates loaded debug CI; the evidence
        // records actual timings against the 33ms frame target on this machine.
        for elapsed in [
            initial,
            pending_draw,
            target_change,
            full_target_change,
            max_tick,
            max_draw,
            unchanged_merge,
            cached_draw,
        ] {
            assert!(
                elapsed < Duration::from_millis(100),
                "UI operation took {elapsed:?}"
            );
        }
    }

    pub(super) fn take_queued_previews(app: &App) -> Vec<(usize, Assessment)> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let results = {
                let view = app.availability_view.borrow();
                view.worker.as_ref().unwrap().drain(view.generation)
            };
            if !results.is_empty() {
                return results;
            }
            assert!(Instant::now() < deadline, "worker did not queue preview");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn hidden_unavailable_selection_survives_target_context_and_partial_async_fill() {
        let mut app = app();
        app.ownit = false;
        let workspace = app.launch.workspace.clone();
        app.launch = Arc::new(
            LaunchFile::parse(
                r#"{"configurations":[{"name":"First"},{"name":"Selected second"}]}"#,
                &workspace.join("launch.json"),
                &workspace,
            )
            .unwrap(),
        );
        app.discovered = vec![app.devices[0].clone()];
        app.discovered[0].name = "Original target".into();
        app.selected_device = Some(app.discovered[0].clone());
        app.page = Page::Configs;
        app.merge();
        app.settle_availability();
        app.set_config_selection(1);
        assert!(!app.show_unavailable);
        assert_eq!(app.visible_entries()[app.config_index].source_index, 1);
        app.discovered[0].name = "Updated target label".into();
        app.merge();
        assert_eq!(app.config_index, usize::MAX);
        assert_eq!(app.availability_view.borrow().desired_source, Some(1));
        let mut results = take_queued_previews(&app);
        assert_eq!(results.len(), 2);
        let selected = results.pop().unwrap();
        app.apply_availability_results(results); // First is ready; Selected is pending.
        assert_eq!(app.visible_entries().len(), 1);
        assert_eq!(app.config_index, usize::MAX);
        assert!(
            !ui::available_actions(&app)
                .iter()
                .any(|item| item.label == "Run configuration")
        );
        app.start_batch(80, 24);
        assert!(app.runs.is_empty());
        assert!(!app.diagnostics_open);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let first = buffer
            .content
            .iter()
            .skip(80 * 4)
            .find(|cell| cell.symbol() == "F")
            .unwrap();
        assert_eq!(
            first.bg,
            ratatui::style::Color::Rgb(22, 28, 39),
            "no other row may inherit the absent selection's highlight"
        );
        app.apply_availability_results(vec![selected]);
        assert_eq!(app.config_index, 1);
        assert_eq!(app.visible_entries()[app.config_index].source_index, 1);
        assert_eq!(app.availability_view.borrow().desired_source, None);
        assert!(
            ui::available_actions(&app)
                .iter()
                .any(|item| item.label == "Run configuration")
        );
        app.discovered[0].platform = "updated-platform".into();
        app.merge();
        assert_eq!(app.config_index, usize::MAX);
        app.settle_availability();
        assert_eq!(app.visible_entries()[app.config_index].source_index, 1);
        app.tool.executable = "/bin/echo".into();
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 80, 24)
            .unwrap();
        assert_eq!(app.runs.len(), 1);
        assert_eq!(app.runs[0].config, "Selected second");
    }

    #[test]
    fn deliberate_picker_transitions_release_a_deferred_source_without_overflow() {
        for transition in [
            "up",
            "down",
            "home",
            "end",
            "query",
            "toggle",
            "diagnostics",
            "mouse",
        ] {
            let mut app = app();
            app.ownit = false;
            app.page = Page::Configs;
            app.selected_device = Some(app.devices[0].clone());
            let workspace = app.launch.workspace.clone();
            app.launch = Arc::new(
                LaunchFile::parse(
                    r#"{"configurations":[{"name":"First"},{"name":"Second"}]}"#,
                    &workspace.join("launch.json"),
                    &workspace,
                )
                .unwrap(),
            );
            app.settle_availability();
            app.set_config_selection(1);
            app.selected_device
                .as_mut()
                .unwrap()
                .name
                .push_str(" changed");
            app.ensure_availability_context();
            app.restore_deferred_config_selection();
            assert_eq!(app.config_index, usize::MAX);
            assert_eq!(app.availability_view.borrow().desired_source, Some(1));
            match transition {
                "query" => {
                    app.searching = true;
                    app.paste("First");
                }
                "toggle" => {
                    app.key(
                        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE),
                        80,
                        24,
                    )
                    .unwrap();
                }
                "diagnostics" => app.open_diagnostics_picker(),
                "mouse" => {
                    app.activate(
                        ui::MouseAction::SelectConfig(0),
                        ratatui::layout::Rect::new(0, 0, 80, 24),
                    )
                    .unwrap();
                }
                name => {
                    let code = match name {
                        "up" => KeyCode::Up,
                        "down" => KeyCode::Down,
                        "home" => KeyCode::Home,
                        _ => KeyCode::End,
                    };
                    app.key(KeyEvent::new(code, KeyModifiers::NONE), 80, 24)
                        .unwrap();
                }
            }
            assert_eq!(
                app.availability_view.borrow().desired_source,
                None,
                "{transition}"
            );
            assert_eq!(app.config_index, 0, "{transition}");
            app.settle_availability();
            assert_eq!(app.config_index, 0, "{transition}");
            assert_eq!(
                app.visible_entries()[app.config_index].source_index,
                0,
                "{transition}"
            );
        }
    }

    #[test]
    fn queued_same_generation_preview_cannot_overwrite_fresh_explanation_preflight() {
        for initially_ready in [true, false] {
            let directory = tempfile::tempdir().unwrap();
            let cwd = directory.path().join("changed-after-preview");
            if initially_ready {
                std::fs::create_dir(&cwd).unwrap();
            }
            let mut app = app();
            app.ownit = false;
            app.selected_device = Some(app.devices[0].clone());
            app.page = Page::Configs;
            app.show_unavailable = true;
            app.launch = Arc::new(
                LaunchFile::parse(
                    &serde_json::json!({"configurations":[{"name":"Fresh validation", "cwd":cwd}]})
                        .to_string(),
                    &directory.path().join("launch.json"),
                    directory.path(),
                )
                .unwrap(),
            );
            app.ensure_availability_context();
            let generation = app.availability_view.borrow().generation;
            let stale = take_queued_previews(&app); // Actual queued result, held without presenting it.
            assert_eq!(stale[0].1.runnable(), initially_ready);
            assert!(app.assessment(&app.launch.entries[0]).pending);
            if initially_ready {
                std::fs::remove_dir(&cwd).unwrap();
            } else {
                std::fs::create_dir(&cwd).unwrap();
            }
            app.start_batch(80, 24);
            assert!(app.diagnostics_open);
            assert!(app.runs.is_empty());
            let fresh = app.assessment(&app.launch.entries[0]);
            assert_eq!(fresh.runnable(), !initially_ready);
            if initially_ready {
                assert_eq!(fresh.reasons[0].code, "cwd");
            }
            assert!(
                app.availability_view
                    .borrow()
                    .synchronous_overrides
                    .contains(&0)
            );
            app.apply_availability_results(stale);
            app.advance_availability();
            assert_eq!(app.availability_view.borrow().generation, generation);
            let retained = app.assessment(&app.launch.entries[0]);
            assert_eq!(retained.runnable(), fresh.runnable());
            assert_eq!(retained.reasons, fresh.reasons);
            assert_eq!(
                app.availability_counts(),
                if initially_ready {
                    (0, 1, 0)
                } else {
                    (1, 0, 0)
                }
            );
            assert!(app.runs.is_empty());
            if initially_ready {
                assert!(
                    !ui::available_actions(&app)
                        .iter()
                        .any(|item| item.label == "Run configuration")
                );
                app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 80, 24)
                    .unwrap();
                app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 80, 24)
                    .unwrap();
                assert!(app.diagnostics_open);
                assert!(app.runs.is_empty());
                std::fs::create_dir(&cwd).unwrap();
            }
            // A new context may refresh the fenced entry normally.
            app.selected_device
                .as_mut()
                .unwrap()
                .name
                .push_str(" updated");
            app.settle_availability();
            assert!(
                app.availability_view
                    .borrow()
                    .synchronous_overrides
                    .is_empty()
            );
            assert!(app.assessment(&app.launch.entries[0]).runnable());
        }
    }

    #[test]
    fn source_selection_apply_helper_preserves_an_explicit_empty_selection_through_merge() {
        let mut app = app();
        app.ownit = false;
        app.page = Page::Configs;
        app.selected_device = Some(app.devices[0].clone());
        let workspace = app.launch.workspace.clone();
        app.launch = Arc::new(
            LaunchFile::parse(
                r#"{"configurations":[{"name":"First"},{"name":"Second"}]}"#,
                &workspace.join("launch.json"),
                &workspace,
            )
            .unwrap(),
        );
        app.set_config_source_selection(Some(1));
        app.merge();
        app.settle_availability();
        assert_eq!(app.visible_entries()[app.config_index].source_index, 1);
        app.set_config_source_selection(None);
        app.merge();
        app.advance_availability();
        assert_eq!(app.config_index, usize::MAX);
        assert!(
            !ui::available_actions(&app)
                .iter()
                .any(|item| item.label == "Run configuration")
        );
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 80, 24)
            .unwrap();
        assert_eq!(app.config_index, 0);
    }

    #[test]
    fn selected_diagnostics_isolate_entry_warnings_keep_file_warnings_and_bound_extreme_text() {
        let mut app = app();
        app.ownit = false;
        app.page = Page::Configs;
        app.show_unavailable = true;
        app.selected_device = Some(app.devices[0].clone());
        let workspace = app.launch.workspace.clone();
        app.launch = Arc::new(
            LaunchFile::parse(
                r#"{"configurations":[
            {"name":"Duplicate","firstUnknown":true},
            {"name":"Duplicate","secondUnknown":true,"args":["--device-id"]}
        ],"compounds":[{}]}"#,
                &workspace.join("launch.json"),
                &workspace,
            )
            .unwrap(),
        );
        app.settle_availability();
        app.set_config_selection(0);
        let first = app.diagnostics_lines().join("\n");
        assert!(first.contains("firstUnknown"));
        assert!(!first.contains("secondUnknown"));
        assert!(first.contains("Compound launch"));
        app.set_config_selection(1);
        let second = app.diagnostics_lines().join("\n");
        assert!(second.contains("secondUnknown"));
        assert!(second.contains("Invalid Flutter arguments"));
        assert!(!second.contains("firstUnknown"));
        assert!(second.contains("Compound launch"));
        app.set_config_source_selection(None);
        let summary = app.diagnostics_lines().join("\n");
        assert!(summary.contains("Select an entry"));
        assert!(summary.contains("1 ready / 1 blocked"));
        assert!(summary.contains("--check"));
        assert!(!summary.contains("firstUnknown") && !summary.contains("secondUnknown"));
        let device_details = app.catalog_warning_lines().join("\n");
        assert!(device_details.contains("Compound launch"));
        assert!(
            !device_details.contains("firstUnknown") && !device_details.contains("secondUnknown")
        );

        let label = "界é".repeat(2_000);
        let mut source = serde_json::Map::new();
        source.insert("name".into(), label.clone().into());
        for i in 0..40 {
            source.insert(format!("unknown{i:02}"), true.into());
        }
        app.launch = Arc::new(
            LaunchFile::parse(
                &serde_json::json!({"configurations":[source],"compounds":[{}]}).to_string(),
                &workspace.join("launch.json"),
                &workspace,
            )
            .unwrap(),
        );
        app.settle_availability();
        app.set_config_selection(0);
        let shown = app.diagnostics_lines();
        assert!(shown[0].contains("shortened; --check"));
        assert!(
            shown
                .iter()
                .any(|line| line.contains("24 more warnings; use --check"))
        );
        assert!(shown.len() < 25);
        assert!(
            shown
                .iter()
                .all(|line| line.chars().count() <= DIAGNOSTIC_TEXT_CHARS + 40)
        );
        assert_eq!(app.launch.entries[0].name, label);
        assert_eq!(app.launch.entries[0].warnings.len(), 40);
        assert_eq!(app.launch.warnings.len(), 41); // CLI retains every original warning.
        app.diagnostics_open = true;
        let area = ratatui::layout::Rect::new(0, 0, 34, 12);
        for _ in 0..1000 {
            app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 34, 12)
                .unwrap();
        }
        let bottom = app.diagnostics_offset;
        assert!(bottom > 0);
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 2,
                row: 2,
                modifiers: KeyModifiers::NONE,
            },
            area,
        )
        .unwrap();
        assert_eq!(app.diagnostics_offset, bottom - 1);
    }

    #[test]
    #[cfg(unix)]
    fn rejected_catalog_diagnostics_are_bounded_while_a_live_pty_is_drained() {
        let mut app = app();
        app.ownit = false;
        app.show_unavailable = true;
        app.page = Page::Configs;
        app.selected_device = Some(app.devices[0].clone());
        let workspace = app.launch.workspace.clone();
        let raw = format!(
            "{{\"configurations\":[{}],\"compounds\":[{{}}]}}",
            vec![r#"{"type":"unsupported"}"#; 100_000].join(",")
        );
        assert!(raw.len() < 4 * 1024 * 1024);
        app.launch =
            Arc::new(LaunchFile::parse(&raw, &workspace.join("launch.json"), &workspace).unwrap());
        assert_eq!(app.launch.warnings.len(), 100_001);
        assert!(
            app.launch
                .entries
                .iter()
                .all(|entry| !entry.diagnostics.is_empty())
        );
        let session = Session::start(
            &flimru::tool::RunCommand {
                capture_vm_service: false,
                hot_capabilities: flimru::batch_commands::HotCapabilities::Supported,
                executable: "/bin/sh".into(),
                args: vec![
                    "-c".into(),
                    "while :; do printf 'rejected-heartbeat\\r\\n'; sleep 0.01; done".into(),
                ],
                cwd: workspace,
                env: Default::default(),
            },
            15,
            77,
        )
        .unwrap();
        app.runs.push(runs::Run::running(
            app.devices[0].clone(),
            "Live".into(),
            session,
            flimru::batch_commands::HotCapabilities::Supported,
        ));
        let started = Instant::now();
        app.key(
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
            80,
            24,
        )
        .unwrap();
        let open = started.elapsed();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        let mut max_draw = Duration::ZERO;
        let mut max_down = Duration::ZERO;
        let mut max_wheel = Duration::ZERO;
        let mut max_tick = Duration::ZERO;
        let mut output_seen = false;
        let deadline = Instant::now() + Duration::from_secs(15);
        while app.availability_counts().2 != 0 || !output_seen {
            assert!(Instant::now() < deadline);
            let started = Instant::now();
            output_seen |= app.tick().unwrap().output_received;
            max_tick = max_tick.max(started.elapsed());
            let started = Instant::now();
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            max_draw = max_draw.max(started.elapsed());
            let started = Instant::now();
            app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 80, 24)
                .unwrap();
            max_down = max_down.max(started.elapsed());
            let started = Instant::now();
            app.mouse(
                MouseEvent {
                    kind: MouseEventKind::ScrollDown,
                    column: 2,
                    row: 2,
                    modifiers: KeyModifiers::NONE,
                },
                ratatui::layout::Rect::new(0, 0, 80, 24),
            )
            .unwrap();
            max_wheel = max_wheel.max(started.elapsed());
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(app.availability_counts(), (0, 100_000, 0));
        assert!(
            app.runs[0]
                .screen()
                .unwrap()
                .contents()
                .contains("rejected-heartbeat")
        );
        let mut max_resize = Duration::ZERO;
        for (width, height) in [(34, 12), (120, 40), (80, 24)] {
            let started = Instant::now();
            ui::update_picker_viewport(&mut app, ratatui::layout::Rect::new(0, 0, width, height));
            let mut resized =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            resized.draw(|frame| ui::draw(frame, &app)).unwrap();
            max_resize = max_resize.max(started.elapsed());
        }
        for index in [0, 50_000, 99_999] {
            app.set_config_selection(index);
            let shown = app.diagnostics_lines();
            assert!(shown.len() < 10);
            assert!(shown.iter().any(|line| line.contains("type must be dart")));
            assert!(shown.iter().any(|line| line.contains("Compound launch")));
            assert!(!shown.iter().any(|line| line.contains("Skipped")));
        }
        app.set_config_source_selection(None);
        assert!(app.diagnostics_lines().len() < 10);
        assert!(app.catalog_warning_lines().len() < 5);
        eprintln!(
            "100000 rejected entries / {} bytes: modal open {open:?}; max draw {max_draw:?}; max Down {max_down:?}; max wheel {max_wheel:?}; max tick {max_tick:?}; max resize/draw {max_resize:?}",
            raw.len()
        );
        for elapsed in [open, max_draw, max_down, max_wheel, max_tick, max_resize] {
            assert!(
                elapsed < Duration::from_millis(100),
                "UI operation took {elapsed:?}"
            );
        }
    }
}

#[cfg(test)]
#[path = "../tests/support/console_search_app.rs"]
mod console_search_app_tests;

#[cfg(test)]
#[path = "../tests/support/log_export_app.rs"]
mod log_export_app_tests;

#[cfg(test)]
#[path = "../tests/support/batch_commands_app.rs"]
mod batch_commands_app_tests;

#[cfg(test)]
#[path = "../tests/support/productivity_workflow_app.rs"]
mod productivity_workflow_app_tests;
