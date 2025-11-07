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

static JOB_UPDATE_SCHEMA_NAME: &'static str = "iglu:com.snowplowanalytics.\
                                               factotum/job_update/jsonschema/1-0-0";
static TASK_UPDATE_SCHEMA_NAME: &'static str = "iglu:com.snowplowanalytics.\
                                               factotum/task_update/jsonschema/1-0-0";

use factotum::executor::{ExecutionState, ExecutionUpdate, TaskSnapshot,
                         Transition as ExecutorTransition, HeartbeatData};
use super::jobcontext::JobContext;
use chrono::{self, UTC};
use std::collections::BTreeMap;
use rustc_serialize::Encodable;
use rustc_serialize;
use rustc_serialize::json::{self, ToJson, Json};
use factotum::executor::task_list::State;
use std::collections::HashMap;

#[derive(RustcDecodable, RustcEncodable, Debug, PartialEq)]
pub enum JobRunState {
    RUNNING,
    WAITING,
    SUCCEEDED,
    FAILED,
}

#[allow(non_snake_case)]
#[allow(non_camel_case_types)]
#[derive(RustcDecodable, RustcEncodable, Debug, PartialEq)]
pub enum TaskRunState {
    RUNNING,
    WAITING,
    SUCCEEDED,
    SUCCEEDED_NO_OP,
    FAILED,
    SKIPPED,
}

#[derive(RustcDecodable, Debug, PartialEq)]
#[allow(non_snake_case)]
pub struct TaskUpdate {
    taskName: String,
    state: TaskRunState,
    started: Option<String>,
    duration: Option<String>,
    stdout: Option<String>,
    stderr: Option<String>,
    returnCode: Option<i32>,
    errorMessage: Option<String>,
}

impl Encodable for TaskUpdate {
    fn encode<S: rustc_serialize::Encoder>(&self, s: &mut S) -> Result<(), S::Error> {
        self.to_json().encode(s)
    }
}

impl ToJson for TaskUpdate {
    fn to_json(&self) -> Json {

        let mut d = BTreeMap::new();

        // don't emit optional fields

        match self.errorMessage {
            Some(ref value) => {
                d.insert("errorMessage".to_string(), value.to_json());
            }
            None => {}
        }

        match self.returnCode {
            Some(ref value) => {
                d.insert("returnCode".to_string(), value.to_json());
            }
            None => {}
        }

        match self.stderr {
            Some(ref value) => {
                d.insert("stderr".to_string(), value.to_json());
            }
            None => {}
        }

        match self.stdout {
            Some(ref value) => {
                d.insert("stdout".to_string(), value.to_json());
            }
            None => {}
        }

        match self.duration {
            Some(ref value) => {
                d.insert("duration".to_string(), value.to_json());
            }
            None => {}
        }

        match self.started {
            Some(ref value) => {
                d.insert("started".to_string(), value.to_json());
            }
            None => {}
        }

        d.insert("taskName".to_string(), self.taskName.to_json());
        d.insert("state".to_string(),
                 Json::from_str(&json::encode(&self.state).unwrap()).unwrap());


        Json::Object(d)
    }
}

#[derive(RustcEncodable, Debug)]
pub struct SelfDescribingWrapper<'a> {
    pub schema: String,
    pub data: &'a JobUpdate,
}

#[derive(RustcDecodable, RustcEncodable, Debug)]
pub struct ApplicationContext {
    version: String,
    name: String
}

impl ApplicationContext {
    pub fn new(context: &JobContext) -> Self {
        ApplicationContext {
            version: context.factotum_version.clone(),
            name: "factotum".to_string()
        }
    }
}

#[derive(RustcDecodable, RustcEncodable, Debug)]
#[allow(non_snake_case)]
pub struct JobTransition {
    previousState: Option<JobRunState>,
    currentState: JobRunState,
}

impl JobTransition {
    pub fn new(prev_state: &Option<ExecutionState>,
               current_state: &ExecutionState,
               task_snap: &TaskSnapshot)
               -> Self {
        JobTransition {
            previousState: match *prev_state { 
                Some(ref s) => Some(to_job_run_state(s, task_snap)),
                None => None,
            },
            currentState: to_job_run_state(current_state, task_snap),
        }
    }
}

fn to_job_run_state(state: &ExecutionState, tasks: &TaskSnapshot) -> JobRunState {
    match *state {
        ExecutionState::Started => JobRunState::WAITING,
        ExecutionState::Finished => {
            // if any tasks failed, set to failed
            let failed_tasks = tasks.iter()
                .any(|t| match t.state {
                    State::Failed(_) => true,
                    _ => false,
                });
            if failed_tasks {
                JobRunState::FAILED
            } else {
                JobRunState::SUCCEEDED
            }
        }
        _ => JobRunState::RUNNING,
    }
}

