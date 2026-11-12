use flimru::batch_commands::{
    BatchTarget, HotCapabilities, HotCommand, HotExclusion, SendFailure, SkipReason, TargetOutcome,
    dispatch,
};
use flimru::config::LaunchConfig;
use std::{collections::BTreeMap, path::PathBuf};

fn config() -> LaunchConfig {
    LaunchConfig {
        name: "Release profile test (label is not a capability)".into(),
        cwd: PathBuf::from("/work/test/workspace"),
        program: Some("lib/main.dart".into()),
        args: vec![],
        tool_args: vec![],
        env: BTreeMap::new(),
        flutter_mode: None,
        device_id: None,
        pre_launch_task: None,
        run_tests_on_device: false,
    }
}

fn target(id: u64) -> BatchTarget {
    BatchTarget {
        id,
        device: format!("Device {id}"),
        capabilities: HotCapabilities::Supported,
        live: true,
        stopping: false,
    }
}

#[test]
fn command_bytes_and_labels_preserve_single_session_protocol() {
    assert_eq!(HotCommand::Reload.bytes(), b"r");
    assert_eq!(HotCommand::Restart.bytes(), b"R");
    assert_eq!(HotCommand::Reload.label(), "Hot reload");
    assert_eq!(HotCommand::Restart.label(), "Hot restart");
}

#[test]
fn implicit_and_explicit_debug_apps_are_supported() {
    let mut launch = config();
    assert_eq!(
        HotCapabilities::from_launch(&launch, false),
        HotCapabilities::Supported
    );
    launch.program = Some("/work/test/workspace/lib/main.dart".into());
    launch.flutter_mode = Some("debug".into());
    for flag in ["--debug", "--debug=true", "--hot", "--hot=true"] {
        launch.args = vec![flag.into()];
        assert_eq!(
            HotCapabilities::from_launch(&launch, false),
            HotCapabilities::Supported
        );
    }
}

#[test]
fn all_mode_sources_and_true_spellings_exclude_profile_and_release() {
    for (mode, reason) in [
        ("profile", HotExclusion::Profile),
        ("release", HotExclusion::Release),
    ] {
        let mut launch = config();
        launch.flutter_mode = Some(mode.into());
        assert_eq!(
            HotCapabilities::from_launch(&launch, false),
            HotCapabilities::Unsupported(reason)
        );
        for flag in [format!("--{mode}"), format!("--{mode}=true")] {
            for tool_args in [false, true] {
                let mut launch = config();
                if tool_args {
                    launch.tool_args.push(flag.clone());
                } else {
                    launch.args.push(flag.clone());
                }
                assert_eq!(
                    HotCapabilities::from_launch(&launch, false),
                    HotCapabilities::Unsupported(reason)
                );
                launch.flutter_mode = Some("debug".into());
                assert_eq!(
                    HotCapabilities::from_launch(&launch, false),
                    HotCapabilities::Unsupported(reason)
                );
            }
        }
    }
    let mut launch = config();
    launch.flutter_mode = Some("unknown".into());
    assert_eq!(
        HotCapabilities::from_launch(&launch, false),
        HotCapabilities::Unsupported(HotExclusion::UnsupportedMode)
    );
}

#[test]
fn false_mode_flags_match_current_launch_config_semantics() {
    let mut launch = config();
    launch.args = vec!["--release=false".into(), "--profile=false".into()];
    assert_eq!(
        HotCapabilities::from_launch(&launch, false),
        HotCapabilities::Supported
    );
}

#[test]
fn tests_including_device_and_integration_tests_are_excluded() {
    for program in [
        "test",
        "test/widget.dart",
        "lib/example_test.dart",
        "integration_test/app.dart",
        "/work/test/workspace/test/app.dart",
    ] {
        for run_on_device in [false, true] {
            let mut launch = config();
            launch.program = Some(program.into());
            launch.run_tests_on_device = run_on_device;
            assert!(launch.is_test());
            assert_eq!(
                HotCapabilities::from_launch(&launch, false),
                HotCapabilities::Unsupported(HotExclusion::Test)
            );
        }
    }
}

#[test]
fn custom_runners_and_hot_disabling_flags_are_excluded_conservatively() {
    assert_eq!(
        HotCapabilities::from_launch(&config(), true),
        HotCapabilities::Unsupported(HotExclusion::CustomRunner)
    );
    for flag in ["--no-hot", "--hot=false"] {
        for tool_args in [false, true] {
            let mut launch = config();
            let args = if tool_args {
                &mut launch.tool_args
            } else {
                &mut launch.args
            };
            args.extend([flag.into(), "--hot=true".into()]);
            assert_eq!(
                HotCapabilities::from_launch(&launch, false),
                HotCapabilities::Unsupported(HotExclusion::HotDisabled)
            );
        }
    }
}

