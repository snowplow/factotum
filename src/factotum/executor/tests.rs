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

use factotum::tests::make_task;
use factotum::factfile::*;
use factotum::executor::*;

#[test]
fn get_task_execution_list_good() {
    let mut ff = Factfile::new("N/A", "test");
    ff.add_task_obj(&make_task("apple", &vec![]));
    ff.add_task_obj(&make_task("turnip", &vec![]));
    ff.add_task_obj(&make_task("orange", &vec!["apple"]));
    ff.add_task_obj(&make_task("egg", &vec!["apple"]));
    ff.add_task_obj(&make_task("potato", &vec!["apple", "egg"]));
    ff.add_task_obj(&make_task("chicken", &vec!["potato", "orange"]));

    //        apple----------             turnip
    //       /     \         \
    //   orange    egg-----   \
    //      \              \   \
    //       \               potato
    //        \                 \
    //         --------------- chicken

    let tl = get_task_execution_list(&ff, None);

    let expected =
        vec![vec!["turnip", "apple"], vec!["egg", "orange"], vec!["potato"], vec!["chicken"]];

    let actual: Vec<Vec<String>> = tl.tasks
        .iter()
        .map(|task_group| task_group.iter().map(|task| task.name.clone()).collect())
        .collect();

    assert_eq!(actual, expected);

    // check the children are correctly mapped

    assert_eq!(tl.get_descendants("turnip"), Vec::<String>::new());
    assert_eq!(tl.get_descendants("apple"),
               vec!["chicken", "egg", "orange", "potato"]);
    assert_eq!(tl.get_descendants("egg"), vec!["chicken", "potato"]);
    assert_eq!(tl.get_descendants("orange"), vec!["chicken"]);
}

#[test]
fn get_task_execution_list_good_reduced() {
    let mut ff = Factfile::new("N/A", "test");
    ff.add_task_obj(&make_task("apple", &vec![]));
    ff.add_task_obj(&make_task("turnip", &vec![]));
    ff.add_task_obj(&make_task("orange", &vec!["apple"]));
    ff.add_task_obj(&make_task("egg", &vec!["apple"]));
    ff.add_task_obj(&make_task("potato", &vec!["apple", "egg"]));
    ff.add_task_obj(&make_task("chicken", &vec!["potato", "orange"]));

    let tl = get_task_execution_list(&ff, Some("potato".to_string()));
    assert!(tl.tasks[0].len() == 1);
    assert_eq!(tl.get_descendants("potato"), vec!["chicken"]);
}

#[test]
fn get_formatted_args() {
    let args_list =
        format_args("echo",
                    &vec!["hello".to_string(), "world".to_string(), "abc abc".to_string()]);
    assert_eq!(args_list, "echo \"hello\" \"world\" \"abc abc\"");
}

#[test]
fn get_task_snapshot_clones() {

    use factotum::executor::task_list::*;
    use factotum::executor::execution_strategy::*;
    use chrono::UTC;
    use chrono::duration::Duration;

    let mut ff = Factfile::new("N/A", "test");
    ff.add_task_obj(&make_task("apple", &vec![]));
    ff.add_task_obj(&make_task("turnip", &vec!["apple"]));

    let mut tl = get_task_execution_list(&ff, None);

    println!("{:?}", tl);

    tl.tasks[0][0].state = State::Success;
    tl.tasks[0][0].run_started = Some(UTC::now());
    tl.tasks[0][0].run_result = Some(RunResult {
        return_code: 0,
        stderr: Some("hello world".to_string()),
        stdout: Some("hello world".to_string()),
        duration: Duration::seconds(0).to_std().ok().unwrap(),
        task_execution_error: None,
    });

    let snapshot = get_task_snapshot(&tl);

    assert_eq!(snapshot.len(), 2);

    assert_eq!(snapshot[0].state, State::Success);
    assert_eq!(snapshot[0].name, "apple");
    assert_eq!(snapshot[0].run_result, tl.tasks[0][0].run_result);
    assert_eq!(&snapshot[0].task_spec, tl.tasks[0][0].task_spec);
    assert!(&snapshot[0].run_started.unwrap() >
            &UTC::now().checked_sub(Duration::seconds(60)).unwrap());

    assert_eq!(snapshot[1].state, State::Waiting);
    assert_eq!(snapshot[1].name, "turnip");
    assert_eq!(snapshot[1].run_result, None);
    assert_eq!(&snapshot[1].task_spec, tl.tasks[1][0].task_spec);

}

