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

use factotum::executor::execution_strategy::*;
use std::process::Command;
use chrono::duration::Duration;
use std::cmp;
use std::iter;

fn fill(fillstr: &str, times: usize) -> String {
    iter::repeat(fillstr).take(times).collect::<String>()
}

#[test]
fn simulation_text_good() {

    let mut command = Command::new("sh");
    command.arg("-c");
    command.arg("does_something.sh");
    let task_name = "Simulation Task!";
    let command_text = format!("{:?}", command);

    let text = simulation_text(task_name, &command);

    // FACTOTUM SIMULATION ONLY. THE TASK HAS NOT BEEN EXECUTED.

    // /--------|------------------------------------------------------------------------------\
    // | TASK   | COMMAND                                                                      |
    // |--------|------------------------------------------------------------------------------|
    // | ABC    | sh -c 'potato'                                                               |
    // \--------|------------------------------------------------------------------------------/

    let task_name_width = cmp::max(task_name.len() + 2, "TASK".len() + 2);
    let command_width = cmp::max("COMMAND".len() + 2, command_text.len() + 2);

    println!("task width:{} command width: {}",
             task_name_width,
             command_width);

    let lines = vec![format!("/{}|{}\\",
                             fill("-", task_name_width),
                             fill("-", command_width)),
                     format!("| TASK {}| COMMAND {}|",
                             fill(" ", task_name_width - " TASK ".len()),
                             fill(" ", command_width - " COMMAND ".len())),
                     format!("|{}|{}|",
                             fill("-", task_name_width),
                             fill("-", command_width)),
                     format!("| {:taskwidth$} | {:commandwidth$} |",
                             task_name,
                             command_text,
                             taskwidth = task_name_width - 2,
                             commandwidth = command_width - 2),
                     format!("\\{}|{}/\n",
                             fill("-", task_name_width),
                             fill("-", command_width))];

    let expected = lines.join("\n");

    println!("*** EXPECTED ***");
    println!("{}", expected);
    println!("*** ACTUAL ***");
    println!("{}", text);

    assert_eq!(expected, text);
}

#[test]
fn simulation_returns_good() {
    let mut command: Command = Command::new("banana");
    command.arg("hello_world");
    let result = execute_simulation("hello-world", &mut command);

    assert_eq!(result.return_code, 0);
    assert_eq!(result.duration, Duration::seconds(0).to_std().ok().unwrap());
    assert_eq!(result.stdout.unwrap(),
               simulation_text("hello-world", &command));
    assert!(result.stderr.is_some() == false)
}

#[test]
fn os_execution_notfound() {
    let mut command: Command = Command::new("sh");
    command.arg("-c");
    command.arg("banana");
    let result = execute_os("hello-world", &mut command);

    assert_eq!(result.return_code, 127);
    assert_eq!(result.duration.as_secs(), 0);
    let stderr = result.stderr.unwrap();
    println!("{}", stderr);
    assert!(stderr.contains("banana"));
    assert!(stderr.contains("not found"));
    assert_eq!(result.stdout, None);
    assert_eq!(result.task_execution_error, None)
}

#[test]
fn os_execution_task_exec_failed() {
    let mut command: Command = Command::new("this-doesn't-exist");
    let result = execute_os("hello-world", &mut command);

    // Platform difference: macOS returns -1 (exec fails), Linux returns 127 (command not found)
    assert!(result.return_code == -1 || result.return_code == 127,
            "Expected return_code -1 or 127, got {}", result.return_code);

    if result.return_code == -1 {
        // macOS behavior: exec fails immediately
        assert_eq!(result.duration.as_secs(), 0);
        assert_eq!(result.stderr, None);
        assert_eq!(result.stdout, None);
        let expected_msg = "Error executing process - No such file or directory".to_string();
        assert_eq!(result.task_execution_error.unwrap()[..expected_msg.len()],
                   expected_msg);
    } else {
        // Linux behavior: exec succeeds but shell returns 127
        assert!(result.task_execution_error.is_none());
    }
}

#[test]
fn os_execution_good() {
    let mut command: Command = Command::new("sh");
    command.arg("-c");
    command.arg("type echo");
    let result = execute_os("hello-world", &mut command);

    assert_eq!(result.return_code, 0);
    assert_eq!(result.duration.as_secs(), 0);
    assert_eq!(result.stderr, None);
    assert_eq!(result.stdout.unwrap(), "echo is a shell builtin");
    assert_eq!(result.task_execution_error, None);
}
