//! Check hangups without consuming input. A lost terminal must never keep the
//! input reader or its Flutter children running in the background.
#[cfg(unix)]
pub fn connected() -> std::io::Result<bool> {
    Ok(connected_fd(libc::STDIN_FILENO)? && connected_fd(libc::STDOUT_FILENO)?)
}

/// Ratatui's Terminal destructor tries to show its cursor and logs errors to
/// stderr. Both streams can already be dead after a PTY hangup; redirect this
/// exiting process's output to a sink so teardown cannot recursively panic.
#[cfg(unix)]
pub fn detach_output() -> std::io::Result<()> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    let opened = std::fs::OpenOptions::new().write(true).open("/dev/null")?;
    // Keep the sink outside standard descriptors even if one was already
    // closed. Dropping the opened File must not close a newly redirected fd.
    // SAFETY: opened owns the live source fd. F_DUPFD_CLOEXEC takes an integer
    // lower bound and returns a new independently owned descriptor.
    let fd = unsafe { libc::fcntl(opened.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: fd is the successful duplication result, has no other owner, and
    // is transferred exactly once into OwnedFd for automatic closing.
    let sink = unsafe { OwnedFd::from_raw_fd(fd) };
    drop(opened);
    for fd in [libc::STDOUT_FILENO, libc::STDERR_FILENO] {
        // SAFETY: sink remains live, and dup2 accepts these standard descriptor
        // numbers even when closed. It does not transfer ownership of sink.
        if unsafe { libc::dup2(sink.as_raw_fd(), fd) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn detach_output() -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(unix))]
pub fn connected() -> std::io::Result<bool> {
    use std::io::IsTerminal;
    Ok(std::io::stdin().is_terminal() && std::io::stdout().is_terminal())
}

#[cfg(unix)]
fn connected_fd(fd: std::os::fd::RawFd) -> std::io::Result<bool> {
    let mut descriptor = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: descriptor is initialized and writable for one pollfd throughout
    // this synchronous call. The timeout is zero, and no pointer is retained.
    let ready = unsafe { libc::poll(&mut descriptor, 1, 0) };
    if ready < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            return Ok(true);
        }
        return Err(error);
    }
    // SAFETY: isatty takes only an integer and safely reports invalid fds; it
    // borrows no Rust memory and does not close or retain the descriptor.
    let terminal = unsafe { libc::isatty(fd) } == 1;
    Ok(descriptor.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) == 0 && terminal)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;
    #[test]
    fn closed_and_non_terminal_descriptors_are_disconnected() {
        assert!(!connected_fd(-1).unwrap());
        let file = tempfile::tempfile().unwrap();
        assert!(!connected_fd(file.as_raw_fd()).unwrap());
    }
}