#[derive(RustcDecodable, RustcEncodable, Debug)]
#[allow(non_snake_case)]
pub struct TaskTransition {
    previousState: TaskRunState,
    currentState: TaskRunState,
    taskName: String,
}


#[derive(RustcDecodable, Debug)]
#[allow(non_snake_case)]
pub struct JobUpdate {
    jobName: String,
    jobReference: String,
    runReference: String,
    factfile: String,
    applicationContext: ApplicationContext,
    runState: JobRunState,
    startTime: String,
    runDuration: String,
    transition: Option<JobTransition>,
    transitions: Option<Vec<TaskTransition>>,
    taskStates: Vec<TaskUpdate>,
    tags: HashMap<String,String>,
}

impl JobUpdate {
    pub fn new(context: &JobContext, execution_update: &ExecutionUpdate, max_stdouterr_size: &usize) -> Self {
        JobUpdate {
            jobName: context.job_name.clone(),
            jobReference: context.job_reference.clone(),
            runReference: context.run_reference.clone(),
            factfile: context.factfile.clone(),
            applicationContext: ApplicationContext::new(&context),
            tags: context.tags.clone(),
            runState: to_job_run_state(&execution_update.execution_state,
                                       &execution_update.task_snapshot),
            startTime: to_string_datetime(&context.start_time),
            runDuration: (UTC::now() - context.start_time).to_string(),
            taskStates: {
                // Get heartbeat data from either Heartbeat transitions OR live_task_logs field
                let heartbeat_data = match execution_update.transition {
                    ExecutorTransition::Heartbeat(ref hb) => Some(hb),
                    _ => execution_update.live_task_logs.as_ref(),
                };
                JobUpdate::to_task_states(&execution_update.task_snapshot, &max_stdouterr_size, heartbeat_data)
            },
            transition: {
                match execution_update.transition {
                    ExecutorTransition::Job(ref j) => {
                        Some(JobTransition::new(&j.from, &j.to, &execution_update.task_snapshot))
                    }
                    _ => None,
                }
            },
            transitions: {
                match execution_update.transition {
                    ExecutorTransition::Task(ref tu) => {
                        let tasks = tu.iter()
                            .map(|t| {
                                TaskTransition {
                                    taskName: t.task_name.clone(),
                                    previousState: match t.from_state {
                                        State::Waiting => TaskRunState::WAITING,
                                        State::Running => TaskRunState::RUNNING,
                                        State::Skipped(_) => TaskRunState::SKIPPED,
                                        State::Success => TaskRunState::SUCCEEDED,
                                        State::SuccessNoop => TaskRunState::SUCCEEDED_NO_OP,
                                        State::Failed(_) => TaskRunState::FAILED,
                                    },
                                    currentState: match t.to_state {
                                        State::Waiting => TaskRunState::WAITING,
                                        State::Running => TaskRunState::RUNNING,
                                        State::Skipped(_) => TaskRunState::SKIPPED,
                                        State::Success => TaskRunState::SUCCEEDED,
                                        State::SuccessNoop => TaskRunState::SUCCEEDED_NO_OP,
                                        State::Failed(_) => TaskRunState::FAILED,
                                    },
                                }
                            })
                            .collect();
                        Some(tasks)
                    }
                    ExecutorTransition::Heartbeat(ref hb) => {
                        // For heartbeats, create synthetic RUNNING->RUNNING transitions
                        let tasks = hb.iter()
                            .map(|t| {
                                TaskTransition {
                                    taskName: t.task_name.clone(),
                                    previousState: TaskRunState::RUNNING,
                                    currentState: TaskRunState::RUNNING,
                                }
                            })
                            .collect();
                        Some(tasks)
                    }
                    _ => None,
                }
            },
        }
    }

    pub fn as_self_desc_json(&self) -> String {
        let wrapped = SelfDescribingWrapper {
            schema: match self.transition {
                Some(_) => JOB_UPDATE_SCHEMA_NAME.into(),
                None => TASK_UPDATE_SCHEMA_NAME.into(),
            },
            data: &self,
        };
        json::encode(&wrapped).unwrap()
    }