#[test]
fn execute_sends_started_msg() {
    use factotum::executor::task_list::State;
    use std::sync::mpsc;
    use std::time::Duration;

    let mut ff = Factfile::new("N/A", "test");
    ff.add_task_obj(&make_task("apple", &vec![]));
    ff.add_task_obj(&make_task("turnip", &vec![]));
    ff.add_task_obj(&make_task("orange", &vec!["apple"]));
    ff.add_task_obj(&make_task("egg", &vec!["apple"]));
    ff.add_task_obj(&make_task("potato", &vec!["apple", "egg"]));
    ff.add_task_obj(&make_task("chicken", &vec!["potato", "orange"]));

    let task_count_in_factfile = 6;

    let (tx, rx) = mpsc::channel::<ExecutionUpdate>();

    execute_factfile(&ff,
                     None,
                     execution_strategy::execute_simulation,
                     Some(tx.clone()),
                     None);

    let expected_starting = rx.recv_timeout(Duration::from_millis(300)).unwrap();

    match expected_starting.execution_state {
        ExecutionState::Started => {
            assert!(expected_starting.task_snapshot.iter().all(|t| t.state == State::Waiting));
            assert_eq!(expected_starting.task_snapshot.len(),
                       task_count_in_factfile);
        }
        _ => panic!("Failed! Didn't receive the correct event type"),
    }
}

#[test]
fn execute_sends_running_messages() {
    use factotum::executor::task_list::State;
    use std::sync::mpsc;
    use std::time::Duration;
    use factotum::factfile::Task as FactfileTask;

    let mut ff = Factfile::new("N/A", "test");

    let tasks: Vec<FactfileTask> = vec![make_task("apple", &vec![]),
                                        make_task("turnip", &vec![]),
                                        make_task("orange", &vec!["apple"]),
                                        make_task("egg", &vec!["apple"]),
                                        make_task("potato", &vec!["apple", "egg"]),
                                        make_task("chicken", &vec!["potato", "orange"])];

    for mut task in tasks.into_iter() {
        task.on_result.continue_job.push(0);
        ff.add_task_obj(&task);
    }

    let task_count_in_factfile = 6;

    let (tx, rx) = mpsc::channel::<ExecutionUpdate>();

    let expected_running_message_count = get_task_execution_list(&ff, None).tasks.len();
    let expected_completed_message_count = task_count_in_factfile;
    let count_starting_msg = 1;
    let count_done_msg = 1;

    let total_expected_task_updates = count_starting_msg + count_done_msg +
                                      expected_running_message_count +
                                      expected_completed_message_count;

    execute_factfile(&ff,
                     None,
                     execution_strategy::execute_simulation,
                     Some(tx.clone()),
                     None);

    println!("Expecting {} messages..", total_expected_task_updates);

    for i in 1..total_expected_task_updates {
        let new_msg = rx.recv_timeout(Duration::from_millis(300)).unwrap();

        println!("Received message: ");
        print!("***\n{:?}\n***\n", new_msg);

        if i == 1 {
            assert!(match (new_msg.execution_state, new_msg.task_snapshot) { 
                (ExecutionState::Started, tasks) => {
                    assert!(tasks.iter().all(|t| t.state == State::Waiting));
                    true
                }
                (_, _) => false,
            })
        } else if i == total_expected_task_updates {
            assert!(match (new_msg.execution_state, new_msg.task_snapshot) { 
                (ExecutionState::Finished, tasks) => {
                    assert!(tasks.iter().all(|t| t.state == State::Success));
                    true
                }
                (_, _) => false,
            })
        } else if i > 0 && i < total_expected_task_updates {
            assert!(match (new_msg.execution_state, new_msg.task_snapshot) { 
                (ExecutionState::Running, tasks) => {
                    assert!(tasks.iter().all(|t| {
                        t.state == State::Success || t.state == State::Waiting ||
                        t.state == State::Running
                    }));
                    true
                }
                (_, _) => false,
            })
        } else if i > total_expected_task_updates {
            panic!("Too many messages received")
        } else {
            unreachable!("Uncaught message");
        }
    }
}

