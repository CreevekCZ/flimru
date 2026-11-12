use crate::{batch_commands::HotCapabilities, config::LaunchConfig};
use anyhow::{Context, Result, bail};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug)]
pub struct FlutterTool {
    pub executable: PathBuf,
    pub prefix: Vec<String>,
    pub workspace: PathBuf,
}

#[derive(Clone, Debug)]
pub struct RunCommand {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: std::collections::BTreeMap<String, String>,
    pub capture_vm_service: bool,
    pub hot_capabilities: HotCapabilities,
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|path| is_executable(path))
            .and_then(|path| std::fs::canonicalize(path).ok())
    })
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

impl FlutterTool {
    pub fn resolve(workspace: &Path, explicit: Option<&Path>, fvm: bool) -> Result<Self> {
        let (executable, prefix) = if let Some(path) = explicit {
            let resolved = if path.components().count() == 1 && !path.is_absolute() {
                on_path(path.to_string_lossy().as_ref())
                    .context("Flutter executable not found in PATH")?
            } else {
                std::fs::canonicalize(path)
                    .with_context(|| format!("Flutter executable not found: {}", path.display()))?
            };
            (resolved, vec![])
        } else if fvm
            || workspace.join(".fvmrc").is_file()
            || workspace.join(".fvm/fvm_config.json").is_file()
        {
            // FVM is resolved from the workspace, so a nested launch cwd uses the same SDK.
            let sdk = workspace.join(".fvm/flutter_sdk/bin/flutter");
            if sdk.is_file() {
                (std::fs::canonicalize(sdk)?, vec![])
            } else {
                (on_path("fvm").context("This project uses FVM, but fvm is not in PATH. Install FVM or pass --flutter /path/to/flutter.")?, vec!["flutter".into()])
            }
        } else {
            (
                on_path("flutter").context(
                    "flutter is not in PATH. Install Flutter or pass --flutter /path/to/flutter.",
                )?,
                vec![],
            )
        };
        if !is_executable(&executable) {
            bail!("Flutter tool is not executable: {}", executable.display());
        }
        Ok(Self {
            executable,
            prefix,
            workspace: workspace.to_path_buf(),
        })
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command.args(&self.prefix).current_dir(&self.workspace);
        command
    }

    pub fn plan(
        &self,
        config: &LaunchConfig,
        device: &str,
        ownit_compat: bool,
    ) -> Result<RunCommand> {
        if !config.cwd.is_dir() {
            bail!("Working directory does not exist: {}", config.cwd.display());
        }
        let flutter_args = config.flutter_args(device)?;
        let custom_runner = ownit_compat && device == "mac-designed-for-ipad";
        let capture_vm_service = !config.is_test()
            && !flutter_args
                .iter()
                .any(|arg| matches!(arg.as_str(), "--release" | "--release=true"))
            && !custom_runner;
        let mut executable = self.executable.clone();
        let mut args = self.prefix.clone();
        args.extend(flutter_args);

        if custom_runner {
            let helper = self.workspace.join("scripts/run_mac_designed_for_ipad.sh");
            if !helper.is_file() {
                bail!("Designed for iPad runner is missing: {}", helper.display());
            }
            if config.is_test() {
                bail!("Designed for iPad runner does not support tests");
            }
            // The helper accepts flutter build flags and owns device selection.
            args = config.flutter_args(device)?;
            args.remove(0);
            if let Some(index) = args.iter().position(|arg| arg == "--device-id") {
                args.drain(index..=index + 1);
            }
            executable = PathBuf::from("/bin/bash");
            args.insert(0, helper.to_string_lossy().into_owned());
        }

        if ownit_compat {
            let guard = self.workspace.join("scripts/check_secrets.sh");
            let mut files = Vec::new();
            for (index, arg) in args.iter().enumerate() {
                if let Some(file) = arg.strip_prefix("--dart-define-from-file=") {
                    files.push(file.to_owned());
                } else if arg == "--dart-define-from-file" {
                    files.push(
                        args.get(index + 1)
                            .context("--dart-define-from-file needs a path")?
                            .clone(),
                    );
                }
            }
            if !files.is_empty() {
                if !guard.is_file() {
                    bail!("OWNIT secrets guard is missing: {}", guard.display());
                }
                // Fixed shell program; all project paths and Flutter arguments are positional.
                // Never interpolate launch.json text into shell source.
                let script = "set -e\nguard=$1; count=$2; shift 2\nwhile [ \"$count\" -gt 0 ]; do /bin/bash \"$guard\" \"$1\"; shift; count=$((count - 1)); done\nexec \"$@\"";
                let mut wrapped = vec![
                    "-c".into(),
                    script.into(),
                    "flimru".into(),
                    guard.to_string_lossy().into_owned(),
                    files.len().to_string(),
                ];
                wrapped.extend(
                    files
                        .into_iter()
                        .map(|file| config.cwd.join(file).to_string_lossy().into_owned()),
                );
                wrapped.push(executable.to_string_lossy().into_owned());
                wrapped.extend(args);
                executable = PathBuf::from("/bin/sh");
                args = wrapped;
            }
        }
        Ok(RunCommand {
            executable,
            args,
            cwd: config.cwd.clone(),
            env: config.env.clone(),
            capture_vm_service,
            hot_capabilities: HotCapabilities::from_launch(config, custom_runner),
        })
    }
}