#[test]
fn noninteractive_launch_options_never_receive_reload_or_restart() {
    for (flag, reason) in [
        ("--machine", HotExclusion::MachineMode),
        ("--machine=true", HotExclusion::MachineMode),
        ("--no-resident", HotExclusion::NonResident),
        ("--resident=false", HotExclusion::NonResident),
        ("--trace-startup", HotExclusion::StartupTracing),
        ("--trace-startup=true", HotExclusion::StartupTracing),
    ] {
        for tool_args in [false, true] {
            let mut launch = config();
            if tool_args {
                launch.tool_args.push(flag.into());
            } else {
                launch.args.push(flag.into());
            }
            // These ordinary launch.json options reach Flutter unchanged.
            let planned = launch.flutter_args("device").unwrap_or_default();
            assert!(planned.iter().any(|arg| arg == flag), "{flag}");
            let mut run = target(900);
            run.capabilities = HotCapabilities::from_launch(&launch, false);
            assert_eq!(run.capabilities, HotCapabilities::Unsupported(reason));
            for command in [HotCommand::Reload, HotCommand::Restart] {
                let report = dispatch(command, std::slice::from_ref(&run), |_, _| {
                    unreachable!("incompatible launch must never receive terminal input")
                });
                assert_eq!((report.requested, report.skipped, report.failed), (0, 1, 0));
                assert_eq!(
                    report.details[0].outcome,
                    TargetOutcome::Skipped(SkipReason::Unsupported(reason))
                );
            }
        }
    }
}

#[test]
fn explicit_interactive_options_preserve_default_and_debug_dispatch() {
    for mode in [None, Some("debug")] {
        for tool_args in [false, true] {
            let mut launch = config();
            launch.flutter_mode = mode.map(str::to_owned);
            let args = if tool_args {
                &mut launch.tool_args
            } else {
                &mut launch.args
            };
            args.extend(
                [
                    "--machine=false",
                    "--trace-startup=false",
                    "--resident=true",
                ]
                .map(str::to_owned),
            );
            let mut run = target(900);
            run.capabilities = HotCapabilities::from_launch(&launch, false);
            assert_eq!(run.capabilities, HotCapabilities::Supported);
            for command in [HotCommand::Reload, HotCommand::Restart] {
                let mut sends = vec![];
                let report = dispatch(command, std::slice::from_ref(&run), |id, bytes| {
                    sends.push((id, bytes.to_vec()));
                    Ok(())
                });
                assert_eq!(sends, [(900, command.bytes().to_vec())]);
                assert_eq!((report.requested, report.skipped, report.failed), (1, 0, 0));
            }
        }
    }
}

#[test]
fn incompatible_options_win_over_reversals_in_any_order_or_source() {
    for (incompatible, reversal, reason) in [
        (
            "--machine=true",
            "--machine=false",
            HotExclusion::MachineMode,
        ),
        (
            "--resident=false",
            "--resident=true",
            HotExclusion::NonResident,
        ),
        (
            "--trace-startup=true",
            "--trace-startup=false",
            HotExclusion::StartupTracing,
        ),
    ] {
        for reverse_order in [false, true] {
            let mut options = [incompatible.to_owned(), reversal.to_owned()];
            if reverse_order {
                options.reverse();
            }
            for placement in 0..3 {
                let mut launch = config();
                match placement {
                    0 => launch.args.extend(options.clone()),
                    1 => launch.tool_args.extend(options.clone()),
                    _ => {
                        launch.tool_args.push(options[0].clone());
                        launch.args.push(options[1].clone());
                    }
                }
                assert_eq!(
                    HotCapabilities::from_launch(&launch, false),
                    HotCapabilities::Unsupported(reason)
                );
            }
        }
    }
}

#[test]
fn capabilities_remain_a_launch_time_value() {
    let mut launch = config();
    let original = HotCapabilities::from_launch(&launch, false);
    launch.flutter_mode = Some("release".into());
    assert_eq!(original, HotCapabilities::Supported);
    assert_eq!(
        HotCapabilities::from_launch(&launch, false),
        HotCapabilities::Unsupported(HotExclusion::Release)
    );
}