#[test]
fn execute_sends_updates_for_skipped_fail() {
    use factotum::executor::task_list::State;
    use std::sync::mpsc;
    use std::time::Duration;
    use factotum::factfile::Task as FactfileTask;

    let mut ff = Factfile::new("N/A", "test");

    let tasks: Vec<FactfileTask> = vec![make_task("apple", &vec![]),
                                        make_task("potato", &vec![]),
                                        make_task("turnip", &vec!["potato"]),
                                        make_task("egg", &vec!["apple", "turnip"])];

    for mut task in tasks.into_iter() {
        if task.name == "potato" {
            task.on_result.continue_job.push(0);
        } else {
            task.on_result.continue_job.push(1);
        }
        ff.add_task_obj(&task);
    }

    let (tx, rx) = mpsc::channel::<ExecutionUpdate>();

    execute_factfile(&ff,
                     None,
                     execution_strategy::execute_simulation,
                     Some(tx.clone()),
                     None);

    let mut recv_msg = vec![];

    for _ in 0..8 {
        if let Ok(v) = rx.recv_timeout(Duration::from_millis(300)) {
            recv_msg.push(v);
        } else {
            panic!("Not enough events received");
        }
    }


    let task_updates = recv_msg.into_iter()
        .filter(|m| match m.transition {
            Transition::Job(_) => false,
            _ => true,
        })
        .collect::<Vec<ExecutionUpdate>>();

    assert_eq!(task_updates.len(), 5);

    let ref final_task_update = task_updates[4];

    println!("{:?}", final_task_update);

    assert_eq!(final_task_update.execution_state, ExecutionState::Running);

    let is_apple_failing =
        match final_task_update.task_snapshot.iter().find(|t| t.name == "apple").unwrap().state {
            State::Failed(_) => true,
            _ => false,
        };
    assert!(is_apple_failing);

    let is_turnip_failing =
        match final_task_update.task_snapshot.iter().find(|t| t.name == "turnip").unwrap().state {
            State::Failed(_) => true,
            _ => false,
        };
    assert!(is_turnip_failing);

    let is_egg_skipped =
        match final_task_update.task_snapshot.iter().find(|t| t.name == "egg").unwrap().state {
            State::Skipped(ref msg) => {
                assert_eq!(msg, "the task 'apple' failed, the task 'turnip' failed");
                true
            }
            _ => false,
        };

    assert!(is_egg_skipped);

    let expected_task_transition =
        vec![TaskTransition::new("turnip",
                                 State::Running,
                                 State::Failed("the task exited with a value not specified in \
                                                continue_job - 0 (task expects one of the \
                                                following return codes to continue [1])"
                                     .to_string())),
             TaskTransition::new("egg",
                                 State::Skipped("the task 'apple' failed".to_string()),
                                 State::Skipped("the task 'apple' failed, the task 'turnip' \
                                                 failed"
                                     .to_string()))];
    assert_eq!(final_task_update.transition,
               Transition::Task(expected_task_transition));
}

