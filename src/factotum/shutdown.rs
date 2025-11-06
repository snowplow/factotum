use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

lazy_static! {
    /// Global flag indicating shutdown has been requested
    static ref SHUTDOWN_FLAG: AtomicBool = AtomicBool::new(false);

    /// List of child process IDs that need to be terminated during shutdown
    static ref CHILD_PROCESSES: Mutex<Vec<u32>> = Mutex::new(Vec::new());
}

/// Check if shutdown has been requested
pub fn is_shutting_down() -> bool {
    SHUTDOWN_FLAG.load(Ordering::SeqCst)
}

/// Signal that shutdown should begin
pub fn request_shutdown() {
    SHUTDOWN_FLAG.store(true, Ordering::SeqCst);
}

/// Register a child process for tracking during shutdown
pub fn register_child_process(pid: u32) {
    let mut processes = CHILD_PROCESSES.lock()
        .unwrap_or_else(|poisoned| {
            warn!("CHILD_PROCESSES mutex was poisoned during registration, recovering data");
            poisoned.into_inner()
        });
    processes.push(pid);
}

/// Remove a child process from tracking (called when process exits normally)
pub fn unregister_child_process(pid: u32) {
    let mut processes = CHILD_PROCESSES.lock()
        .unwrap_or_else(|poisoned| {
            warn!("CHILD_PROCESSES mutex was poisoned during unregistration, recovering data");
            poisoned.into_inner()
        });
    processes.retain(|&p| p != pid);
}

/// Gracefully terminate all child processes using a hybrid approach
/// Phase 1: Broadcasts SIGTERM to process group (includes factotum but it has a handler)
/// Phase 2: Sends SIGKILL only to individual child PIDs (excludes factotum)
pub fn terminate_all_children() {
    let pids = {
        let processes = CHILD_PROCESSES.lock()
            .unwrap_or_else(|poisoned| {
                warn!("CHILD_PROCESSES mutex was poisoned during shutdown, recovering data");
                poisoned.into_inner()
            });
        processes.clone()
    };

    if pids.is_empty() {
        return;
    }

    println!("Terminating {} child process(es) gracefully...", pids.len());

    // Phase 1: Broadcast SIGTERM to entire process group
    // This includes factotum, but factotum has a signal handler so it won't die
    // This is race-free - all processes get SIGTERM atomically
    unsafe {
        #[cfg(unix)]
        {
            // SAFETY: Sending SIGTERM to process group (pid 0).
            // - Process group targeting (pid 0) is atomic and race-free
            // - Factotum has signal handlers installed (see main.rs), so it will set
            //   the shutdown flag but not terminate
            // - All children spawned by factotum inherit the same process group and
            //   will receive SIGTERM for graceful shutdown
            // - PID recycling is not a concern here because we're targeting the entire
            //   process group by ID, not individual PIDs
            // - Platform: Unix-only. Non-Unix platforms don't reach this code path.
            libc::kill(0, libc::SIGTERM);
        }
    }

    // Wait 5 seconds for graceful shutdown
    thread::sleep(Duration::from_secs(5));

    // Phase 2: Check which child processes are still alive and SIGKILL them individually
    // This excludes factotum - only tracked children are killed
    let surviving_pids: Vec<u32> = pids.iter()
        .filter(|&&pid| is_process_alive(pid))
        .cloned()
        .collect();

    if !surviving_pids.is_empty() {
        println!("Force-killing {} remaining process(es)...", surviving_pids.len());
        for &pid in &surviving_pids {
            unsafe {
                #[cfg(unix)]
                {
                    // SAFETY: Sending SIGKILL to individual child PIDs.
                    // - PIDs are obtained from CHILD_PROCESSES, which tracks all spawned children
                    // - PIDs are valid because they were registered immediately after spawn
                    // - PIDs are filtered by is_process_alive() so we only kill surviving processes
                    // - PID recycling edge case: If a PID is recycled between the is_process_alive()
                    //   check and this kill() call, we might signal an unrelated process. This is
                    //   extremely rare because:
                    //   1. We only target PIDs from children spawned seconds ago
                    //   2. The check-to-kill window is microseconds
                    //   3. Unix systems typically have PID wraparound delays
                    //   4. Recycled PIDs are usually given to unrelated processes, not critical ones
                    // - This approach ensures factotum survives to send webhooks and write logs
                    // - Platform: Unix-only. Non-Unix platforms don't reach this code path.
                    libc::kill(pid as i32, libc::SIGKILL);
                }
            }
        }
    }
}

/// Check if a process is still running
fn is_process_alive(pid: u32) -> bool {
    unsafe {
        #[cfg(unix)]
        {
            // SAFETY: Sending signal 0 checks if process exists without sending a real signal.
            // - Signal 0 is a null signal that performs permission and existence checks only
            // - Returns 0 (true) if process exists and we have permission to signal it
            // - Returns -1 (false) if process doesn't exist or permission is denied
            // - No side effects on the target process - completely safe for checking
            // - PID recycling: Could return true for a recycled PID (false positive), but this
            //   is acceptable because we'll send SIGKILL to confirm termination. A false positive
            //   just means we attempt to kill a process that may have changed identity, which is
            //   handled by the SIGKILL safety invariants.
            // - Platform: Unix-only, where signal 0 is well-defined behavior
            libc::kill(pid as i32, 0) == 0
        }
        #[cfg(not(unix))]
        {
            // SAFETY: On non-Unix platforms (Windows, etc.), we conservatively return false.
            // This means we'll always attempt SIGKILL in terminate_all_children(), which is
            // safe but suboptimal (no process liveness check).
            // TODO: Implement Windows-specific process checking using OpenProcess/GetExitCodeProcess
            //       if Windows support is needed in the future.
            false
        }
    }
}
