//! VS Code JSONC loading and shell-free Flutter command planning.
//!
//! `args` are treated as Flutter CLI arguments for compatibility with existing
//! launch.json based `make run` launchers. `toolArgs` are forwarded first.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};
use jsonc_parser::{ParseOptions, parse_to_serde_value};
use serde_json::{Map, Value};

#[derive(Clone, Debug)]
pub struct LaunchConfig {
    pub name: String,
    pub cwd: PathBuf,
    pub program: Option<String>,
    pub args: Vec<String>,
    pub tool_args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub flutter_mode: Option<String>,
    pub device_id: Option<String>,
    pub pre_launch_task: Option<String>,
    pub run_tests_on_device: bool,
}

#[derive(Clone, Debug)]
pub struct LaunchFile {
    pub path: PathBuf,
    pub workspace: PathBuf,
    pub configurations: Vec<LaunchConfig>,
    pub warnings: Vec<String>,
}

impl LaunchFile {
    pub fn load(path: &Path, workspace: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("Could not read {}", path.display()))?;
        Self::parse(&raw, path, workspace)
    }

    pub fn parse(raw: &str, path: &Path, workspace: &Path) -> Result<Self> {
        // VS Code JSONC permits comments and trailing commas, but not the other
        // loose JavaScript-like syntax accepted by this parser's defaults.
        let value: Value = parse_to_serde_value(
            raw.trim_start_matches('\u{feff}'),
            &ParseOptions {
                allow_comments: true,
                allow_trailing_commas: true,
                allow_loose_object_property_names: false,
                allow_missing_commas: false,
                allow_single_quoted_strings: false,
                allow_hexadecimal_numbers: false,
                allow_unary_plus_numbers: false,
            },
        )
        .with_context(|| format!("Invalid JSONC in {}", path.display()))?;
        let root = value
            .as_object()
            .context("launch.json must contain a JSON object")?;
        let entries = root
            .get("configurations")
            .and_then(Value::as_array)
            .context("launch.json must contain a configurations array")?;
        let workspace = absolute_path(workspace)?;
        let mut file = Self {
            path: absolute_path(path)?,
            workspace,
            configurations: Vec::new(),
            warnings: Vec::new(),
        };

        for (index, entry) in entries.iter().enumerate() {
            let label = entry
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Configuration {}", index + 1));
            match parse_configuration(entry, &file.workspace, &mut file.warnings, &label) {
                Ok(config) => file.configurations.push(config),
                Err(error) => file.warnings.push(format!("Skipped {label}: {error:#}")),
            }
        }
        if root
            .get("compounds")
            .is_some_and(|value| value.as_array().is_none_or(|entries| !entries.is_empty()))
        {
            file.warnings
                .push("Compound launch configurations are not supported.".into());
        }
        Ok(file)
    }
}

impl LaunchConfig {
    pub fn is_test(&self) -> bool {
        self.program_path().is_some_and(|program| {
            program == Path::new("test")
                || program.components().any(|part| {
                    matches!(part, Component::Normal(name) if name == "test" || name == "integration_test")
                })
                || program
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with("_test.dart"))
        })
    }

    fn program_path(&self) -> Option<&Path> {
        self.program.as_deref().map(|program| {
            let path = Path::new(program);
            // A workspace or ancestor named "test" must not turn every target
            // in that project into a test command.
            path.strip_prefix(&self.cwd).unwrap_or(path)
        })
    }

    pub fn flutter_args(&self, selected_device: &str) -> Result<Vec<String>> {
        if let Some(task) = &self.pre_launch_task {
            bail!(
                "{} requires preLaunchTask '{task}', which this launcher cannot execute. Run the task yourself and remove preLaunchTask from a dedicated launch configuration.",
                self.name
            );
        }
        ensure!(
            !selected_device.trim().is_empty() && !selected_device.contains('\0'),
            "A valid device must be selected"
        );
        let mut forwarded = strip_device_arguments(&self.tool_args)?;
        forwarded.extend(strip_device_arguments(&self.args)?);

        let mut modes = BTreeSet::new();
        for arg in &forwarded {
            if let Some(mode) = mode_argument(arg) {
                modes.insert(mode.to_owned());
            }
        }
        if let Some(mode) = &self.flutter_mode {
            ensure!(
                matches!(mode.as_str(), "debug" | "profile" | "release"),
                "Unsupported flutterMode '{mode}'"
            );
            modes.insert(mode.clone());
        }
        ensure!(
            modes.len() <= 1,
            "Conflicting Flutter modes in {}: {}",
            self.name,
            modes.into_iter().collect::<Vec<_>>().join(", ")
        );

        let is_test = self.is_test();
        let mut result = vec![if is_test { "test" } else { "run" }.into()];
        if self.needs_device() {
            result.extend(["--device-id".into(), selected_device.into()]);
        }
        if let Some(mode) = &self.flutter_mode
            && !forwarded
                .iter()
                .any(|arg| mode_argument(arg) == Some(mode.as_str()))
        {
            result.push(format!("--{mode}"));
        }
        if let Some(program) = &self.program {
            if is_test {
                if program != "test" {
                    result.push(program.clone());
                }
            } else {
                result.extend(["--target".into(), program.clone()]);
            }
        }
        result.extend(forwarded);
        Ok(result)
    }

    pub fn needs_device(&self) -> bool {
        !self.is_test()
            || self.run_tests_on_device
            || self.program_path().is_some_and(|program| {
                program.components().any(
                    |part| matches!(part, Component::Normal(name) if name == "integration_test"),
                )
            })
    }
}

