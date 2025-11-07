use std::sync::Mutex;
use std::collections::HashMap;
use chrono::{DateTime, UTC};
use std::sync::Arc;
use std::time::Duration;
use std::sync::mpsc::Sender;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use super::{ExecutionUpdate, ExecutionState, Transition, HeartbeatTaskData, get_task_snapshot};

// Shared state for streaming logs
pub struct TaskStreamState {
    pub start_time: DateTime<UTC>,
    pub stdout_buffer: Arc<Mutex<String>>,
    pub stderr_buffer: Arc<Mutex<String>>,
}

// Global state - tracks running tasks with log buffers
lazy_static! {
    static ref RUNNING_TASKS: Mutex<HashMap<String, TaskStreamState>> =
        Mutex::new(HashMap::new());
}

// Called by executor when task starts
// Returns Arc clones to log buffers for the streaming threads
pub fn register_task_start(name: String, start: DateTime<UTC>)
    -> (Arc<Mutex<String>>, Arc<Mutex<String>>)
{
    let stdout_buffer = Arc::new(Mutex::new(String::new()));
    let stderr_buffer = Arc::new(Mutex::new(String::new()));

    RUNNING_TASKS.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(name, TaskStreamState {
            start_time: start,
            stdout_buffer: stdout_buffer.clone(),
            stderr_buffer: stderr_buffer.clone(),
        });

    (stdout_buffer, stderr_buffer)
}

// Called by executor when task completes
pub fn unregister_task_completion(name: &str) {
    RUNNING_TASKS.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(name);
}

// Snapshot with cumulative logs (for heartbeats)
pub struct TaskHeartbeatData {
    pub name: String,
    pub start_time: DateTime<UTC>,
    pub elapsed: Duration,
    pub stdout: String,  // Cumulative stdout up to now
    pub stderr: String,  // Cumulative stderr up to now
}

// Get snapshot for heartbeat with cumulative logs
pub fn get_running_tasks_with_logs() -> Vec<TaskHeartbeatData> {
    let now = UTC::now();
    let tasks = RUNNING_TASKS.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    tasks.iter()
        .map(|(name, state)| {
            let stdout = state.stdout_buffer.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            let stderr = state.stderr_buffer.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();

            let elapsed_chrono = now - state.start_time;
            let elapsed_std = elapsed_chrono.to_std()
                .unwrap_or(Duration::from_secs(0));

            TaskHeartbeatData {
                name: name.clone(),
                start_time: state.start_time,
                elapsed: elapsed_std,
                stdout,
                stderr,
            }
        })
        .collect()
}

// Spawn heartbeat timer thread
pub fn spawn_heartbeat_thread(
    interval: Duration,
    progress_channel: Sender<ExecutionUpdate>,
    shutdown_flag: Arc<AtomicBool>,
    shared_snapshot: Arc<Mutex<super::TaskSnapshot>>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("heartbeat".to_string())
        .spawn(move || {
            loop {
                thread::sleep(interval);

                // Check shutdown first
                if shutdown_flag.load(Ordering::SeqCst) {
                    break;
                }

                let running = get_running_tasks_with_logs();
                if running.is_empty() {
                    continue;
                }

                // Convert to HeartbeatTaskData for the transition
                let heartbeat_data: Vec<HeartbeatTaskData> = running.into_iter()
                    .map(|t| HeartbeatTaskData {
                        task_name: t.name,
                        elapsed_seconds: t.elapsed.as_secs(),
                        stdout: t.stdout,
                        stderr: t.stderr,
                    })
                    .collect();

                // Get live snapshot from shared state
                let snapshot = shared_snapshot.lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone();

                // Create heartbeat ExecutionUpdate
                let update = ExecutionUpdate::new(
                    ExecutionState::Running,
                    snapshot,
                    Transition::Heartbeat(heartbeat_data)
                );

                if let Err(e) = progress_channel.send(update) {
                    warn!("Heartbeat send failed: {}, exiting", e);
                    break;
                }
            }
        })
        .expect("Failed to spawn heartbeat thread")
}
