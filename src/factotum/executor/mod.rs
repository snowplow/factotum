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

pub mod execution_strategy;
pub mod task_list;
#[cfg(test)]
mod tests;

use factotum::executor::task_list::*;
use factotum::executor::execution_strategy::*;
use chrono::UTC;
use factotum::factfile::Task as FactfileTask;
use factotum::factfile::Factfile;
use std::process::Command;
use std::thread;
use std::sync::mpsc;

pub fn get_task_execution_list(factfile: &Factfile,
                               start_from: Option<String>)
                               -> TaskList<&FactfileTask> {
    let mut task_list = TaskList::<&FactfileTask>::new();

    let tasks = if let Some(start_task) = start_from {
        info!("Reduced run! starting from {}", &start_task);
        factfile.get_tasks_in_order_from(&start_task)
    } else {
        factfile.get_tasks_in_order()
    };

    for task_level in tasks.iter() {
        let task_group: TaskGroup<&FactfileTask> = task_level.iter()
            .map(|t| task_list::Task::<&FactfileTask>::new(t.name.clone(), t))
            .collect();
        match task_list.add_group(task_group) {
            Ok(_) => (),
            Err(msg) => panic!(format!("Couldn't add task to group: {}", msg)),
        }
    }

    for task_level in tasks.iter() {
        for task in task_level.iter() {
            for dep in task.depends_on.iter() {
                if task_list.is_task_name_present(&dep) &&
                   task_list.is_task_name_present(&task.name) {
                    match task_list.set_child(&dep, &task.name) {
                        Ok(_) => (),
                        Err(msg) => {
                            panic!(format!("Executor: couldn't add '{}' to child '{}': {}",
                                           dep,
                                           task.name,
                                           msg))
                        }
                    }
                }
            }
        }
    }

    task_list
}


use factotum::executor::task_list::State as TaskExecutionState;

pub type TaskTransitions = Vec<TaskTransition>;

#[derive(Debug, PartialEq, Clone)]
pub struct TaskTransition {
    pub task_name: String,
    pub from_state: TaskExecutionState,
    pub to_state: TaskExecutionState,
}