fn parse_configuration(
    value: &Value,
    workspace: &Path,
    warnings: &mut Vec<String>,
    label: &str,
) -> Result<LaunchConfig> {
    let object = value.as_object().context("entry must be an object")?;
    let kind = optional_string(object, "type")?.unwrap_or_else(|| "dart".into());
    ensure!(kind == "dart", "unsupported debugger type '{kind}'");
    let request = optional_string(object, "request")?.unwrap_or_else(|| "launch".into());
    ensure!(request == "launch", "unsupported request '{request}'");

    for field in [
        "customTool",
        "customToolReplacesArgs",
        "vmAdditionalArgs",
        "dartAdditionalArgs",
    ] {
        if object.get(field).is_some_and(meaningful_value) {
            bail!("{field} is not supported; this configuration cannot be launched faithfully");
        }
    }
    for field in object.keys() {
        if !matches!(
            field.as_str(),
            "name"
                | "type"
                | "request"
                | "cwd"
                | "program"
                | "args"
                | "toolArgs"
                | "env"
                | "flutterMode"
                | "deviceId"
                | "preLaunchTask"
                | "runTestsOnDevice"
                | "presentation"
        ) {
            warnings.push(format!("{label}: unsupported field '{field}' is ignored."));
        }
    }

    let expand = |text: &str| expand_variables(text, workspace);
    let name = expand(&optional_string(object, "name")?.unwrap_or_else(|| label.to_owned()))?;
    ensure!(!name.trim().is_empty(), "name must not be empty");
    let cwd = optional_string(object, "cwd")?
        .map(|path| {
            ensure!(!path.trim().is_empty(), "cwd must not be empty");
            expand(&path)
        })
        .transpose()?
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                workspace.join(path)
            }
        })
        .unwrap_or_else(|| workspace.to_path_buf());
    let program = optional_string(object, "program")?
        .map(|program| {
            ensure!(!program.trim().is_empty(), "program must not be empty");
            expand(&program)
        })
        .transpose()?
        .map(|program| {
            if program == "test" || Path::new(&program).is_absolute() {
                program
            } else {
                cwd.join(program).to_string_lossy().into_owned()
            }
        });
    let args = argument_list(object, "args", workspace)?;
    let tool_args = argument_list(object, "toolArgs", workspace)?;
    let flutter_mode = optional_string(object, "flutterMode")?;
    if let Some(mode) = &flutter_mode {
        ensure!(
            matches!(mode.as_str(), "debug" | "profile" | "release"),
            "unsupported flutterMode '{mode}'"
        );
    }
    let device_id = optional_string(object, "deviceId")?
        .map(|device| expand(&device))
        .transpose()?;
    let pre_launch_task = optional_string(object, "preLaunchTask")?
        .map(|task| expand(&task))
        .transpose()?;
    if pre_launch_task.is_some() {
        warnings.push(format!(
            "{label}: preLaunchTask is not supported and will block launching."
        ));
    }
    let run_tests_on_device = match object.get("runTestsOnDevice") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => bail!("runTestsOnDevice must be a boolean"),
    };
    let mut env = BTreeMap::new();
    if let Some(value) = object.get("env").filter(|value| !value.is_null()) {
        for (key, value) in value.as_object().context("env must be an object")? {
            ensure!(
                !key.is_empty() && !key.contains(['=', '\0']),
                "env contains an invalid variable name"
            );
            let text = match value {
                Value::String(value) => value.clone(),
                Value::Bool(_) | Value::Number(_) => value.to_string(),
                Value::Null => bail!(
                    "env variable '{key}' is null; removing inherited environment variables is not supported"
                ),
                _ => bail!("env variable '{key}' must be a string, boolean, or number"),
            };
            env.insert(key.clone(), expand(&text)?);
        }
    }
    ensure!(!cwd.as_os_str().is_empty(), "cwd must not be empty");
    if let Some(program) = &program {
        ensure!(!program.is_empty(), "program must not be empty");
    }
    if let Some(device) = &device_id {
        ensure!(!device.trim().is_empty(), "deviceId must not be empty");
    }
    Ok(LaunchConfig {
        name,
        cwd,
        program,
        args,
        tool_args,
        env,
        flutter_mode,
        device_id,
        pre_launch_task,
        run_tests_on_device,
    })
}

