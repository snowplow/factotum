// Copyright (c) 2016-2021 Snowplow Analytics Ltd. All rights reserved.
//
// This program is licensed to you under the Apache License Version 2.0, and
// you may not use this file except in compliance with the Apache License
// Version 2.0.  You may obtain a copy of the Apache License Version 2.0 at
// http://www.apache.org/licenses/LICENSE-2.0.
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the Apache License Version 2.0 is distributed on an "AS
// IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
// implied.  See the Apache License Version 2.0 for the specific language
// governing permissions and limitations there under.
//

#[cfg(test)]
mod tests;
use std::process::{Command, Stdio};
use std::time::{Instant, Duration};
use std::io::{BufRead, BufReader};
use std::thread;
use colored::*;

#[derive(Clone, PartialEq, Debug)]
pub struct RunResult {
    pub duration: Duration,
    pub task_execution_error: Option<String>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub return_code: i32,
}

pub fn simulation_text(name: &str, command: &Command) -> String {

    use std::cmp;
    let command_text = format!("{:?}", command);

    let col_task_title = "TASK";
    let col_command_title = "COMMAND";
    let col_padding = 2;
    let task_col_width = cmp::max(name.len() + col_padding, col_task_title.len() + col_padding);
    let command_col_width = cmp::max(command_text.len() + col_padding,
                                     col_command_title.len() + col_padding);

    let lines = vec![ 
                      format!("/{fill:->taskwidth$}|{fill:->cmdwidth$}\\", fill="-", taskwidth=task_col_width, cmdwidth=command_col_width),
                      format!("| {:taskwidth$} | {:cmdwidth$} |", "TASK", "COMMAND", taskwidth=task_col_width-col_padding, cmdwidth=command_col_width-col_padding),
                      format!("|{fill:-<taskwidth$}|{fill:-<cmdwidth$}|", fill="-", taskwidth=task_col_width, cmdwidth=command_col_width),
                      format!("| {:taskwidth$} | {:-<cmdwidth$} |", name, command_text, taskwidth=task_col_width-col_padding, cmdwidth=command_col_width-col_padding),
                      format!("\\{fill:-<taskwidth$}|{fill:-<cmdwidth$}/\n", fill="-", taskwidth=task_col_width, cmdwidth=command_col_width),
                    ];

    lines.join("\n")
}

pub fn execute_simulation(name: &str, command: &mut Command) -> RunResult {
    info!("Simulating execution for {} with command {:?}",
          name,
          command);
    RunResult {
        duration: Duration::from_secs(0),
        task_execution_error: None,
        stdout: Some(simulation_text(name, &command)),
        stderr: None,
        return_code: 0,
    }
}

pub fn execute_os(name: &str, command: &mut Command) -> RunResult {
    let run_start = Instant::now();
    info!("Executing sh {:?}", command);

    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    match command.spawn() {
        Ok(mut child) => {
            // Register child process for tracking during shutdown
            let child_pid = child.id();
            crate::factotum::shutdown::register_child_process(child_pid);

            let stdout = child.stdout.take().expect("Failed to capture stdout");
            let stderr = child.stderr.take().expect("Failed to capture stderr");

            let name_stdout = name.to_string();
            let name_stderr = name.to_string();

            let stdout_handle = thread::spawn(move || {
                let reader = BufReader::new(stdout);
                let mut lines = Vec::new();
                for line in reader.lines() {
                    if let Ok(line) = line {
                        println!("[{}] {}", name_stdout.cyan(), line);
                        lines.push(line);
                    }
                }
                lines.join("\n")
            });

            let stderr_handle = thread::spawn(move || {
                let reader = BufReader::new(stderr);
                let mut lines = Vec::new();
                for line in reader.lines() {
                    if let Ok(line) = line {
                        eprintln!("[{}] {}", name_stderr.cyan(), line.red());
                        lines.push(line);
                    }
                }
                lines.join("\n")
            });

            // Check if shutdown was requested before waiting
            if crate::factotum::shutdown::is_shutting_down() {
                // Kill the child process
                let _ = child.kill();

                // Wait for it to exit (reap zombie)
                let _ = child.wait();

                // Unregister since we're handling shutdown
                crate::factotum::shutdown::unregister_child_process(child_pid);

                // Join the streaming threads to capture any final output
                let task_stdout = stdout_handle.join().unwrap_or_else(|_| {
                    warn!("stdout reader thread panicked for task '{}'", name);
                    String::new()
                });
                let task_stderr = stderr_handle.join().unwrap_or_else(|_| {
                    warn!("stderr reader thread panicked for task '{}'", name);
                    String::new()
                });

                return RunResult {
                    duration: run_start.elapsed(),
                    task_execution_error: Some("Task cancelled due to shutdown".to_string()),
                    stdout: if task_stdout.is_empty() { None } else { Some(task_stdout) },
                    stderr: if task_stderr.is_empty() { None } else { Some(task_stderr) },
                    return_code: -1,
                };
            }

            match child.wait() {
                Ok(status) => {
                    // Unregister child process after it completes
                    crate::factotum::shutdown::unregister_child_process(child_pid);

                    let run_duration = run_start.elapsed();
                    let return_code = status.code().unwrap_or(1);

                    let task_stdout = stdout_handle.join().unwrap_or_else(|_| {
                        warn!("stdout reader thread panicked for task '{}'", name);
                        String::new()
                    });
                    let task_stderr = stderr_handle.join().unwrap_or_else(|_| {
                        warn!("stderr reader thread panicked for task '{}'", name);
                        String::new()
                    });

                    info!("task '{}' completed with return code {}", name, return_code);

                    let task_stdout_opt = if task_stdout.is_empty() {
                        None
                    } else {
                        Some(task_stdout)
                    };
                    let task_stderr_opt = if task_stderr.is_empty() {
                        None
                    } else {
                        Some(task_stderr)
                    };

                    RunResult {
                        duration: run_duration,
                        task_execution_error: None,
                        stdout: task_stdout_opt,
                        stderr: task_stderr_opt,
                        return_code: return_code,
                    }
                }
                Err(e) => {
                    RunResult {
                        duration: run_start.elapsed(),
                        task_execution_error: Some(format!("Error waiting for process - {}", e)),
                        stdout: None,
                        stderr: None,
                        return_code: -1,
                    }
                }
            }
        }
        Err(message) => {
            RunResult {
                duration: Duration::from_secs(0),
                task_execution_error: Some(format!("Error executing process - {}", message)),
                stdout: None,
                stderr: None,
                return_code: -1,
            }
        }
    }
}
