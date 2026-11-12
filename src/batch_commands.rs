//! Launch-time hot-command support and best-effort dispatch to stable run IDs.
//!
//! A successful send means input was requested/queued, never that Flutter
//! completed a reload or restart. Callers must send through `Session::send` so
//! existing nonblocking input and animation restart handling remain in effect.

use crate::config::LaunchConfig;
use ratatui::{style::Style, text::Span};

const MAX_DETAILS: usize = 8;
const MAX_DEVICE_CHARS: usize = 120;

/// Remove controls and bound labels without splitting a grapheme. A truncated
/// label includes an ellipsis within the same 120-scalar output budget.
pub fn bounded_device_label(value: &str) -> String {
    // One lookahead scalar detects omitted text while bounding allocation.
    let sanitized: String = value
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_DEVICE_CHARS + 1)
        .collect();
    if sanitized.chars().count() <= MAX_DEVICE_CHARS {
        return sanitized;
    }
    let mut label = String::new();
    let mut scalars = 0;
    let span = Span::raw(&sanitized);
    for grapheme in span.styled_graphemes(Style::default()) {
        let size = grapheme.symbol.chars().count();
        if scalars + size >= MAX_DEVICE_CHARS {
            break;
        }
        label.push_str(grapheme.symbol);
        scalars += size;
    }
    label.push('…');
    label
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotCommand {
    Reload,
    Restart,
}

