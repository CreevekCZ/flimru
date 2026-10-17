//! Clipboard helpers have an owner and a deadline; no detached worker or pipe write.
use anyhow::{Context, Result, bail};
use std::{
    io::{Seek, SeekFrom, Write},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct Clipboard {
    job: Option<Job>,
}

struct Job {
    child: Child,
    started: Instant,
}

impl Drop for Job {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Clipboard {
    /// None means a native copy is pending; Some sends OSC52 to the terminal host.
    pub fn copy(&mut self, text: &str) -> Result<Option<String>> {
        if text.len() > crate::selection::MAX_COPY_BYTES {
            bail!("Selection exceeds the 128 KiB clipboard limit; select a smaller range.");
        }
        self.job.take();
        if cfg!(target_os = "macos")
            && std::env::var_os("SSH_CONNECTION").is_none()
            && std::env::var_os("SSH_TTY").is_none()
        {
            let mut command = Command::new("/usr/bin/pbcopy");
            command.env("LC_ALL", "en_US.UTF-8");
            self.start(command, text)?;
            Ok(None)
        } else {
            Ok(Some(text.to_owned()))
        }
    }

    fn start(&mut self, mut command: Command, text: &str) -> Result<()> {
        let mut input = tempfile::tempfile().context("Could not prepare clipboard input")?;
        input.write_all(text.as_bytes())?;
        input.seek(SeekFrom::Start(0))?;
        // A file instead of a pipe prevents large selections blocking the UI.
        let child = command
            .stdin(Stdio::from(input))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Could not start the system clipboard")?;
        self.job = Some(Job {
            child,
            started: Instant::now(),
        });
        Ok(())
    }

    pub fn tick(&mut self) -> Option<Result<()>> {
        let job = self.job.as_mut()?;
        let result = match job.child.try_wait() {
            Ok(Some(status)) if status.success() => Ok(()),
            Ok(Some(_)) => Err(anyhow::anyhow!(
                "The system clipboard could not copy the selection"
            )),
            Err(error) => Err(error.into()),
            Ok(None) if job.started.elapsed() >= Duration::from_secs(2) => {
                Err(anyhow::anyhow!("The system clipboard timed out"))
            }
            Ok(None) => return None,
        };
        self.job.take();
        Some(result)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn oversized_selection_is_rejected_without_spawning_a_process() {
        let mut clipboard = Clipboard::default();
        assert!(
            clipboard
                .copy(&"x".repeat(crate::selection::MAX_COPY_BYTES + 1))
                .is_err()
        );
        assert!(clipboard.job.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn helper_receives_literal_utf8_data_and_is_reaped() {
        let temporary = tempfile::tempdir().unwrap();
        let output = temporary.path().join("copied.txt");
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "cat > \"$1\"", "clipboard-test"])
            .arg(&output);
        let mut clipboard = Clipboard::default();
        let text = "界 e\u{301}\n$(not a command)\n".repeat(1000);
        clipboard.start(command, &text).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(result) = clipboard.tick() {
                result.unwrap();
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(std::fs::read_to_string(output).unwrap(), text);
        assert!(clipboard.job.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn dropping_or_timing_out_a_hung_helper_kills_and_reaps_it() {
        for timeout in [false, true] {
            let mut clipboard = Clipboard::default();
            let mut command = Command::new("/bin/sleep");
            command.arg("30");
            clipboard.start(command, "text").unwrap();
            let pid = clipboard.job.as_ref().unwrap().child.id();
            let started = Instant::now();
            if timeout {
                clipboard.job.as_mut().unwrap().started = Instant::now() - Duration::from_secs(3);
                assert!(clipboard.tick().unwrap().is_err());
            }
            drop(clipboard);
            assert!(started.elapsed() < Duration::from_secs(1));
            // SAFETY: signal zero inspects this owned helper PID without sending a signal.
            assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
        }
    }
}
