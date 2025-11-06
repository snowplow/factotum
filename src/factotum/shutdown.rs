use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

lazy_static! {
    /// Global flag indicating shutdown has been requested
    static ref SHUTDOWN_FLAG: AtomicBool = AtomicBool::new(false);

    /// List of child process IDs that need to be terminated during shutdown
    static ref CHILD_PROCESSES: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
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
    if let Ok(mut processes) = CHILD_PROCESSES.lock() {
        processes.push(pid);
    }
}

/// Remove a child process from tracking (called when process exits normally)
pub fn unregister_child_process(pid: u32) {
    if let Ok(mut processes) = CHILD_PROCESSES.lock() {
        processes.retain(|&p| p != pid);
    }
}

/// Gracefully terminate all tracked child processes
/// Sends SIGTERM first, waits 5 seconds, then sends SIGKILL if needed
pub fn terminate_all_children() {
    let pids = {
        if let Ok(processes) = CHILD_PROCESSES.lock() {
            processes.clone()
        } else {
            return;
        }
    };

    if pids.is_empty() {
        return;
    }

    println!("Terminating {} child process(es) gracefully...", pids.len());

    // Send SIGTERM to all children
    for &pid in &pids {
        unsafe {
            #[cfg(unix)]
            libc::kill(pid as i32, libc::SIGTERM);
        }
    }

    // Wait 5 seconds for graceful shutdown
    thread::sleep(Duration::from_secs(5));

    // Check which processes are still alive and send SIGKILL
    let surviving_pids: Vec<u32> = pids.iter()
        .filter(|&&pid| is_process_alive(pid))
        .cloned()
        .collect();

    if !surviving_pids.is_empty() {
        println!("Force-killing {} remaining process(es)...", surviving_pids.len());
        for &pid in &surviving_pids {
            unsafe {
                #[cfg(unix)]
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
    }
}

/// Check if a process is still running
fn is_process_alive(pid: u32) -> bool {
    unsafe {
        #[cfg(unix)]
        {
            // Sending signal 0 checks if process exists without actually sending a signal
            libc::kill(pid as i32, 0) == 0
        }
        #[cfg(not(unix))]
        {
            false
        }
    }
}
