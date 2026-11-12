//! Shared, secret-safe presentation and launch preflight.
use crate::{
    config::{Diagnostic, LaunchFile, SourceEntry},
    device::Device,
    tool::{FlutterTool, RunCommand},
};

#[derive(Clone)]
pub struct Assessment {
    pub pending: bool,
    pub reasons: Vec<Diagnostic>,
    pub target_required: bool,
    pub plans: Vec<RunCommand>,
}

impl Assessment {
    pub fn runnable(&self) -> bool {
        !self.pending && self.reasons.is_empty() && !self.target_required
    }
}

/// Target-independent checks still run without a selected target. No command is
/// executed here, and no argument, environment value, or expanded path is exposed.
pub fn assess(
    launch: &LaunchFile,
    entry: &SourceEntry,
    tool: &FlutterTool,
    targets: &[&Device],
    ownit: bool,
) -> Assessment {
    let mut assessment = Assessment {
        pending: false,
        reasons: entry.diagnostics.clone(),
        target_required: targets.is_empty(),
        plans: Vec::new(),
    };
    let Some(config) = entry
        .configuration
        .and_then(|i| launch.configurations.get(i))
    else {
        return assessment;
    };
    if !config.cwd.is_dir() {
        assessment.reasons.push(Diagnostic::new(
            "cwd",
            "cwd is not an existing directory. Correct cwd or create the directory.",
        ));
    }
    if config.pre_launch_task.is_some() {
        assessment.reasons.push(Diagnostic::new(
            "preLaunchTask",
            "preLaunchTask is unsupported. Run the task yourself, then remove this field.",
        ));
    } else if config.flutter_args("selected-device").is_err() {
        assessment.reasons.push(Diagnostic::new("arguments", "Invalid Flutter arguments. Check device flag values and conflicting debug/profile/release modes."));
    }
    if targets.len() > 1 && !config.needs_device() {
        assessment.reasons.push(Diagnostic::new(
            "multi-target",
            "Device-less tests run once. Select a single target or Flutter tests (no device).",
        ));
    }
    let flags: Vec<_> = config.tool_args.iter().chain(&config.args).collect();
    let solana = flags.iter().enumerate().any(|(i, flag)| {
        (flag.as_str() == "--flavor" && flags.get(i + 1).is_some_and(|f| f.contains("Solana")))
            || (flag.starts_with("--flavor=") && flag.contains("Solana"))
    });
    for target in targets {
        let mut block = |code, message| {
            let mut reason = Diagnostic::new(code, message);
            reason.target = Some(target.name.clone());
            assessment.reasons.push(reason);
        };
        if config.device_id.as_ref().is_some_and(|id| id != &target.id) {
            block(
                "deviceId",
                "deviceId pins a different target. Select that target or correct deviceId.",
            );
        }
        if target.id == "flutter-tester" && config.needs_device() {
            block(
                "device",
                "This configuration needs a device. Select a physical device or emulator.",
            );
        }
        if ownit && solana && !target.platform.starts_with("android") {
            block(
                "adapter",
                "The project adapter requires Android for this flavor.",
            );
        }
        if ownit && target.id == "macos" && config.device_id.as_deref() != Some("macos") {
            block(
                "adapter",
                "The project adapter requires an explicit macos deviceId for this target.",
            );
        }
        if ownit && target.id == "mac-designed-for-ipad" && config.is_test() {
            block(
                "adapter",
                "The Designed for iPad runner does not support tests.",
            );
        }
    }
    if assessment.reasons.is_empty() {
        // The same deterministic planner supplies the commands consumed by Start.
        // A placeholder validates target-independent helper/argument requirements.
        let ids: Vec<_> = if targets.is_empty() {
            vec!["selected-device"]
        } else {
            targets.iter().map(|device| device.id.as_str()).collect()
        };
        for id in ids {
            match tool.plan(config, id, ownit) {
                Ok(plan) if !targets.is_empty() => assessment.plans.push(plan),
                Ok(_) => {}
                Err(_) => {
                    let mut reason = Diagnostic::new(
                        "plan",
                        "Cannot plan this launch. Check argument values and required project helper scripts.",
                    );
                    reason.target = targets
                        .iter()
                        .find(|device| device.id == id)
                        .map(|device| device.name.clone());
                    assessment.reasons.push(reason);
                }
            }
        }
    }
    assessment
}