#[test]
fn execute_sends_failed_skipped_messages() {
    use factotum::executor::task_list::State;
    use std::sync::mpsc;
    use std::time::Duration;
    use factotum::factfile::Task as FactfileTask;

    let mut ff = Factfile::new("N/A", "test");

    let tasks: Vec<FactfileTask> = vec![make_task("apple", &vec![]),
                                        make_task("turnip", &vec!["apple"])];

    for mut task in tasks.into_iter() {
        task.on_result.continue_job.push(1);
        ff.add_task_obj(&task);
    }

    let (tx, rx) = mpsc::channel::<ExecutionUpdate>();

    execute_factfile(&ff,
                     None,
                     execution_strategy::execute_simulation,
                     Some(tx.clone()),
                     None);

    let mut recv_msg = vec![];

    for _ in 0..5 {
        if let Ok(v) = rx.recv_timeout(Duration::from_millis(300)) {
            recv_msg.push(v);
        } else {
            panic!("Not enough events received");
        }
    }

    let (job_updates, task_updates): (Vec<ExecutionUpdate>, Vec<ExecutionUpdate>) =
        recv_msg.into_iter()
            .partition(|m| match m.transition {
                Transition::Job(_) => true,
                _ => false,
            });

    println!("********* JOB UPDATES ***********");
    for (idx, job_update) in job_updates.iter().enumerate() {
        println!("Received job update: ");
        print!("***\n{}: {:?}\n***\n", idx + 1, job_update);
    }

    assert_eq!(job_updates.len(), 3); // null -> starting, starting->running, started->complete
    assert_eq!(task_updates.len(), 2); // apple running, (apple failed, turnip skipped)

    assert_eq!(job_updates[0].execution_state, ExecutionState::Started);
    assert!(job_updates[0].task_snapshot.iter().all(|t| t.state == State::Waiting));
    assert_eq!(job_updates[0].transition,
               Transition::Job(JobTransition::new(None, ExecutionState::Started)));

    assert_eq!(job_updates[1].execution_state, ExecutionState::Running);
    assert!(job_updates[1].task_snapshot.iter().find(|t| t.name == "apple").unwrap().state ==
            State::Running);
    assert!(job_updates[1].task_snapshot.iter().find(|t| t.name == "turnip").unwrap().state ==
            State::Waiting);
    assert_eq!(job_updates[1].transition,
               Transition::Job(JobTransition::new(Some(ExecutionState::Started),
                                                  ExecutionState::Running)));

    assert_eq!(job_updates[2].execution_state, ExecutionState::Finished);
    let task_apple_failed =
        match job_updates[2].task_snapshot.iter().find(|t| t.name == "apple").unwrap().state {
            State::Failed(_) => true,
            _ => false,
        };
    assert!(task_apple_failed);
    let task_turnip_skipped =
        match job_updates[2].task_snapshot.iter().find(|t| t.name == "turnip").unwrap().state {
            State::Skipped(ref msg) => {
                assert_eq!(msg, "the task 'apple' failed");
                true
            }
            _ => false,
        };
    assert!(task_turnip_skipped);
    assert_eq!(job_updates[2].transition,
               Transition::Job(JobTransition::new(Some(ExecutionState::Running),
                                                  ExecutionState::Finished)));

    println!("********* TASK UPDATES ***********");
    for (idx, task_update) in task_updates.iter().enumerate() {
        println!("Received task update: ");
        print!("***\n{}: {:?}\n***\n", idx + 1, task_update);
    }

    assert_eq!(task_updates[0].execution_state, ExecutionState::Running);
    assert!(task_updates[0].task_snapshot.iter().find(|t| t.name == "apple").unwrap().state ==
            State::Running);
    assert!(task_updates[0].task_snapshot.iter().find(|t| t.name == "turnip").unwrap().state ==
            State::Waiting);
    let expected_first_task_transition =
        vec![TaskTransition::new("apple", State::Waiting, State::Running)];
    assert_eq!(task_updates[0].transition,
               Transition::Task(expected_first_task_transition));


    assert_eq!(task_updates[1].execution_state, ExecutionState::Running);

    let task_apple_failed_task_update =
        match task_updates[1].task_snapshot.iter().find(|t| t.name == "apple").unwrap().state {
            State::Failed(_) => true,
            _ => false,
        };
    assert!(task_apple_failed_task_update);

    let task_turnip_skipped_task_update =
        match task_updates[1].task_snapshot.iter().find(|t| t.name == "turnip").unwrap().state {
            State::Skipped(_) => true,
            _ => false,
        };

    assert!(task_turnip_skipped_task_update);
    let expected_second_task_transition = vec![
        TaskTransition::new("apple", State::Running, State::Failed("the task exited with a value not specified in continue_job - 0 (task expects one of the following return codes to continue [1])".to_string())),
        TaskTransition::new("turnip", State::Waiting, State::Skipped("the task 'apple' failed".to_string())), 
    ];
    assert_eq!(task_updates[1].transition,
               Transition::Task(expected_second_task_transition));
}

