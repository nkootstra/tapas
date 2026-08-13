use std::io;
use std::os::fd::RawFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

const FORWARDED_SIGNALS: [(libc::c_int, u32); 4] = [
    (libc::SIGINT, 1 << 0),
    (libc::SIGTERM, 1 << 1),
    (libc::SIGHUP, 1 << 2),
    (libc::SIGQUIT, 1 << 3),
];

static SIGNAL_FORWARDING_LOCK: Mutex<()> = Mutex::new(());
static CHILD_PROCESS_GROUP: AtomicI32 = AtomicI32::new(0);
static PENDING_SIGNALS: AtomicU32 = AtomicU32::new(0);

extern "C" fn record_signal(signal: libc::c_int) {
    for &(candidate, bit) in &FORWARDED_SIGNALS {
        if signal == candidate {
            PENDING_SIGNALS.fetch_or(bit, Ordering::Relaxed);
            break;
        }
    }
}

pub struct SignalForwarder {
    previous_actions: [libc::sigaction; FORWARDED_SIGNALS.len()],
    _exclusive: MutexGuard<'static, ()>,
}

impl SignalForwarder {
    fn install() -> io::Result<Self> {
        let action = forwarding_action()?;
        let exclusive = SIGNAL_FORWARDING_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous_mask = block_forwarded_signals()?;
        CHILD_PROCESS_GROUP.store(0, Ordering::Release);
        PENDING_SIGNALS.store(0, Ordering::Release);

        // SAFETY: sigaction is a plain C data structure which is fully initialized by
        // sigaction before an entry is observed.
        let mut previous_actions =
            std::array::from_fn(|_| unsafe { std::mem::zeroed::<libc::sigaction>() });
        let mut installed = 0;
        for (index, &(signal, _)) in FORWARDED_SIGNALS.iter().enumerate() {
            // SAFETY: action and the output slot are valid for this call, and the
            // handler has the C signal-handler ABI.
            if unsafe { libc::sigaction(signal, &action, &mut previous_actions[index]) } == -1 {
                let error = io::Error::last_os_error();
                restore_actions(&previous_actions, installed);
                let _ = restore_signal_mask(&previous_mask);
                return Err(error);
            }
            installed += 1;
        }
        if let Err(error) = restore_signal_mask(&previous_mask) {
            restore_actions(&previous_actions, installed);
            return Err(error);
        }

        Ok(Self {
            previous_actions,
            _exclusive: exclusive,
        })
    }

    pub fn forward_pending(&self) -> io::Result<()> {
        let pending = PENDING_SIGNALS.swap(0, Ordering::AcqRel);
        let process_group = CHILD_PROCESS_GROUP.load(Ordering::Acquire);
        if process_group <= 0 {
            PENDING_SIGNALS.fetch_or(pending, Ordering::Release);
            return Ok(());
        }

        let mut first_error = None;
        for &(signal, bit) in &FORWARDED_SIGNALS {
            if pending & bit == 0 {
                continue;
            }
            // SAFETY: a negative, nonzero PID asks kill to target the child process
            // group. kill is called from ordinary Rust control flow, not the handler.
            if unsafe { libc::kill(-process_group, signal) } == -1 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) && first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

impl Drop for SignalForwarder {
    fn drop(&mut self) {
        let Ok(previous_mask) = block_forwarded_signals() else {
            return;
        };
        let _ = self.forward_pending();
        CHILD_PROCESS_GROUP.store(0, Ordering::Release);
        PENDING_SIGNALS.store(0, Ordering::Release);
        restore_actions(&self.previous_actions, self.previous_actions.len());
        let _ = restore_signal_mask(&previous_mask);
    }
}

pub fn spawn_process_group(command: &mut Command) -> io::Result<(Child, SignalForwarder)> {
    let forwarder = SignalForwarder::install()?;
    command.process_group(0);
    let mut child = spawn_with_text_busy_retry(command)?;
    let process_group = match libc::pid_t::try_from(child.id()) {
        Ok(process_group) => process_group,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("child PID does not fit pid_t"));
        }
    };
    CHILD_PROCESS_GROUP.store(process_group, Ordering::Release);
    if let Err(error) = forwarder.forward_pending() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    Ok((child, forwarder))
}

fn spawn_with_text_busy_retry(command: &mut Command) -> io::Result<Child> {
    const RETRIES: usize = 5;
    const RETRY_DELAY: Duration = Duration::from_millis(10);

    for attempt in 0..=RETRIES {
        match command.spawn() {
            Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) && attempt < RETRIES => {
                std::thread::sleep(RETRY_DELAY);
            }
            result => return result,
        }
    }
    unreachable!("the bounded spawn loop always returns")
}

pub fn wait_for_child(child: &mut Child, forwarder: &SignalForwarder) -> io::Result<ExitStatus> {
    loop {
        forwarder.forward_pending()?;
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        poll_readable(None, None)?;
    }
}

fn forwarding_action() -> io::Result<libc::sigaction> {
    // SAFETY: all-zero is a valid starting representation for sigaction, whose
    // handler, flags, and mask are assigned below before use.
    let mut action = unsafe { std::mem::zeroed::<libc::sigaction>() };
    action.sa_sigaction = record_signal as *const () as libc::sighandler_t;
    action.sa_flags = 0;
    // SAFETY: sa_mask is a valid sigset_t owned by action.
    if unsafe { libc::sigemptyset(&mut action.sa_mask) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(action)
}

fn block_forwarded_signals() -> io::Result<libc::sigset_t> {
    // SAFETY: sigset_t is initialized by sigemptyset before it is read.
    let mut signals = unsafe { std::mem::zeroed::<libc::sigset_t>() };
    // SAFETY: signals points to a valid sigset_t.
    if unsafe { libc::sigemptyset(&mut signals) } == -1 {
        return Err(io::Error::last_os_error());
    }
    for &(signal, _) in &FORWARDED_SIGNALS {
        // SAFETY: signals remains valid and signal is one of the platform constants.
        if unsafe { libc::sigaddset(&mut signals, signal) } == -1 {
            return Err(io::Error::last_os_error());
        }
    }

    // SAFETY: both signal-set pointers are valid for the duration of the call.
    let mut previous = unsafe { std::mem::zeroed::<libc::sigset_t>() };
    // SAFETY: signals and previous point to initialized storage.
    let result = unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &signals, &mut previous) };
    if result == 0 {
        Ok(previous)
    } else {
        Err(io::Error::from_raw_os_error(result))
    }
}

fn restore_signal_mask(previous: &libc::sigset_t) -> io::Result<()> {
    // SAFETY: previous points to a signal mask returned by pthread_sigmask.
    let result =
        unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, previous, std::ptr::null_mut()) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(result))
    }
}

fn restore_actions(
    previous_actions: &[libc::sigaction; FORWARDED_SIGNALS.len()],
    installed: usize,
) {
    for (index, &(signal, _)) in FORWARDED_SIGNALS.iter().take(installed).enumerate() {
        // SAFETY: each saved action was initialized by a successful sigaction call.
        let _ = unsafe { libc::sigaction(signal, &previous_actions[index], std::ptr::null_mut()) };
    }
}

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
        .or_else(|| {
            status
                .signal()
                .or_else(|| status.stopped_signal())
                .map(|signal| 128 + signal)
        })
        .unwrap_or(1)
}