    fn to_task_states(tasks: &TaskSnapshot, max_stdouterr_size: &usize, heartbeat_logs: Option<&HeartbeatData>) -> Vec<TaskUpdate> {
        use chrono::duration::Duration as ChronoDuration;
        use std::collections::HashMap;

        // Build a lookup map for heartbeat logs
        let heartbeat_map: HashMap<&str, (&str, &str)> = if let Some(hb_data) = heartbeat_logs {
            hb_data.iter()
                .map(|hb| (hb.task_name.as_str(), (hb.stdout.as_str(), hb.stderr.as_str())))
                .collect()
        } else {
            HashMap::new()
        };

        tasks.iter()
            .map(|task| {
                // Check if this task has heartbeat logs
                let hb_logs = heartbeat_map.get(task.name.as_str());

                TaskUpdate {
                    taskName: task.name.clone(),
                    state: match task.state {
                        State::Waiting => TaskRunState::WAITING,
                        State::Running => TaskRunState::RUNNING,
                        State::Skipped(_) => TaskRunState::SKIPPED,
                        State::Success => TaskRunState::SUCCEEDED,
                        State::SuccessNoop => TaskRunState::SUCCEEDED_NO_OP,
                        State::Failed(_) => TaskRunState::FAILED,
                    },
                    started: if let Some(ref r) = task.run_started {
                        Some(to_string_datetime(r))
                    } else {
                        None
                    },
                    duration: if let Some(ref r) = task.run_result {
                        Some(ChronoDuration::from_std(r.duration).unwrap().to_string())
                    } else {
                        None
                    },
                    stdout: if let Some(ref r) = task.run_result {
                        // Completed task - use run_result
                        if let Some(ref stdout) = r.stdout {
                            Some(tail_n_chars(stdout, *max_stdouterr_size).into())
                        } else {
                            None
                        }
                    } else if let Some(&(stdout, _)) = hb_logs {
                        // Running task with heartbeat logs
                        if stdout.is_empty() {
                            None
                        } else {
                            Some(tail_n_chars(stdout, *max_stdouterr_size).into())
                        }
                    } else {
                        None
                    },
                    stderr: if let Some(ref r) = task.run_result {
                        // Completed task - use run_result
                        if let Some(ref stderr) = r.stderr {
                            Some(tail_n_chars(stderr, *max_stdouterr_size).into())
                        } else {
                            None
                        }
                    } else if let Some(&(_, stderr)) = hb_logs {
                        // Running task with heartbeat logs
                        if stderr.is_empty() {
                            None
                        } else {
                            Some(tail_n_chars(stderr, *max_stdouterr_size).into())
                        }
                    } else {
                        None
                    },
                    returnCode: if let Some(ref r) = task.run_result {
                        Some(r.return_code)
                    } else {
                        None
                    },
                    errorMessage: match (&task.state, &task.run_result) {
                        (&State::Skipped(ref reason), _) => Some(reason.clone()),
                        (&State::Failed(ref reason), &Some(ref result)) => {
                            if let Some(ref execution_error) = result.task_execution_error {
                                Some(execution_error.clone())
                            } else {
                                Some(reason.clone())
                            }
                        },
                        _ => None
                    },
                }
            })
            .collect()
    }
}

impl Encodable for JobUpdate {
    fn encode<S: rustc_serialize::Encoder>(&self, s: &mut S) -> Result<(), S::Error> {
        self.to_json().encode(s)
    }
}

impl ToJson for JobUpdate {
    fn to_json(&self) -> Json {

        let mut d = BTreeMap::new();

        d.insert("jobName".into(), self.jobName.to_json());
        d.insert("jobReference".into(), self.jobReference.to_json());
        d.insert("runReference".into(), self.runReference.to_json());
        d.insert("factfile".into(), self.factfile.to_json());

        d.insert("applicationContext".into(),
                 Json::from_str(&json::encode(&self.applicationContext).unwrap()).unwrap());

        d.insert("runState".into(),
                 Json::from_str(&json::encode(&self.runState).unwrap()).unwrap());

        d.insert("startTime".into(), self.startTime.to_json());
        d.insert("runDuration".into(), self.runDuration.to_json());

        d.insert("tags".into(), self.tags.to_json());

        match self.transition {
            Some(ref job_transition) => {
                d.insert("jobTransition".into(),
                         Json::from_str(&json::encode(&job_transition).unwrap()).unwrap());
            }
            None => {}
        }

        match self.transitions {
            Some(ref task_transition) => {
                d.insert("taskTransitions".into(),
                         Json::from_str(&json::encode(&task_transition).unwrap()).unwrap());
            } 
            None => {}
        }

        d.insert("taskStates".into(),
                 Json::from_str(&json::encode(&self.taskStates).unwrap()).unwrap());

        Json::Object(d)
    }
}

pub fn to_string_datetime(datetime: &chrono::DateTime<UTC>) -> String {
    datetime.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

pub fn tail_n_chars(s: &str, n: usize) -> &str {
    if n < 1 {
       ""
    } else {
        if let Some(v) = s.char_indices().rev().nth(n-1).map(|(i, _)| &s[i..]) {
            v
        } else {
            s
        }
    }
}