impl TaskTransition {
    pub fn new(task_name: &str, old_state: TaskExecutionState, state: TaskExecutionState) -> Self {
        TaskTransition {
            task_name: task_name.to_string(),
            from_state: old_state,
            to_state: state,
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct JobTransition {
    pub from: Option<ExecutionState>,
    pub to: ExecutionState,
}


impl JobTransition {
    pub fn new(from: Option<ExecutionState>, to: ExecutionState) -> Self {
        JobTransition {
            from: from,
            to: to,
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum Transition {
    Job(JobTransition),
    Task(TaskTransitions),
}

pub type TaskSnapshot = Vec<Task<FactfileTask>>;

#[derive(Debug, PartialEq, Clone)]
pub enum ExecutionState {
    Started,
    Running,
    Finished,
}

#[derive(Debug, PartialEq, Clone)]
pub struct ExecutionUpdate {
    pub execution_state: ExecutionState,
    pub task_snapshot: TaskSnapshot,
    pub transition: Transition,
}

impl ExecutionUpdate {
    pub fn new(execution_state: ExecutionState,
               task_snapshot: TaskSnapshot,
               transition: Transition)
               -> Self {
        ExecutionUpdate {
            execution_state: execution_state,
            task_snapshot: task_snapshot,
            transition: transition,
        }
    }
}

pub fn get_task_snapshot(tasklist: &TaskList<&FactfileTask>) -> TaskSnapshot {
    tasklist.tasks
        .iter()
        .flat_map(|task_grp| {
            task_grp.iter().map(|task| {
                Task {
                    name: task.name.clone(),
                    task_spec: task.task_spec.clone(),
                    state: task.state.clone(),
                    run_started: task.run_started.clone(),
                    run_result: task.run_result.clone(),
                }
            })
        })
        .collect()
}

pub fn execute_factfile<'a, F>(factfile: &'a Factfile,
                               start_from: Option<String>,
                               strategy: F,
                               progress_channel: Option<mpsc::Sender<ExecutionUpdate>>)
                               -> TaskList<&'a FactfileTask>
    where F: Fn(&str, &mut Command) -> RunResult + Send + Sync + 'static + Copy
{

    let mut tasklist = get_task_execution_list(factfile, start_from);

    // notify the progress channel
    if let Some(ref send) = progress_channel {
        let update =
            ExecutionUpdate::new(ExecutionState::Started,
                                 get_task_snapshot(&tasklist),
                                 Transition::Job(JobTransition::new(None,
                                                                    ExecutionState::Started)));
        send.send(update).unwrap();
    }

    for task_grp_idx in 0..tasklist.tasks.len() {
        // everything in a task "group" gets run together
        let (tx, rx) = mpsc::channel::<(usize, RunResult)>();

        {
            let ref mut task_group = tasklist.tasks[task_grp_idx];
            for (idx, task) in task_group.into_iter().enumerate() {

                if task.state == State::Waiting {
                    info!("Running task '{}'!", task.name);
                    task.state = State::Running;
                    let start_time = UTC::now();
                    task.run_started = Some(start_time);
                    eprintln!("Task '{}' was started at {}", task.name, start_time);
                    {
                        let tx = tx.clone();
                        let args = format_args(&task.task_spec.command, &task.task_spec.arguments);
                        let task_name = task.name.to_string();

                        thread::spawn(move || {
                            let mut command = Command::new("sh");
                            command.arg("-c");
                            command.arg(args);
                            let task_result = strategy(&task_name, &mut command);
                            tx.send((idx, task_result)).unwrap();
                        });
                    }
                } else {
                    info!("Skipped task '{}'", task.name);
                }

            }
        }

        let expected_count = tasklist.tasks[task_grp_idx]
            .iter()
            .filter(|t| t.state == State::Running)
            .count();

        let is_first_run = task_grp_idx == 0;

        if is_first_run {
            if let Some(ref send) = progress_channel {
                let update = ExecutionUpdate::new(ExecutionState::Running, 
                                          get_task_snapshot(&tasklist),
                                          Transition::Job( JobTransition::new(Some(ExecutionState::Started), ExecutionState::Running) ));
                send.send(update).unwrap();
            }
        }

        if expected_count > 0 {

            if let Some(ref send) = progress_channel {
                let running_task_transitions = tasklist.tasks[task_grp_idx]
                    .iter()
                    .filter(|t| t.state == State::Running)
                    .map(|t| {
                        TaskTransition::new(&t.name,
                                            TaskExecutionState::Waiting,
                                            TaskExecutionState::Running)
                    })
                    .collect::<Vec<TaskTransition>>();

                let update = ExecutionUpdate::new(ExecutionState::Running,
                                                  get_task_snapshot(&tasklist),
                                                  Transition::Task(running_task_transitions));

                send.send(update).unwrap();
            }

            for _ in 0..expected_count {
                let (idx, task_result) = rx.recv().unwrap();

                info!("'{}' returned {} in {:?}",
                      tasklist.tasks[task_grp_idx][idx].name,
                      task_result.return_code,
                      task_result.duration);

                let mut additional_transitions = vec![];

                if tasklist.tasks[task_grp_idx][idx]
                    .task_spec
                    .on_result
                    .terminate_job
                    .contains(&task_result.return_code) {
                    // if the return code is in the terminate early list, prune the sub-tree (set to skipped) return early term
                    tasklist.tasks[task_grp_idx][idx].state = State::SuccessNoop;

                    let skip_list =
                        tasklist.get_descendants(&tasklist.tasks[task_grp_idx][idx].name);

                    let cause_task = tasklist.tasks[task_grp_idx][idx].name.clone();

                    for mut task in tasklist.tasks.iter_mut().flat_map(|tg| tg.iter_mut()) {
                        // all the tasks
                        if skip_list.contains(&task.name) {
                            let skip_message = if let State::Skipped(ref msg) = task.state {
                                format!("{}, the task '{}' requested early termination",
                                        msg,
                                        &cause_task)
                            } else {
                                format!("the task '{}' requested early termination", &cause_task)
                            };
                            let prev_state = task.state.clone();
                            task.state = State::Skipped(skip_message);
                            let skip_transition =
                                TaskTransition::new(&task.name, prev_state, task.state.clone());
                            additional_transitions.push(skip_transition);
                        }
                    }
                } else if tasklist.tasks[task_grp_idx][idx]
                    .task_spec
                    .on_result
                    .continue_job
                    .contains(&task_result.return_code) {
                    // if the return code is in the continue list, return success
                    tasklist.tasks[task_grp_idx][idx].state = State::Success;
                    eprintln!("Task '{}': succeeded after {:.1}s",
                             tasklist.tasks[task_grp_idx][idx].name,
                             task_result.duration.as_secs_f64());
                } else {
                    // if the return code is not in either list, prune the sub-tree (set to skipped) and return error
                    let expected_codes = tasklist.tasks[task_grp_idx][idx]
                        .task_spec
                        .on_result
                        .continue_job
                        .iter()
                        .map(|code| code.to_string())
                        .collect::<Vec<String>>()
                        .join(",");
                    let err_msg = format!("the task exited with a value not specified in \
                                           continue_job - {} (task expects one of the following \
                                           return codes to continue [{}])",
                                          task_result.return_code,
                                          expected_codes);
                    eprintln!("Task '{}': failed after {:.1}s. Reason: {}",
                             tasklist.tasks[task_grp_idx][idx].name,
                             task_result.duration.as_secs_f64(),
                             err_msg);
                    tasklist.tasks[task_grp_idx][idx].state = State::Failed(err_msg);
                    let skip_list =
                        tasklist.get_descendants(&tasklist.tasks[task_grp_idx][idx].name);

                    let cause_task = tasklist.tasks[task_grp_idx][idx].name.clone();

                    for mut task in tasklist.tasks.iter_mut().flat_map(|tg| tg.iter_mut()) {
                        // all the tasks
                        if skip_list.contains(&task.name) {
                            let skip_message = if let State::Skipped(ref msg) = task.state {
                                format!("{}, the task '{}' failed", msg, cause_task)
                            } else {
                                format!("the task '{}' failed", cause_task)
                            };
                            let prev_state = task.state.clone();
                            task.state = State::Skipped(skip_message);
                            let skip_transition =
                                TaskTransition::new(&task.name, prev_state, task.state.clone());
                            additional_transitions.push(skip_transition);
                        }
                    }
                }

                tasklist.tasks[task_grp_idx][idx].run_result = Some(task_result);

                if let Some(ref send) = progress_channel {
                    let exec_task_transition =
                        TaskTransition::new(&tasklist.tasks[task_grp_idx][idx].name,
                                            TaskExecutionState::Running,
                                            tasklist.tasks[task_grp_idx][idx].state.clone());
                    additional_transitions.insert(0, exec_task_transition);

                    let update = ExecutionUpdate::new(ExecutionState::Running,
                                                      get_task_snapshot(&tasklist),
                                                      Transition::Task(additional_transitions));
                    send.send(update).unwrap();
                }

            }
        }
    }

    if let Some(ref send) = progress_channel {
        let update = ExecutionUpdate::new(ExecutionState::Finished, 
                                          get_task_snapshot(&tasklist),
                                          Transition::Job( JobTransition::new(Some(ExecutionState::Running), ExecutionState::Finished) ));
        send.send(update).unwrap();
    }

    tasklist
}

pub fn format_args(command: &str, args: &Vec<String>) -> String {
    let arg_str = args.iter()
        .map(|s| format!("\"{}\"", s))
        .collect::<Vec<String>>()
        .join(" ");
    format!("{} {}", command, arg_str)
}
