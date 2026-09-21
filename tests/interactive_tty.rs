#![cfg(unix)]

//! PTY-based coverage for the interactive (`--raw`) path.
//!
//! The wrapped child must be able to read from the controlling terminal. With
//! the child in a dedicated background process group the kernel stops it with
//! SIGTTIN on the first read and Tapas waits forever.

use std::ffi::CStr;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

static PTSNAME_LOCK: Mutex<()> = Mutex::new(());

struct TapasChild(Child);

impl Drop for TapasChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn slave_name(master: RawFd) -> std::io::Result<std::ffi::CString> {
    // `ptsname` returns a pointer into a static buffer, so serialize access.
    let _guard = PTSNAME_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let pointer = unsafe { libc::ptsname(master) };
    if pointer.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { CStr::from_ptr(pointer) }.to_owned())
}

fn open_pty() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let master = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
    if master == -1 {
        return Err(std::io::Error::last_os_error());
    }
    let master = unsafe { OwnedFd::from_raw_fd(master) };
    if unsafe { libc::grantpt(master.as_raw_fd()) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe { libc::unlockpt(master.as_raw_fd()) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    let name = slave_name(master.as_raw_fd())?;
    let slave = unsafe { libc::open(name.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
    if slave == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((master, unsafe { OwnedFd::from_raw_fd(slave) }))
}

fn disable_echo(slave: &OwnedFd) {
    let mut attributes = unsafe { std::mem::zeroed::<libc::termios>() };
    if unsafe { libc::tcgetattr(slave.as_raw_fd(), &mut attributes) } == 0 {
        attributes.c_lflag &= !libc::ECHO;
        let _ = unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &attributes) };
    }
}

fn spawn_tapas_on_pty(slave: &OwnedFd, args: &[&str]) -> std::io::Result<TapasChild> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tapas"));
    command.args(args);
    command.stdin(Stdio::from(slave.try_clone()?));
    command.stdout(Stdio::from(slave.try_clone()?));
    command.stderr(Stdio::from(slave.try_clone()?));
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::ioctl(0, libc::TIOCSCTTY as libc::c_ulong, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn().map(TapasChild)
}

fn read_until(master: std::fs::File, needle: &[u8], timeout: Duration) -> bool {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut master = master;
        let mut buffer = [0_u8; 1024];
        loop {
            match master.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    if sender.send(buffer[..read].to_vec()).is_err() {
                        break;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });

    let deadline = Instant::now() + timeout;
    let mut collected = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        match receiver.recv_timeout(remaining) {
            Ok(chunk) => {
                collected.extend_from_slice(&chunk);
                if collected
                    .windows(needle.len())
                    .any(|window| window == needle)
                {
                    return true;
                }
            }
            Err(_) => return false,
        }
    }
}

fn wait_with_deadline(child: &mut TapasChild, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.0.try_wait().expect("wait for tapas") {
            Some(status) => return Some(status),
            None if Instant::now() >= deadline => return None,
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

#[test]
fn interactive_child_reads_from_the_controlling_terminal() {
    let (master, slave) = open_pty().expect("open pty");
    disable_echo(&slave);
    let mut child =
        spawn_tapas_on_pty(&slave, &["--raw", "--", "/bin/cat"]).expect("spawn tapas on pty");
    drop(slave);

    let mut master = std::fs::File::from(master);
    let reader = master.try_clone().expect("clone pty master");
    master.write_all(b"hello terminal\n").expect("write to pty");
    master.flush().expect("flush pty");

    assert!(
        read_until(reader, b"hello terminal", Duration::from_secs(5)),
        "the wrapped child did not read from the terminal"
    );

    let _ = child.0.kill();
    let _ = child.0.wait();
}

#[test]
fn ctrl_c_reaches_the_child_and_tapas_reports_its_exit() {
    let (master, slave) = open_pty().expect("open pty");
    disable_echo(&slave);
    let mut child = spawn_tapas_on_pty(
        &slave,
        &["--raw", "--", "/bin/sh", "-c", "printf ready; exec cat"],
    )
    .expect("spawn tapas on pty");
    drop(slave);

    let mut master = std::fs::File::from(master);
    let reader = master.try_clone().expect("clone pty master");
    assert!(
        read_until(reader, b"ready", Duration::from_secs(5)),
        "tapas did not start the wrapped child"
    );

    master.write_all(&[0x03]).expect("send ctrl-c to pty");
    master.flush().expect("flush pty");

    let status =
        wait_with_deadline(&mut child, Duration::from_secs(5)).expect("tapas returns after ctrl-c");
    assert_eq!(
        status.code(),
        Some(130),
        "tapas should report SIGINT as 130"
    );
}