#[test]
fn execute_sends_updates_for_skipped_noop() {
    use factotum::executor::task_list::State;
    use std::sync::mpsc;
    use std::time::Duration;
    use factotum::factfile::Task as FactfileTask;

    let mut ff = Factfile::new("N/A", "test");

    let tasks: Vec<FactfileTask> = vec![make_task("apple", &vec![]),
                                        make_task("potato", &vec![]),
                                        make_task("turnip", &vec!["potato"]),
                                        make_task("egg", &vec!["apple", "turnip"])];

    for mut task in tasks.into_iter() {
        if task.name == "potato" {
            task.on_result.continue_job.push(0);
        } else {
            task.on_result.terminate_job.push(0);
        }
        ff.add_task_obj(&task);
    }

    let (tx, rx) = mpsc::channel::<ExecutionUpdate>();

    execute_factfile(&ff,
                     None,
                     execution_strategy::execute_simulation,
                     Some(tx.clone()),
                     None);

    let mut recv_msg = vec![];

    for _ in 0..8 {
        if let Ok(v) = rx.recv_timeout(Duration::from_millis(300)) {
            recv_msg.push(v);
        } else {
            panic!("Not enough events received");
        }
    }


    let task_updates = recv_msg.into_iter()
        .filter(|m| match m.transition {
            Transition::Job(_) => false,
            _ => true,
        })
        .collect::<Vec<ExecutionUpdate>>();

    assert_eq!(task_updates.len(), 5);

    let ref final_task_update = task_updates[4];

    println!("{:?}", final_task_update);

    assert_eq!(final_task_update.execution_state, ExecutionState::Running);

    let is_apple_noop =
        match final_task_update.task_snapshot.iter().find(|t| t.name == "apple").unwrap().state {
            State::SuccessNoop => true,
            _ => false,
        };
    assert!(is_apple_noop);

    let is_turnip_noop =
        match final_task_update.task_snapshot.iter().find(|t| t.name == "turnip").unwrap().state {
            State::SuccessNoop => true,
            _ => false,
        };
    assert!(is_turnip_noop);

    let is_egg_skipped =
        match final_task_update.task_snapshot.iter().find(|t| t.name == "egg").unwrap().state {
            State::Skipped(ref msg) => {
                assert_eq!(msg,
                           "the task 'apple' requested early termination, the task 'turnip' \
                            requested early termination");
                true
            }
            _ => false,
        };

    assert!(is_egg_skipped);

    let expected_task_transition =
        vec![TaskTransition::new("turnip", State::Running, State::SuccessNoop),
             TaskTransition::new("egg",
                                 State::Skipped("the task 'apple' requested early termination"
                                     .to_string()),
                                 State::Skipped("the task 'apple' requested early termination, \
                                                 the task 'turnip' requested early termination"
                                     .to_string()))];
    assert_eq!(final_task_update.transition,
               Transition::Task(expected_task_transition));
}