#[test]
fn mixed_targets_skip_unsupported_ended_and_stopping_and_continue_after_failures() {
    let mut targets: Vec<_> = (1..=7).map(target).collect();
    targets[1].live = false;
    targets[2].stopping = true;
    targets[3].capabilities = HotCapabilities::Unsupported(HotExclusion::Test);
    let mut sends = vec![];
    let report = dispatch(HotCommand::Restart, &targets, |id, bytes| {
        sends.push((id, bytes.to_vec()));
        match id {
            5 => Err(SendFailure::InputRejected),
            6 => Err(SendFailure::SessionUnavailable),
            _ => Ok(()),
        }
    });
    assert_eq!(
        sends,
        [
            (1, b"R".to_vec()),
            (5, b"R".to_vec()),
            (6, b"R".to_vec()),
            (7, b"R".to_vec())
        ]
    );
    assert_eq!((report.requested, report.skipped, report.failed), (2, 3, 2));
    assert_eq!(
        report.details[1].outcome,
        TargetOutcome::Skipped(SkipReason::Ended)
    );
    assert_eq!(
        report.details[2].outcome,
        TargetOutcome::Skipped(SkipReason::Stopping)
    );
    assert_eq!(
        report.details[3].outcome,
        TargetOutcome::Skipped(SkipReason::Unsupported(HotExclusion::Test))
    );
    assert_eq!(
        report.details[4].outcome,
        TargetOutcome::Failed(SendFailure::InputRejected)
    );
    assert!(
        report
            .summary(HotCommand::Restart)
            .contains("2 requested/queued, 3 skipped, 2 failed")
    );
    assert!(
        report
            .summary(HotCommand::Restart)
            .contains("completion is unconfirmed")
    );
}

#[test]
fn each_identity_receives_one_request_and_no_vector_index_is_substituted() {
    let targets = [target(900), target(2), target(900), target(500)];
    let mut sends = vec![];
    let report = dispatch(HotCommand::Reload, &targets, |id, bytes| {
        sends.push((id, bytes.to_vec()));
        Ok(())
    });
    assert_eq!(
        sends,
        [
            (900, b"r".to_vec()),
            (2, b"r".to_vec()),
            (500, b"r".to_vec())
        ]
    );
    assert_eq!((report.requested, report.skipped, report.failed), (3, 1, 0));
    assert_eq!(
        report.details[2].outcome,
        TargetOutcome::Skipped(SkipReason::Duplicate)
    );
}

#[test]
fn no_targets_and_zero_eligible_targets_never_send() {
    let empty = dispatch(HotCommand::Reload, &[], |_, _| unreachable!());
    assert_eq!((empty.requested, empty.skipped, empty.failed), (0, 0, 0));
    let mut ended = target(11);
    ended.live = false;
    let mut unsupported = target(12);
    unsupported.capabilities = HotCapabilities::Unsupported(HotExclusion::Release);
    let report = dispatch(
        HotCommand::Reload,
        &[ended, unsupported],
        |_, _| unreachable!(),
    );
    assert_eq!((report.requested, report.skipped, report.failed), (0, 2, 0));
}

#[test]
fn snapshot_labels_mark_truncation_without_splitting_boundary_graphemes() {
    for (device, expected) in [
        (
            format!("{}🧑‍💻", "x".repeat(118)),
            format!("{}…", "x".repeat(118)),
        ),
        (
            format!("{}e\u{301}", "x".repeat(119)),
            format!("{}…", "x".repeat(119)),
        ),
        (
            format!("{}🧑‍💻", "x".repeat(117)),
            format!("{}🧑‍💻", "x".repeat(117)),
        ),
        (
            format!("{}e\u{301}", "x".repeat(118)),
            format!("{}e\u{301}", "x".repeat(118)),
        ),
        (format!("a{}", "\u{301}".repeat(130)), "…".to_owned()),
        (
            format!("\n{}\tTAIL\r", "x".repeat(120)),
            format!("{}…", "x".repeat(119)),
        ),
    ] {
        let mut run = target(900);
        run.device = device;
        let report = dispatch(HotCommand::Reload, &[run], |_, _| Ok(()));
        let label = &report.details[0].device;
        assert_eq!(label, &expected);
        assert!(label.chars().count() <= 120);
        assert!(!label.chars().any(char::is_control));
        assert_eq!(report.requested, 1);
    }
}

#[test]
fn per_device_details_are_control_free_and_bounded_with_fixed_error_messages() {
    let mut targets: Vec<_> = (0..12).map(target).collect();
    targets[0].device = format!("\x1b[31mDevice\n\r\t{}", "界".repeat(200));
    let report = dispatch(HotCommand::Reload, &targets, |_, _| {
        Err(SendFailure::InputRejected)
    });
    assert_eq!(report.failed, 12);
    assert_eq!(report.details.len(), 8);
    assert_eq!(report.details[0].device.chars().count(), 120);
    assert!(!report.details[0].device.chars().any(char::is_control));
    assert_eq!(
        report.details[0].outcome.detail(),
        "session could not queue the input request"
    );
    assert!(
        TargetOutcome::Requested
            .detail()
            .contains("completion is unconfirmed")
    );
    for reason in [
        HotExclusion::Test,
        HotExclusion::Profile,
        HotExclusion::Release,
        HotExclusion::HotDisabled,
        HotExclusion::MachineMode,
        HotExclusion::NonResident,
        HotExclusion::StartupTracing,
        HotExclusion::CustomRunner,
        HotExclusion::UnsupportedMode,
    ] {
        assert!(!reason.reason().is_empty());
    }
    assert_eq!(
        SendFailure::SessionUnavailable.reason(),
        "session is no longer available for input"
    );
}
