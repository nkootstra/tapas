use std::io;
use std::os::fd::RawFd;
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;

pub fn outputs_are_tty() -> bool {
    // SAFETY: isatty only inspects the process's standard output descriptors.
    unsafe { libc::isatty(libc::STDOUT_FILENO) == 1 && libc::isatty(libc::STDERR_FILENO) == 1 }
}

pub fn stdout_is_tty() -> bool {
    // SAFETY: isatty only inspects the process's standard-output descriptor.
    unsafe { libc::isatty(libc::STDOUT_FILENO) == 1 }
}

pub fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: fcntl is called with a valid pipe descriptor and integer commands.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the descriptor remains owned by ChildStdout/ChildStderr; this only
    // adds the nonblocking status flag.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn poll_readable(stdout_fd: Option<RawFd>, stderr_fd: Option<RawFd>) -> io::Result<()> {
    let mut descriptors = [
        libc::pollfd {
            fd: stdout_fd.unwrap_or(-1),
            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
            revents: 0,
        },
        libc::pollfd {
            fd: stderr_fd.unwrap_or(-1),
            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
            revents: 0,
        },
    ];
    loop {
        // SAFETY: descriptors points to two initialized pollfd values for the
        // duration of this call.
        let result = unsafe { libc::poll(descriptors.as_mut_ptr(), descriptors.len() as _, 25) };
        if result >= 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

pub fn exit_code(status: ExitStatus) -> i32 {
    status
        .code()
        .or_else(|| status.signal().map(|signal| 128 + signal))
        .unwrap_or(1)
}
