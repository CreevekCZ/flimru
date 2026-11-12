#![allow(clippy::unwrap_used, clippy::expect_used)]
use flimru::{availability::assess, config::LaunchFile, device::Device, tool::FlutterTool};

fn target(id: &str, platform: &str) -> Device {
    Device {
        id: id.into(),
        name: format!("Target {id}"),
        platform: platform.into(),
        available: true,
        favorite: false,
    }
}

#[test]
fn catalog_retains_rejected_source_order_and_safe_field_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let raw = r#"{"configurations":[
        {"name":"Duplicate", "type":"SECRET_DEBUGGER"},
        {"name":"Duplicate", "args":["--dart-define=TOKEN=TOP_SECRET"]},
        42,
        {"name":"Bad request", "request":"SECRET_REQUEST"},
        {"name":"Variable", "args":["${input:flavor}"]},
        {"name":"Unknown field", "futureFeature":"SECRET_VALUE"}
    ], "compounds":[{"name":"Compound"}]}"#;
    let launch =
        LaunchFile::parse(raw, &directory.path().join("launch.json"), directory.path()).unwrap();
    assert_eq!(launch.configurations.len(), 2);
    assert_eq!(launch.entries.len(), 6);
    assert_eq!(
        launch
            .entries
            .iter()
            .map(|entry| entry.source_index)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4, 5]
    );
    assert_eq!(launch.entries[0].name, "Duplicate");
    assert_eq!(launch.entries[1].name, "Duplicate");
    assert_eq!(launch.entries[1].configuration, Some(0));
    assert_eq!(launch.entries[2].name, "Configuration 3");
    assert!(launch.entries[5].configuration.is_some());
    assert!(
        launch
            .warnings
            .iter()
            .any(|warning| warning.contains("futureFeature") && warning.contains("ignored"))
    );
    assert!(launch.entries[0].diagnostics[0].message.contains("type"));
    assert!(
        launch.entries[4].diagnostics[0]
            .message
            .contains("input:flavor")
    );
    let diagnostics = format!("{:?} {:?}", launch.entries, launch.warnings);
    assert!(!diagnostics.contains("SECRET"), "{diagnostics}");
    assert!(diagnostics.contains("Compound"));
}

#[test]
fn no_target_still_reports_source_cwd_arguments_and_tasks_without_values() {
    let directory = tempfile::tempdir().unwrap();
    let launch = LaunchFile::parse(
        r#"{"configurations":[
        {"name":"Bad cwd", "cwd":"${env:PATH}"},
        {"name":"Bad args", "args":["--dart-define=SECRET=value", "--device-id"]},
        {"name":"Task", "preLaunchTask":"${env:PATH}"},
        {"name":"Good"}
    ]}"#,
        &directory.path().join("launch.json"),
        directory.path(),
    )
    .unwrap();
    let tool = FlutterTool {
        executable: "/bin/echo".into(),
        prefix: vec![],
        workspace: directory.path().into(),
    };
    for (entry, code) in launch
        .entries
        .iter()
        .zip(["cwd", "arguments", "preLaunchTask", ""])
    {
        let assessment = assess(&launch, entry, &tool, &[], false);
        assert!(assessment.target_required);
        assert!(!assessment.runnable());
        assert!(assessment.plans.is_empty());
        if !code.is_empty() {
            assert_eq!(assessment.reasons[0].code, code);
        }
        let text = format!("{:?}", assessment.reasons);
        assert!(!text.contains(&std::env::var("PATH").unwrap()));
        assert!(!text.contains("SECRET=value"));
    }
}

#[test]
fn all_targets_share_preflight_and_receive_target_specific_reasons() {
    let directory = tempfile::tempdir().unwrap();
    let launch = LaunchFile::parse(
        r#"{"configurations":[
        {"name":"Pinned", "deviceId":"phone"},
        {"name":"Tests", "program":"test"},
        {"name":"Flavor", "args":["--flavor", "devSolana"]},
        {"name":"Shared"}
    ]}"#,
        &directory.path().join("launch.json"),
        directory.path(),
    )
    .unwrap();
    let tool = FlutterTool {
        executable: "/bin/echo".into(),
        prefix: vec![],
        workspace: directory.path().into(),
    };
    let phone = target("phone", "android");
    let other = target("other", "ios");
    let tester = target("flutter-tester", "tester");
    let reasons = assess(&launch, &launch.entries[0], &tool, &[&phone, &other], false).reasons;
    assert_eq!(reasons.len(), 1);
    assert_eq!(reasons[0].target.as_deref(), Some("Target other"));
    assert!(!assess(&launch, &launch.entries[1], &tool, &[&phone, &other], false).runnable());
    assert!(assess(&launch, &launch.entries[1], &tool, &[&tester], false).runnable());
    assert!(!assess(&launch, &launch.entries[3], &tool, &[&tester], false).runnable());
    assert!(!assess(&launch, &launch.entries[0], &tool, &[&tester], false).runnable());
    let flavor = assess(&launch, &launch.entries[2], &tool, &[&phone, &other], true);
    assert_eq!(flavor.reasons[0].code, "adapter");
    assert_eq!(flavor.reasons[0].target.as_deref(), Some("Target other"));
    let shared = assess(&launch, &launch.entries[3], &tool, &[&phone, &other], false);
    assert!(shared.runnable());
    assert_eq!(shared.plans.len(), 2);
    assert_eq!(shared.plans[0].args, ["run", "--device-id", "phone"]);
    assert_eq!(shared.plans[1].args, ["run", "--device-id", "other"]);
    let macos = target("macos", "macos");
    assert!(!assess(&launch, &launch.entries[3], &tool, &[&macos], true).runnable());
    let designed = target("mac-designed-for-ipad", "ios");
    assert!(!assess(&launch, &launch.entries[3], &tool, &[&designed], true).runnable());
}