pub fn has_ownit_helpers(workspace: &Path) -> bool {
    workspace.join("scripts/check_secrets.sh").is_file()
        && workspace
            .join("scripts/run_mac_designed_for_ipad.sh")
            .is_file()
}

impl RunCommand {
    /// Display only. The actual command always uses separate argument tokens.
    pub fn display(&self) -> String {
        let mut redact_next = false;
        std::iter::once(self.executable.as_os_str())
            .chain(self.args.iter().map(OsStr::new))
            .map(|arg| {
                let value = arg.to_string_lossy();
                if redact_next {
                    redact_next = false;
                    return "'<redacted>'".into();
                }
                if value == "--dart-define" {
                    redact_next = true;
                }
                if value.contains("--dart-define=") {
                    return "'--dart-define=<redacted>'".into();
                }
                format!("'{}'", value.replace('\'', "'\\''"))
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LaunchFile;

    #[test]
    fn vm_service_capture_is_only_enabled_for_supported_app_runs() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path();
        let launch = LaunchFile::parse(
            r#"{"configurations":[
            {"name":"Debug"},
            {"name":"Profile","flutterMode":"profile"},
            {"name":"Release","args":["--release"]},
            {"name":"Explicit release","args":["--release=true"]},
            {"name":"Tests","program":"test"}
        ]}"#,
            &workspace.join("launch.json"),
            workspace,
        )
        .unwrap();
        let tool = FlutterTool {
            executable: "/bin/echo".into(),
            prefix: vec![],
            workspace: workspace.into(),
        };
        for (config, expected) in launch
            .configurations
            .iter()
            .zip([true, true, false, false, false])
        {
            assert_eq!(
                tool.plan(config, "device", false)
                    .unwrap()
                    .capture_vm_service,
                expected,
                "{}",
                config.name
            );
        }
        std::fs::create_dir(workspace.join("scripts")).unwrap();
        std::fs::write(workspace.join("scripts/run_mac_designed_for_ipad.sh"), "").unwrap();
        assert!(
            !tool
                .plan(&launch.configurations[0], "mac-designed-for-ipad", true)
                .unwrap()
                .capture_vm_service
        );
    }