fn meaningful_value(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::String(value) => !value.is_empty(),
        Value::Array(values) => !values.is_empty(),
        Value::Object(values) => !values.is_empty(),
        Value::Number(_) => true,
    }
}

fn optional_string(object: &Map<String, Value>, field: &str) -> Result<Option<String>> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => bail!("{field} must be a string"),
    }
}

fn argument_list(
    object: &Map<String, Value>,
    field: &str,
    workspace: &Path,
) -> Result<Vec<String>> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(value) => value
            .as_array()
            .with_context(|| format!("{field} must be an array of strings"))?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let value = value
                    .as_str()
                    .with_context(|| format!("{field}[{index}] must be a string"))?;
                expand_variables(value, workspace)
                    .with_context(|| format!("Could not expand {field}[{index}]"))
            })
            .collect(),
    }
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .context("Could not determine the current directory")?
            .join(path))
    }
}

fn expand_variables(text: &str, workspace: &Path) -> Result<String> {
    let mut remaining = text;
    let mut result = String::new();
    while let Some(start) = remaining.find("${") {
        result.push_str(&remaining[..start]);
        remaining = &remaining[start + 2..];
        let end = remaining
            .find('}')
            .context("Unterminated VS Code variable substitution")?;
        let variable = &remaining[..end];
        let expanded = match variable {
            "workspaceFolder" | "workspaceRoot" => workspace.to_string_lossy().into_owned(),
            "workspaceFolderBasename" => workspace
                .file_name()
                .context("workspaceFolderBasename is unavailable for this workspace")?
                .to_string_lossy()
                .into_owned(),
            name if name.starts_with("env:") => {
                let name = &name[4..];
                ensure!(!name.is_empty(), "Environment variable name is empty");
                std::env::var(name)
                    .map_err(|_| anyhow!("Environment variable '{name}' is missing or not UTF-8"))?
            }
            _ => bail!("Unsupported VS Code variable '${{{variable}}}'"),
        };
        result.push_str(&expanded);
        remaining = &remaining[end + 1..];
    }
    result.push_str(remaining);
    ensure!(!result.contains('\0'), "Value contains a NUL character");
    Ok(result)
}

fn strip_device_arguments(arguments: &[String]) -> Result<Vec<String>> {
    let mut result = Vec::new();
    let mut index = 0;
    while index < arguments.len() {
        let arg = &arguments[index];
        if matches!(arg.as_str(), "-d" | "--device-id") {
            let device = arguments
                .get(index + 1)
                .with_context(|| format!("{arg} requires a device ID"))?;
            ensure!(
                !device.is_empty() && !device.starts_with('-'),
                "{arg} requires a device ID"
            );
            index += 2;
        } else if let Some(device) = arg
            .strip_prefix("--device-id=")
            .or_else(|| arg.strip_prefix("-d="))
        {
            ensure!(!device.is_empty(), "{arg} requires a device ID");
            index += 1;
        } else {
            result.push(arg.clone());
            index += 1;
        }
    }
    Ok(result)
}

