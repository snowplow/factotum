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

use factotum::factfile::Task;
use factotum::factfile::OnResult;

pub fn compare_tasks(expected: Vec<Vec<&str>>, actual: Vec<Vec<&Task>>) {
    for i in 0..expected.len() {
        for j in 0..expected[i].len() {
            let expected_row = expected.get(i).unwrap();
            let actual_row = actual.get(i).unwrap();
            assert_eq!(expected_row.len(), actual_row.len());
            assert_eq!(expected_row.get(j).unwrap(),
                       &actual_row.get(j).unwrap().name);
        }
    }
}

pub fn make_task(name: &str, depends_on: &Vec<&str>) -> Task {
    Task {
        name: name.to_string(),
        depends_on: depends_on.iter().map(|s| String::from(*s)).collect::<Vec<String>>(),
        executor: "".to_string(),
        command: "".to_string(),
        arguments: vec![],
        on_result: OnResult {
            terminate_job: vec![],
            continue_job: vec![],
        },
    }
}

// Shutdown state is process-wide and terminate_all_children() signals the whole process
// group, so each scenario runs in a child test process that leads its own process group.
#[cfg(unix)]
mod shutdown {
    use factotum::shutdown::*;
    use libc;
    use signal_hook::consts::SIGTERM;
    use signal_hook::flag;
    use std::env;
    use std::io::{BufRead, BufReader};
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::{Command, Stdio};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    const SCENARIO_VAR: &'static str = "FACTOTUM_TEST_SHUTDOWN_SCENARIO";

    fn run_scenario(scenario: &str) {
        // Test names exclude the crate name that module_path! starts with
        let module = module_path!().splitn(2, "::").nth(1).unwrap();
        let test_name = format!("{}::scenario", module);

        let output = Command::new(env::current_exe().unwrap())
            .args(&[&test_name, "--exact", "--ignored", "--nocapture", "--test-threads=1"])
            .env(SCENARIO_VAR, scenario)
            .process_group(0)
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success() && stdout.contains("test result: ok. 1 passed"),
                "scenario '{}' failed ({})\nstdout:\n{}\nstderr:\n{}",
                scenario,
                output.status,
                stdout,
                stderr);
    }

    // Entry point for the child process, a no-op unless launched by run_scenario()
    #[test]
    #[ignore]
    fn scenario() {
        let scenario = match env::var(SCENARIO_VAR) {
            Ok(s) => s,
            Err(_) => return,
        };

        // Never signal the process group of whatever launched the tests
        assert_eq!(unsafe { libc::getpgrp() }, unsafe { libc::getpid() },
                   "scenario must lead its own process group");

        match scenario.as_ref() {
            "flag" => flag_scenario(),
            "terminate" => terminate_scenario(),
            "nothing_registered" => nothing_registered_scenario(),
            other => panic!("unknown scenario '{}'", other),
        }
    }

    fn sigterm_flag() -> Arc<AtomicBool> {
        let received = Arc::new(AtomicBool::new(false));
        flag::register(SIGTERM, received.clone()).unwrap();
        received
    }

    fn sleep_command() -> Command {
        let mut cmd = Command::new("sleep");
        cmd.arg("60").stdout(Stdio::null());
        cmd
    }

    fn flag_scenario() {
        assert!(!is_shutting_down());
        request_shutdown();
        assert!(is_shutting_down());
        request_shutdown();
        assert!(is_shutting_down());
    }

    fn terminate_scenario() {
        let sigterm_received = sigterm_flag();

        // Exits on SIGTERM
        let mut obeys_sigterm = sleep_command().spawn().unwrap();

        // Ignores SIGTERM, so has to be killed. Waits until the trap is set before continuing.
        let mut ignores_sigterm = Command::new("sh")
            .args(&["-c", "trap '' TERM; echo ready; exec sleep 60"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(ignores_sigterm.stdout.take().unwrap()).read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "ready");

        // Unregistered and outside the process group, so should be left alone
        let mut unregistered = sleep_command().process_group(0).spawn().unwrap();

        register_child_process(obeys_sigterm.id());
        register_child_process(ignores_sigterm.id());
        register_child_process(unregistered.id());
        unregister_child_process(unregistered.id());

        let started = Instant::now();
        terminate_all_children();
        let elapsed = started.elapsed();

        let unregistered_status = unregistered.try_wait().unwrap();
        let _ = unregistered.kill();
        let _ = unregistered.wait();

        assert!(elapsed >= Duration::from_secs(5),
                "should wait for graceful shutdown, took {:?}",
                elapsed);
        assert!(sigterm_received.load(Ordering::SeqCst),
                "SIGTERM should be sent to the process group");
        assert_eq!(obeys_sigterm.wait().unwrap().signal(), Some(libc::SIGTERM));
        assert_eq!(ignores_sigterm.wait().unwrap().signal(), Some(libc::SIGKILL));
        assert!(unregistered_status.is_none(), "unregistered process should still be running");
    }

    fn nothing_registered_scenario() {
        let sigterm_received = sigterm_flag();

        let started = Instant::now();
        terminate_all_children();

        assert!(started.elapsed() < Duration::from_secs(1),
                "should return immediately with no child processes");
        assert!(!sigterm_received.load(Ordering::SeqCst), "no signal should be sent");
    }

    #[test]
    fn request_shutdown_sets_flag() {
        run_scenario("flag");
    }

    #[test]
    fn terminate_all_children_sigterms_then_sigkills_survivors() {
        run_scenario("terminate");
    }

    #[test]
    fn terminate_all_children_does_nothing_without_children() {
        run_scenario("nothing_registered");
    }
}