    #[test]
    fn display_redacts_split_and_joined_defines() {
        let command = RunCommand {
            capture_vm_service: false,
            hot_capabilities: crate::batch_commands::HotCapabilities::Supported,
            executable: "flutter".into(),
            args: vec![
                "run".into(),
                "--dart-define=SECRET=one".into(),
                "--dart-define".into(),
                "OTHER=two".into(),
            ],
            cwd: ".".into(),
            env: Default::default(),
        };
        let display = command.display();
        assert!(!display.contains("one"));
        assert!(!display.contains("two"));
        assert!(display.contains("redacted"));
    }

    #[test]
    fn hot_capabilities_follow_launch_options_and_actual_adapter_not_tool_names_or_guards() {
        use crate::batch_commands::{HotCapabilities::*, HotExclusion};
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path();
        std::fs::create_dir(workspace.join("scripts")).unwrap();
        for script in ["run_mac_designed_for_ipad.sh", "check_secrets.sh"] {
            std::fs::write(workspace.join("scripts").join(script), "").unwrap();
        }
        let launch = LaunchFile::parse(
            r#"{"configurations":[
            {"name":"Release test profile custom label"},
            {"name":"Guarded","args":["--dart-define-from-file=secret.env"]},
            {"name":"Profile","flutterMode":"profile"},
            {"name":"Tests","program":"test"}
        ]}"#,
            &workspace.join("launch.json"),
            workspace,
        )
        .unwrap();
        for prefix in [vec![], vec!["flutter".into()]] {
            let tool = FlutterTool {
                executable: workspace.join("custom-compatible-cli"),
                prefix,
                workspace: workspace.into(),
            };
            for (index, expected) in [
                (0, Supported),
                (1, Supported),
                (2, Unsupported(HotExclusion::Profile)),
                (3, Unsupported(HotExclusion::Test)),
            ] {
                assert_eq!(
                    tool.plan(&launch.configurations[index], "normal-device", true)
                        .unwrap()
                        .hot_capabilities,
                    expected
                );
            }
            assert_eq!(
                tool.plan(&launch.configurations[0], "mac-designed-for-ipad", true)
                    .unwrap()
                    .hot_capabilities,
                Unsupported(HotExclusion::CustomRunner)
            );
            assert_eq!(
                tool.plan(&launch.configurations[0], "mac-designed-for-ipad", false)
                    .unwrap()
                    .hot_capabilities,
                Supported
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn guard_keeps_arguments_literal_and_failure_blocks_flutter() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path();
        std::fs::create_dir(workspace.join("scripts")).unwrap();
        let guard = workspace.join("scripts/check_secrets.sh");
        std::fs::write(&guard, "printf '%s' \"$1\" > \"$GUARD_LOG\"\n").unwrap();
        let marker = workspace.join("MUST_NOT_EXIST");
        let log = workspace.join("guard.log");
        let raw = serde_json::json!({"configurations":[{"name":"guarded","args":["--dart-define-from-file", "secrets/a file.env", format!("--dart-define=PAYLOAD=$(touch {})", marker.display())], "env":{"GUARD_LOG":log}}]}).to_string();
        let launch = LaunchFile::parse(&raw, &workspace.join("launch.json"), workspace).unwrap();
        let tool = FlutterTool {
            executable: "/bin/echo".into(),
            prefix: vec![],
            workspace: workspace.into(),
        };
        let plan = tool
            .plan(&launch.configurations[0], "device with spaces", true)
            .unwrap();
        let run = || {
            Command::new(&plan.executable)
                .args(&plan.args)
                .envs(&plan.env)
                .current_dir(&plan.cwd)
                .output()
                .unwrap()
        };
        let output = run();
        assert!(output.status.success());
        assert!(!marker.exists());
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            workspace.join("secrets/a file.env").to_string_lossy()
        );
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("--device-id device with spaces"));
        assert!(text.contains("PAYLOAD=$(touch"));
        std::fs::write(&guard, "echo guard-blocked >&2\nexit 9\n").unwrap();
        let blocked = run();
        assert_eq!(blocked.status.code(), Some(9));
        assert!(blocked.stdout.is_empty());
    }
}