#[test]
fn execute_sends_noop_skipped_messages() {
    use factotum::executor::task_list::State;
    use std::sync::mpsc;
    use std::time::Duration;
    use factotum::factfile::Task as FactfileTask;

    let mut ff = Factfile::new("N/A", "test");

    let tasks: Vec<FactfileTask> = vec![make_task("apple", &vec![]),
                                        make_task("turnip", &vec!["apple"]),
                                        make_task("egg", &vec!["turnip"])];

    for mut task in tasks.into_iter() {
        task.on_result.terminate_job.push(0);
        ff.add_task_obj(&task);
    }

    let (tx, rx) = mpsc::channel::<ExecutionUpdate>();

    execute_factfile(&ff,
                     None,
                     execution_strategy::execute_simulation,
                     Some(tx.clone()),
                     None);

    let mut recv_msg = vec![];

    for _ in 0..5 {
        if let Ok(v) = rx.recv_timeout(Duration::from_millis(300)) {
            recv_msg.push(v);
        } else {
            panic!("Not enough events received");
        }
    }

    let (job_updates, task_updates): (Vec<ExecutionUpdate>, Vec<ExecutionUpdate>) =
        recv_msg.into_iter()
            .partition(|m| match m.transition {
                Transition::Job(_) => true,
                _ => false,
            });

    println!("********* JOB UPDATES ***********");
    for (idx, job_update) in job_updates.iter().enumerate() {
        println!("Received job update: ");
        print!("***\n{}: {:?}\n***\n", idx + 1, job_update);
    }

    assert_eq!(job_updates.len(), 3); // null -> starting, starting->running, started->complete
    assert_eq!(task_updates.len(), 2); // apple running, (apple failed, turnip skipped, egg skipped)

    assert_eq!(job_updates[0].execution_state, ExecutionState::Started);
    assert!(job_updates[0].task_snapshot.iter().all(|t| t.state == State::Waiting));
    assert_eq!(job_updates[0].transition,
               Transition::Job(JobTransition::new(None, ExecutionState::Started)));

    assert_eq!(job_updates[1].execution_state, ExecutionState::Running);
    assert!(job_updates[1].task_snapshot.iter().find(|t| t.name == "apple").unwrap().state ==
            State::Running);
    assert!(job_updates[1].task_snapshot.iter().find(|t| t.name == "turnip").unwrap().state ==
            State::Waiting);
    assert!(job_updates[1].task_snapshot.iter().find(|t| t.name == "egg").unwrap().state ==
            State::Waiting);
    assert_eq!(job_updates[1].transition,
               Transition::Job(JobTransition::new(Some(ExecutionState::Started),
                                                  ExecutionState::Running)));

    assert_eq!(job_updates[2].execution_state, ExecutionState::Finished);
    let task_apple_noop =
        match job_updates[2].task_snapshot.iter().find(|t| t.name == "apple").unwrap().state {
            State::SuccessNoop => true,
            _ => false,
        };
    assert!(task_apple_noop);

    let task_turnip_skipped =
        match job_updates[2].task_snapshot.iter().find(|t| t.name == "turnip").unwrap().state {
            State::Skipped(ref msg) => {
                assert_eq!(msg, "the task 'apple' requested early termination");
                true
            }
            _ => false,
        };
    assert!(task_turnip_skipped);

    let task_egg_skipped =
        match job_updates[2].task_snapshot.iter().find(|t| t.name == "egg").unwrap().state {
            State::Skipped(ref msg) => {
                assert_eq!(msg, "the task 'apple' requested early termination");
                true
            }
            _ => false,
        };
    assert!(task_egg_skipped);

    assert_eq!(job_updates[2].transition,
               Transition::Job(JobTransition::new(Some(ExecutionState::Running),
                                                  ExecutionState::Finished)));

    println!("********* TASK UPDATES ***********");
    for (idx, task_update) in task_updates.iter().enumerate() {
        println!("Received task update: ");
        print!("***\n{}: {:?}\n***\n", idx + 1, task_update);
    }

    assert_eq!(task_updates[0].execution_state, ExecutionState::Running);
    assert!(task_updates[0].task_snapshot.iter().find(|t| t.name == "apple").unwrap().state ==
            State::Running);
    assert!(task_updates[0].task_snapshot.iter().find(|t| t.name == "turnip").unwrap().state ==
            State::Waiting);
    assert!(task_updates[0].task_snapshot.iter().find(|t| t.name == "egg").unwrap().state ==
            State::Waiting);

    let expected_first_task_transition =
        vec![TaskTransition::new("apple", State::Waiting, State::Running)];
    assert_eq!(task_updates[0].transition,
               Transition::Task(expected_first_task_transition));


    assert_eq!(task_updates[1].execution_state, ExecutionState::Running);

    let task_apple_noop_task_update =
        match task_updates[1].task_snapshot.iter().find(|t| t.name == "apple").unwrap().state {
            State::SuccessNoop => true,
            _ => false,
        };
    assert!(task_apple_noop_task_update);

    let task_turnip_skipped_task_update =
        match task_updates[1].task_snapshot.iter().find(|t| t.name == "turnip").unwrap().state {
            State::Skipped(_) => true,
            _ => false,
        };
    assert!(task_turnip_skipped_task_update);

    let task_egg_skipped_task_update =
        match job_updates[2].task_snapshot.iter().find(|t| t.name == "egg").unwrap().state {
            State::Skipped(ref msg) => {
                assert_eq!(msg, "the task 'apple' requested early termination");
                true
            }
            _ => false,
        };
    assert!(task_egg_skipped_task_update);

    let expected_second_task_transition = vec![
        TaskTransition::new("apple", State::Running, State::SuccessNoop),
        TaskTransition::new("turnip", State::Waiting, State::Skipped("the task 'apple' requested early termination".to_string())), 
        TaskTransition::new("egg", State::Waiting, State::Skipped("the task 'apple' requested early termination".to_string())), 
    ];
    assert_eq!(task_updates[1].transition,
               Transition::Task(expected_second_task_transition));
}

// todo write test for rejecting non "shell" execution types