impl HotCommand {
    pub fn bytes(self) -> &'static [u8] {
        match self {
            Self::Reload => b"r",
            Self::Restart => b"R",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Reload => "Hot reload",
            Self::Restart => "Hot restart",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotExclusion {
    Test,
    Profile,
    Release,
    HotDisabled,
    MachineMode,
    NonResident,
    StartupTracing,
    CustomRunner,
    UnsupportedMode,
}

impl HotExclusion {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Test => "test runs do not support these app hot commands",
            Self::Profile => "profile runs are excluded from batch hot commands",
            Self::Release => "release runs do not support hot commands",
            Self::HotDisabled => "hot mode was disabled at launch",
            Self::MachineMode => "machine mode requires the Flutter daemon protocol",
            Self::NonResident => "nonresident runs do not accept terminal hot commands",
            Self::StartupTracing => "startup tracing disables hot commands",
            Self::CustomRunner => "custom runner hot-command support is unknown",
            Self::UnsupportedMode => "launch mode is not a supported debug mode",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotCapabilities {
    Supported,
    Unsupported(HotExclusion),
}

impl HotCapabilities {
    /// Record this value when a run is launched, before configuration reloads.
    /// `custom_runner` identifies a runner whose stdin command support is unknown;
    /// it must not be inferred merely from a shell wrapper or configuration name.
    pub fn from_launch(config: &LaunchConfig, custom_runner: bool) -> Self {
        if custom_runner {
            return Self::Unsupported(HotExclusion::CustomRunner);
        }
        if config.is_test() {
            return Self::Unsupported(HotExclusion::Test);
        }
        let args = || config.tool_args.iter().chain(&config.args);
        // Match LaunchConfig's mode argument semantics, including =true. Any
        // non-debug mode excludes the run even if another flag claims debug.
        if config.flutter_mode.as_deref() == Some("release")
            || args().any(|arg| matches!(arg.as_str(), "--release" | "--release=true"))
        {
            return Self::Unsupported(HotExclusion::Release);
        }
        if config.flutter_mode.as_deref() == Some("profile")
            || args().any(|arg| matches!(arg.as_str(), "--profile" | "--profile=true"))
        {
            return Self::Unsupported(HotExclusion::Profile);
        }
        if config
            .flutter_mode
            .as_deref()
            .is_some_and(|mode| mode != "debug")
        {
            return Self::Unsupported(HotExclusion::UnsupportedMode);
        }
        // Capability detection is conservative, not a replay of Flutter's
        // last-option-wins parser: any hot-incompatible occurrence in either
        // source excludes the launch, even if another option reverses it.
        // Explicit --machine=false / --trace-startup=false and --resident=true
        // are harmless alone, but cannot cancel an incompatible occurrence.
        if args().any(|arg| matches!(arg.as_str(), "--no-hot" | "--hot=false")) {
            return Self::Unsupported(HotExclusion::HotDisabled);
        }
        if args().any(|arg| matches!(arg.as_str(), "--machine" | "--machine=true")) {
            return Self::Unsupported(HotExclusion::MachineMode);
        }
        if args().any(|arg| matches!(arg.as_str(), "--no-resident" | "--resident=false")) {
            return Self::Unsupported(HotExclusion::NonResident);
        }
        if args().any(|arg| matches!(arg.as_str(), "--trace-startup" | "--trace-startup=true")) {
            return Self::Unsupported(HotExclusion::StartupTracing);
        }
        Self::Supported
    }
}

/// An invocation-time snapshot. IDs identify runs, never mutable vector slots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchTarget {
    pub id: u64,
    pub device: String,
    pub capabilities: HotCapabilities,
    pub live: bool,
    pub stopping: bool,
}

impl BatchTarget {
    pub fn exclusion(&self) -> Option<SkipReason> {
        if !self.live {
            Some(SkipReason::Ended)
        } else if self.stopping {
            Some(SkipReason::Stopping)
        } else {
            match self.capabilities {
                HotCapabilities::Supported => None,
                HotCapabilities::Unsupported(reason) => Some(SkipReason::Unsupported(reason)),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    Ended,
    Stopping,
    Unsupported(HotExclusion),
    Duplicate,
}

impl SkipReason {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Ended => "session has ended",
            Self::Stopping => "session is stopping",
            Self::Unsupported(reason) => reason.reason(),
            Self::Duplicate => "session was already included in this request",
        }
    }
}

/// Fixed categories prevent opaque I/O errors or command data leaking into UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendFailure {
    SessionUnavailable,
    InputRejected,
}

impl SendFailure {
    pub fn reason(self) -> &'static str {
        match self {
            Self::SessionUnavailable => "session is no longer available for input",
            Self::InputRejected => "session could not queue the input request",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetOutcome {
    Requested,
    Skipped(SkipReason),
    Failed(SendFailure),
}

impl TargetOutcome {
    pub fn detail(self) -> &'static str {
        match self {
            Self::Requested => "input requested/queued; Flutter completion is unconfirmed",
            Self::Skipped(reason) => reason.reason(),
            Self::Failed(reason) => reason.reason(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetDetail {
    pub id: u64,
    pub device: String,
    pub outcome: TargetOutcome,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BatchReport {
    /// Successful input submissions, not completed Flutter operations.
    pub requested: usize,
    pub skipped: usize,
    pub failed: usize,
    /// Bounded to the application's current eight-run limit. Counts cover all
    /// input snapshots even if a caller provides more than eight entries.
    pub details: Vec<TargetDetail>,
}

impl BatchReport {
    pub fn summary(&self, command: HotCommand) -> String {
        format!(
            "{}: {} requested/queued, {} skipped, {} failed. Flutter completion is unconfirmed.",
            command.label(),
            self.requested,
            self.skipped,
            self.failed
        )
    }
}

/// Dispatch once per eligible stable identity and keep going after a failed send.
/// The caller resolves each identity again before sending: a disappeared or
/// newly stopping session should return `SessionUnavailable`, never be replaced
/// by whatever now occupies its former vector position.
pub fn dispatch(
    command: HotCommand,
    targets: &[BatchTarget],
    mut send: impl FnMut(u64, &[u8]) -> Result<(), SendFailure>,
) -> BatchReport {
    let mut report = BatchReport {
        details: Vec::with_capacity(targets.len().min(MAX_DETAILS)),
        ..BatchReport::default()
    };
    for (index, target) in targets.iter().enumerate() {
        let outcome = if targets[..index].iter().any(|prior| prior.id == target.id) {
            TargetOutcome::Skipped(SkipReason::Duplicate)
        } else if let Some(reason) = target.exclusion() {
            TargetOutcome::Skipped(reason)
        } else {
            match send(target.id, command.bytes()) {
                Ok(()) => TargetOutcome::Requested,
                Err(reason) => TargetOutcome::Failed(reason),
            }
        };
        match outcome {
            TargetOutcome::Requested => report.requested += 1,
            TargetOutcome::Skipped(_) => report.skipped += 1,
            TargetOutcome::Failed(_) => report.failed += 1,
        }
        if report.details.len() < MAX_DETAILS {
            report.details.push(TargetDetail {
                id: target.id,
                device: bounded_device_label(&target.device),
                outcome,
            });
        }
    }
    report
}
