//! CLI and MCP adapters for sessions owned by the open Flimru TUI.

mod mcp;
mod transport;

use crate::{App, runs::Run};
use anyhow::{Context, Result, bail, ensure};
use clap::{Subcommand, ValueEnum};
use flimru::{animation::AnimationState, availability, session_log};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{self, Write},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

pub use transport::Server;
const WIRE_VERSION: u32 = 1;
const CONFIG_PAGE_SIZE: usize = 50;
const MCP_LOG_BYTES: usize = 8192;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Control shared Flutter runs from a shell.
    Session {
        /// Workspace root (defaults to the nearest .vscode/launch.json or the current directory).
        #[arg(long, global = true, value_name = "DIR")]
        project: Option<PathBuf>,
        #[command(subcommand)]
        action: CliAction,
    },
    /// Serve the shared-session MCP tools over stdio.
    Mcp {
        /// Workspace root (defaults to the nearest .vscode/launch.json or the current directory).
        #[arg(long, value_name = "DIR")]
        project: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
pub enum CliAction {
    /// List device IDs and the current page of launch configurations.
    Targets {
        #[arg(long, default_value_t = 0)]
        config_offset: usize,
    },
    /// List live and retained ended sessions.
    List,
    /// Read the state, outcome and available controls for a session.
    Inspect { id: String },
    /// Launch in the open Flimru window and return the new session ID.
    Start {
        #[arg(long)]
        device: String,
        /// Exact launch configuration name.
        #[arg(
            long,
            conflicts_with = "config_index",
            required_unless_present = "config_index"
        )]
        config: Option<String>,
        /// One-based position shown by `session targets`.
        #[arg(long)]
        config_index: Option<usize>,
    },
    /// Print retained console output, optionally continuing until the run ends.
    Logs {
        id: String,
        #[arg(long)]
        follow: bool,
    },
    /// Request hot reload for one session.
    Reload { id: String },
    /// Request hot restart for one session.
    Restart { id: String },
    /// Stop one session and leave Flimru open.
    Stop { id: String },
    /// Send a named control to one session.
    Control {
        id: String,
        #[arg(value_enum)]
        control: Control,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, ValueEnum, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Control {
    Reload,
    Restart,
    Stop,
    Help,
    Devtools,
    ToggleAnimations,
    ReconnectAnimations,
}

#[derive(Deserialize, Serialize)]
pub struct WireRequest {
    version: u32,
    request: Request,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Targets {
        #[serde(default)]
        config_offset: usize,
    },
    Sessions {
        id: Option<String>,
    },
    Start {
        device: String,
        config: Option<String>,
        config_index: Option<usize>,
    },
    Logs {
        id: String,
        cursor: Option<u64>,
        #[serde(default = "mcp_log_bytes")]
        limit: usize,
        until: Option<u64>,
    },
    Control {
        id: String,
        control: Control,
    },
}

fn mcp_log_bytes() -> usize {
    MCP_LOG_BYTES
}

#[derive(Deserialize, Serialize)]
struct Logs {
    #[serde(flatten)]
    page: session_log::LogPage,
    state: RunState,
    outcome: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RunState {
    Running,
    Stopping,
    Ended,
}

pub fn run(command: Command) -> Result<()> {
    match command {
        Command::Mcp { project } => mcp::serve(client_workspace(project)?),
        Command::Session { project, action } => cli(&client_workspace(project)?, action),
    }
}

fn client_workspace(project: Option<PathBuf>) -> Result<PathBuf> {
    let explicit = project.is_some();
    let directory = std::fs::canonicalize(project.unwrap_or(std::env::current_dir()?))
        .context("Project directory not found")?;
    ensure!(directory.is_dir(), "--project must be a directory");
    if !explicit
        && let Some(root) = directory
            .ancestors()
            .find(|path| path.join(".vscode/launch.json").is_file())
    {
        return Ok(root.to_path_buf());
    }
    Ok(directory)
}

fn cli(workspace: &Path, action: CliAction) -> Result<()> {
    let request = match action {
        CliAction::List => Request::Sessions { id: None },
        CliAction::Inspect { id } => Request::Sessions { id: Some(id) },
        CliAction::Targets { config_offset } => Request::Targets { config_offset },
        CliAction::Start {
            device,
            config,
            config_index,
        } => Request::Start {
            device,
            config,
            config_index,
        },
        CliAction::Logs { id, follow } => return cli_logs(workspace, id, follow),
        CliAction::Reload { id } => Request::Control {
            id,
            control: Control::Reload,
        },
        CliAction::Restart { id } => Request::Control {
            id,
            control: Control::Restart,
        },
        CliAction::Stop { id } => Request::Control {
            id,
            control: Control::Stop,
        },
        CliAction::Control { id, control } => Request::Control { id, control },
    };
    let value = transport::request(workspace, request)?;
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &value)?;
    writeln!(stdout)?;
    Ok(())
}

fn cli_logs(workspace: &Path, id: String, follow: bool) -> Result<()> {
    let mut cursor = 0;
    let mut until = None;
    let mut stdout = io::stdout().lock();
    loop {
        let logs: Logs = serde_json::from_value(transport::request(
            workspace,
            Request::Logs {
                id: id.clone(),
                cursor: Some(cursor),
                limit: session_log::MAX_READ,
                until,
            },
        )?)?;
        if logs.page.truncated {
            eprintln!("Some earlier console output is outside the retained history.");
        }
        if let Err(error) = stdout
            .write_all(logs.page.text.as_bytes())
            .and_then(|()| stdout.flush())
        {
            if error.kind() == io::ErrorKind::BrokenPipe {
                return Ok(());
            }
            return Err(error.into());
        }
        cursor = logs.page.cursor;
        if !follow {
            until.get_or_insert(logs.page.end_cursor);
        }
        if cursor >= logs.page.end_cursor {
            if follow && logs.state == RunState::Ended {
                eprintln!(
                    "Session ended: {}",
                    logs.outcome.as_deref().unwrap_or("finished")
                );
            }
            if !follow || logs.state == RunState::Ended {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(150));
        }
    }
}

impl App {
    pub fn handle_shared(
        &mut self,
        request: Request,
        instance: &str,
        area: ratatui::layout::Size,
        redraw: &mut bool,
    ) -> Result<Value> {
        match request {
            Request::Targets { config_offset } => Ok(self.shared_targets(config_offset)),
            Request::Sessions { id: Some(id) } => Ok(summary(
                self.shared_run(parse_id(&id, instance)?)?,
                instance,
            )),
            Request::Sessions { id: None } => {
                let sessions: Vec<_> = self
                    .runs
                    .iter()
                    .chain(&self.last_session)
                    .map(|run| summary(run, instance))
                    .collect();
                Ok(json!({"sessions": sessions}))
            }
            Request::Start {
                device,
                config,
                config_index,
            } => {
                let id = self.shared_start(&device, config.as_deref(), config_index, area)?;
                *redraw = true;
                Ok(summary(self.shared_run(id)?, instance))
            }
            Request::Logs {
                id,
                cursor,
                limit,
                until,
            } => {
                let run = self.shared_run(parse_id(&id, instance)?)?;
                let log = run.log().context("No output retained for this session")?;
                Ok(serde_json::to_value(Logs {
                    page: log.read(cursor, limit, until),
                    state: run_state(run),
                    outcome: run.outcome.clone(),
                })?)
            }
            Request::Control { id, control } => {
                *redraw = true;
                self.shared_control(parse_id(&id, instance)?, control, instance)
            }
        }
    }

    fn shared_targets(&self, config_offset: usize) -> Value {
        let devices: Vec<_> = self.devices.iter().map(device_json).collect();
        let entries = &self.launch.entries;
        let configurations: Vec<_> = entries
            .iter()
            .skip(config_offset)
            .take(CONFIG_PAGE_SIZE)
            .map(|entry| {
                let config = entry
                    .configuration
                    .and_then(|index| self.launch.configurations.get(index));
                let diagnostics: Vec<_> = entry
                    .diagnostics
                    .iter()
                    .map(|reason| reason.display())
                    .collect();
                json!({
                    "index": entry.source_index + 1,
                    "name": entry.name,
                    "device_id": config.and_then(|config| config.device_id.as_ref()),
                    "needs_device": config.map(|config| config.needs_device()),
                    "diagnostics": diagnostics,
                })
            })
            .collect();
        let next = config_offset.saturating_add(configurations.len());
        json!({
            "devices": devices,
            "discovering": self.busy,
            "configurations": configurations,
            "next_config_offset": (next < entries.len()).then_some(next),
        })
    }

    fn shared_start(
        &mut self,
        device_id: &str,
        config_name: Option<&str>,
        config_index: Option<usize>,
        area: ratatui::layout::Size,
    ) -> Result<u64> {
        if let Some(message) = self.reload_run_guard() {
            bail!("{message}");
        }
        ensure!(
            !self.quit_confirmation && !self.console_ui_owned(),
            "Finish or dismiss the open Flimru dialog or console review, then retry."
        );
        let launch = &self.launch;
        let entry = match (config_name, config_index) {
            (Some(name), None) => {
                let mut matches = launch.entries.iter().filter(|entry| entry.name == name);
                let entry = matches.next().context("Unknown configuration name. List launch targets for its exact name or index.")?;
                ensure!(
                    matches.next().is_none(),
                    "Configuration name is ambiguous. List launch targets and select its one-based configuration index."
                );
                entry
            }
            (None, Some(index)) => index.checked_sub(1).and_then(|index| launch.entries.get(index))
                .context("Unknown configuration index. List launch targets for current one-based indexes.")?,
            _ => bail!("Provide exactly one of config or config_index."),
        };
        let device = self.devices.iter().find(|device| device.id == device_id)
            .context("Unknown device ID. List launch targets for exact IDs and wait for device discovery if needed.")?;
        let assessment = availability::assess(launch, entry, &self.tool, &[device], self.ownit);
        if !assessment.runnable() {
            let reasons = assessment
                .reasons
                .iter()
                .take(4)
                .map(|reason| reason.display())
                .collect::<Vec<_>>();
            bail!(
                "Cannot launch this configuration on the selected target. {}",
                reasons.join(" ")
            );
        }
        let config = entry
            .configuration
            .and_then(|index| launch.configurations.get(index))
            .context("The chosen configuration could not be loaded.")?
            .clone();
        let id = self.launch_runs(
            config,
            vec![device.clone()],
            assessment.plans,
            (area.width, area.height),
            true,
        )?;
        self.diagnostics_open = false;
        Ok(id)
    }

    fn shared_run(&self, id: u64) -> Result<&Run> {
        self.runs
            .iter()
            .chain(&self.last_session)
            .find(|run| run.id == id)
            .context("Unknown or no longer retained session. List sessions for current IDs.")
    }

    fn shared_control(&mut self, id: u64, control: Control, instance: &str) -> Result<Value> {
        let run = self.shared_run(id)?;
        if let Some(reason) = control_unavailable(run, control) {
            bail!("{reason}");
        }
        let device = flimru::batch_commands::bounded_device_label(&run.device.name);
        let session = self
            .runs
            .iter_mut()
            .find(|run| run.id == id)
            .and_then(|run| run.session.as_mut())
            .context("The selected session has ended.")?;
        let message = match control {
            Control::Reload => {
                session.send(crate::HotCommand::Reload.bytes())?;
                "Hot reload requested"
            }
            Control::Restart => {
                session.send(crate::HotCommand::Restart.bytes())?;
                "Hot restart requested"
            }
            Control::Stop if session.stop_requested.is_some() => "Stop already requested",
            Control::Stop => {
                session.request_stop()?;
                "Stop requested"
            }
            Control::Help => {
                session.send(b"h")?;
                "Flutter help requested"
            }
            Control::Devtools => {
                session.send(b"v")?;
                "DevTools requested"
            }
            Control::ToggleAnimations => {
                session.animations.toggle()?;
                "Animation change requested"
            }
            Control::ReconnectAnimations => {
                session.reconnect_animations()?;
                "VM Service reconnect requested"
            }
        };
        self.report_operation(format!("{message} · {device}"));
        Ok(json!({"message": message, "session": summary(self.shared_run(id)?, instance)}))
    }
}

fn parse_id(id: &str, instance: &str) -> Result<u64> {
    let Some((owner, id)) = id.split_once('-') else {
        bail!("Invalid session ID. Use the full ID shown in the session list.");
    };
    ensure!(
        owner == instance,
        "That session ID belongs to an earlier or different Flimru instance. List sessions for current IDs."
    );
    id.parse().context("Invalid session ID")
}

fn run_state(run: &Run) -> RunState {
    match &run.session {
        None => RunState::Ended,
        Some(session) if session.stop_requested.is_some() => RunState::Stopping,
        Some(_) => RunState::Running,
    }
}

fn device_json(device: &flimru::device::Device) -> Value {
    json!({"id": device.id, "name": device.name, "platform": device.platform, "discovered": device.available})
}

fn summary(run: &Run, instance: &str) -> Value {
    let controls: BTreeMap<_, _> = Control::value_variants()
        .iter()
        .map(|&control| (control, control_unavailable(run, control)))
        .collect();
    json!({
        "id": format!("{instance}-{}", run.id),
        "configuration": run.config,
        "device": device_json(&run.device),
        "state": run_state(run),
        "outcome": run.outcome,
        "controls": controls,
        "animation_state": run.session.as_ref().map(|session| session.animations.label()),
    })
}

/// A reason here is also enforced on dispatch. A null reason indicates that
/// the request can be sent, independently of Flutter's eventual response.
fn control_unavailable(run: &Run, control: Control) -> Option<&'static str> {
    let Some(session) = &run.session else {
        return Some("The selected session has ended.");
    };
    if session.stop_requested.is_some() && control != Control::Stop {
        return Some("The selected session is stopping.");
    }
    match control {
        Control::Reload | Control::Restart => {
            run.batch_target().exclusion().map(|reason| reason.reason())
        }
        Control::Help if !session.interactive_flutter => {
            Some("This run does not expose Flutter's interactive commands.")
        }
        Control::Devtools if !session.interactive_flutter || !session.animations.is_enabled() => {
            Some("DevTools requires an interactive Flutter debug or profile run.")
        }
        Control::ToggleAnimations
            if !matches!(session.animations.state(), AnimationState::Confirmed(_)) =>
        {
            Some("Animation control needs an active VM Service with the timeDilation extension.")
        }
        Control::ReconnectAnimations if !session.animations.is_enabled() => {
            Some("Animation control requires a Flutter debug or profile app.")
        }
        _ => None,
    }
}