fn mode_argument(argument: &str) -> Option<&str> {
    match argument {
        "--debug" | "--debug=true" => Some("debug"),
        "--profile" | "--profile=true" => Some("profile"),
        "--release" | "--release=true" => Some("release"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> LaunchFile {
        LaunchFile::parse(
            raw,
            Path::new("/work/app/.vscode/launch.json"),
            Path::new("/work/app"),
        )
        .unwrap()
    }

    fn configuration(fields: &str) -> LaunchConfig {
        let file = parse(&format!("{{\"configurations\":[{{{fields}}}]}}"));
        assert!(file.warnings.is_empty(), "{:?}", file.warnings);
        file.configurations.into_iter().next().unwrap()
    }

    #[test]
    fn jsonc_preserves_urls_and_quoted_comment_markers() {
        let file = parse(
            r#"{
              // comment
              "configurations": [{
                "name": "URL // example",
                "args": ["--dart-define=URL=https://example.com/a/*b*/",],
              },],
            }"#,
        );
        assert_eq!(file.configurations[0].name, "URL // example");
        assert_eq!(
            file.configurations[0].args[0],
            "--dart-define=URL=https://example.com/a/*b*/"
        );
    }

    #[test]
    fn invalid_json_reports_file_and_position() {
        let error = LaunchFile::parse(
            "{\"configurations\": [}",
            Path::new("/work/launch.json"),
            Path::new("/work"),
        )
        .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("/work/launch.json"));
        assert!(message.contains("line 1"));
        assert!(
            LaunchFile::parse(
                "{\"configurations\": [{\"name\":\"a\" \"args\":[]}]}",
                Path::new("/work/launch.json"),
                Path::new("/work")
            )
            .is_err()
        );
    }

    #[test]
    fn argv_boundaries_are_preserved() {
        let config = configuration(
            r#""name":"dev", "args":["--flavor","dev name","--dart-define=VALUE=a b;$(danger)"], "toolArgs":["--verbose"]"#,
        );
        assert_eq!(
            config.flutter_args("phone one").unwrap(),
            [
                "run",
                "--device-id",
                "phone one",
                "--verbose",
                "--flavor",
                "dev name",
                "--dart-define=VALUE=a b;$(danger)"
            ]
        );
    }

    #[test]
    fn workspace_cwd_program_and_environment_expansion() {
        let env_name = "PATH";
        let config = configuration(&format!(
            r#""name":"widgetbook", "cwd":"${{workspaceRoot}}/widgetbook", "program":"lib/main.dart", "env":{{"APP":"${{workspaceFolderBasename}}", "WORKSPACE":"${{workspaceFolder}}", "INHERITED":"${{env:{env_name}}}", "BOOLEAN":true, "NUMBER":42}}"#
        ));
        assert_eq!(config.cwd, Path::new("/work/app/widgetbook"));
        assert_eq!(
            config.program.as_deref(),
            Some("/work/app/widgetbook/lib/main.dart")
        );
        assert_eq!(config.env["APP"], "app");
        assert_eq!(config.env["WORKSPACE"], "/work/app");
        assert_eq!(config.env["INHERITED"], std::env::var(env_name).unwrap());
        assert_eq!(config.env["BOOLEAN"], "true");
        assert_eq!(config.env["NUMBER"], "42");
        let relative = configuration(r#""name":"relative", "cwd":"widgetbook""#);
        assert_eq!(relative.cwd, Path::new("/work/app/widgetbook"));
    }

    #[test]
    fn unsupported_variables_and_missing_environment_skip_only_affected_entries() {
        let file = parse(
            r#"{"configurations":[
                {"name":"input", "args":["${input:flavor}"]},
                {"name":"command", "program":"${command:chooseFile}"},
                {"name":"missing", "env":{"VALUE":"${env:FLUTTER_RUN_TUI_MISSING_TEST_VARIABLE_928572}"}},
                {"name":"good"}
            ]}"#,
        );
        assert_eq!(file.configurations.len(), 1);
        assert_eq!(file.configurations[0].name, "good");
        assert_eq!(file.warnings.len(), 3);
        assert!(file.warnings[0].contains("input:flavor"));
        assert!(file.warnings[1].contains("command:chooseFile"));
        assert!(file.warnings[2].contains("missing"));
    }

    #[test]
    fn selected_device_replaces_all_configured_device_flags() {
        let config = configuration(
            r#""name":"device", "deviceId":"macos", "toolArgs":["-d","old","--device-id=other"], "args":["--device-id","third","-d=fourth","--verbose"]"#,
        );
        assert_eq!(config.device_id.as_deref(), Some("macos"));
        assert_eq!(
            config.flutter_args("selected").unwrap(),
            ["run", "--device-id", "selected", "--verbose"]
        );
        let malformed = configuration(r#""name":"bad", "args":["--device-id"]"#);
        assert!(malformed.flutter_args("selected").is_err());
    }

    #[test]
    fn flutter_mode_is_added_once_and_conflicts_are_rejected() {
        let config =
            configuration(r#""name":"profile", "flutterMode":"profile", "args":["--profile"]"#);
        assert_eq!(
            config.flutter_args("macos").unwrap(),
            ["run", "--device-id", "macos", "--profile"]
        );
        let config = configuration(r#""name":"release", "flutterMode":"release""#);
        assert!(
            config
                .flutter_args("macos")
                .unwrap()
                .contains(&"--release".into())
        );
        let conflicting =
            configuration(r#""name":"conflict", "flutterMode":"profile", "args":["--release"]"#);
        assert!(
            conflicting
                .flutter_args("macos")
                .unwrap_err()
                .to_string()
                .contains("Conflicting")
        );
        let flags = configuration(r#""name":"flags", "args":["--debug", "--release"]"#);
        assert!(flags.flutter_args("macos").is_err());
    }

    #[test]
    fn test_command_distinguishes_unit_tests_and_device_tests() {
        let all_tests = configuration(r#""name":"tests", "program":"test""#);
        assert!(all_tests.is_test());
        assert_eq!(all_tests.flutter_args("macos").unwrap(), ["test"]);
        let unit = configuration(r#""name":"unit", "program":"test/widget_test.dart""#);
        assert_eq!(
            unit.flutter_args("macos").unwrap(),
            ["test", "/work/app/test/widget_test.dart"]
        );
        let on_device =
            configuration(r#""name":"device tests", "program":"test", "runTestsOnDevice":true"#);
        assert_eq!(
            on_device.flutter_args("macos").unwrap(),
            ["test", "--device-id", "macos"]
        );
        let integration =
            configuration(r#""name":"integration", "program":"integration_test/app_test.dart""#);
        assert_eq!(
            integration.flutter_args("macos").unwrap(),
            [
                "test",
                "--device-id",
                "macos",
                "/work/app/integration_test/app_test.dart"
            ]
        );
    }

    #[test]
    fn ancestor_named_test_does_not_change_run_to_test() {
        let file = LaunchFile::parse(
            r#"{"configurations":[{"name":"run", "program":"lib/main.dart"}]}"#,
            Path::new("/work/test/app/.vscode/launch.json"),
            Path::new("/work/test/app"),
        )
        .unwrap();
        assert!(!file.configurations[0].is_test());
        assert_eq!(
            file.configurations[0].flutter_args("macos").unwrap()[0],
            "run"
        );
    }

    #[test]
    fn pre_launch_tasks_block_launch_and_other_types_are_skipped() {
        let file = parse(
            r#"{"configurations":[
                {"name":"task", "preLaunchTask":"build assets"},
                {"name":"attach", "request":"attach"},
                {"name":"node", "type":"node"},
                {"name":"vm", "vmAdditionalArgs":["--old_gen_heap_size=1024"]},
                {"name":"custom", "customTool":"other"}
            ]}"#,
        );
        assert_eq!(file.configurations.len(), 1);
        assert_eq!(file.warnings.len(), 5);
        assert!(file.configurations[0].flutter_args("macos").is_err());
    }

    #[test]
    fn null_environment_values_are_explicitly_rejected() {
        let file = parse(r#"{"configurations":[{"name":"null env", "env":{"SECRET":null}}]}"#);
        assert!(file.configurations.is_empty());
        assert!(file.warnings[0].contains("removing inherited environment variables"));
    }
}
