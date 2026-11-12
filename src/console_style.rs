use ratatui::style::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Severity {
    Error,
    Warning,
    Info,
    Debug,
    Config,
    Success,
}

impl Severity {
    fn color(self) -> Color {
        match self {
            Self::Error => Color::Rgb(255, 128, 142),
            Self::Warning => Color::Rgb(255, 205, 112),
            Self::Info => Color::Rgb(96, 210, 225),
            Self::Debug => Color::Rgb(151, 163, 180),
            Self::Config => Color::Rgb(205, 164, 235),
            Self::Success => Color::Rgb(128, 214, 159),
        }
    }
}

fn level(value: &str) -> Option<Severity> {
    const LEVELS: &[(&str, Severity)] = &[
        ("ERROR", Severity::Error),
        ("SEVERE", Severity::Error),
        ("FATAL", Severity::Error),
        ("SHOUT", Severity::Error),
        ("WARNING", Severity::Warning),
        ("WARN", Severity::Warning),
        ("INFO", Severity::Info),
        ("DEBUG", Severity::Debug),
        ("TRACE", Severity::Debug),
        ("FINE", Severity::Debug),
        ("FINER", Severity::Debug),
        ("FINEST", Severity::Debug),
        ("CONFIG", Severity::Config),
        ("SUCCESS", Severity::Success),
    ];
    LEVELS
        .iter()
        .find_map(|(label, severity)| value.eq_ignore_ascii_case(label).then_some(*severity))
}

/// Color recognized severity prefixes, never words buried in a message.
/// Incoming ANSI colors take precedence at the call site.
pub fn fallback_color(text: &str) -> Option<Color> {
    classify(text).map(Severity::color)
}

/// Semantic search filters use exactly the same recognized prefixes as colors.
pub fn review_severity(text: &str) -> Option<flimru::console_review::Severity> {
    match classify(text) {
        Some(Severity::Error) => Some(flimru::console_review::Severity::Error),
        Some(Severity::Warning) => Some(flimru::console_review::Severity::Warning),
        _ => None,
    }
}

fn classify(text: &str) -> Option<Severity> {
    let mut message = text.trim_start();
    let mut transport = None;
    if let Some(rest) = message.strip_prefix("flutter:") {
        message = rest.trim_start();
    } else if let Some((prefix, rest)) = message.split_once(':')
        && let Some((priority, tag)) = prefix.split_once('/')
        && !tag.is_empty()
        && prefix.len() <= 128
    {
        transport = match priority {
            "E" | "F" => Some(Severity::Error),
            "W" => Some(Severity::Warning),
            "I" => Some(Severity::Info),
            "D" | "V" => Some(Severity::Debug),
            _ => None,
        };
        if transport.is_some() {
            message = rest.trim_start();
        }
    }

    // Common forms: [WARNING] message, [LoggerName] WARNING: message.
    // Bound metadata handling, including [time] [logger] forms.
    for _ in 0..4 {
        if let Some(rest) = message.strip_prefix('[')
            && let Some((label, rest)) = rest.split_once(']')
        {
            if let Some(severity) = level(label.trim()) {
                return Some(severity);
            }
            message = rest.trim_start();
        } else {
            break;
        }
    }
    if let Some((label, _)) = message.split_once(':')
        && let Some(severity) = level(label.trim())
    {
        return Some(severity);
    }
    if message
        .trim_start_matches('═')
        .strip_prefix('╡')
        .is_some_and(|rest| rest.trim_start().starts_with("EXCEPTION CAUGHT BY"))
    {
        return Some(Severity::Error);
    }
    transport
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_app_severity_wins_over_android_transport_priority() {
        for message in [
            "flutter: [Orders] SEVERE: Request failed",
            "I/flutter ( 123): [Orders] SEVERE: Request failed",
            "I/flutter: [ERROR] Request failed",
            "[Orders] error: Request failed",
            "══╡ EXCEPTION CAUGHT BY WIDGETS LIBRARY ╞══",
        ] {
            assert_eq!(fallback_color(message), Some(Severity::Error.color()));
        }
        assert_eq!(
            fallback_color("W/ActivityManager(123): native warning"),
            Some(Severity::Warning.color())
        );
        assert_eq!(
            fallback_color("flutter: [Feed] INFO: Connected"),
            Some(Severity::Info.color())
        );
        assert_eq!(
            fallback_color("[time] [Feed] [debug] packet"),
            Some(Severity::Debug.color())
        );
    }

    #[test]
    fn ordinary_messages_urls_and_source_paths_stay_uncolored() {
        for message in [
            "request had no error",
            "https://example.com/error:42",
            "lib/error.dart:42:1",
            "[Orders] Received {error: null}",
            "Connected to the VM Service.",
        ] {
            assert_eq!(fallback_color(message), None);
        }
    }
    #[test]
    fn review_filters_share_prefix_precedence_and_leave_stack_lines_unclassified() {
        use flimru::console_review::Severity as ReviewSeverity;
        for (line, expected) in [
            ("\u{1b}[31m", None),
            (
                "I/flutter: [Orders] SEVERE: failed",
                Some(ReviewSeverity::Error),
            ),
            ("W/Tag: native warning", Some(ReviewSeverity::Warning)),
            (
                "[time] [Logger] WARNING: retry",
                Some(ReviewSeverity::Warning),
            ),
            ("INFO: useful context", None),
            ("  stack error at source.dart", None),
            ("WARN buried without recognized delimiter", None),
        ] {
            assert_eq!(review_severity(line), expected, "{line}");
        }
    }
}
