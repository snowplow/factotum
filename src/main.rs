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

#[macro_use]
extern crate log;
extern crate log4rs;
extern crate docopt;
extern crate daggy;
extern crate rustc_serialize;
extern crate valico;
extern crate colored;
extern crate chrono;
extern crate rand;
extern crate crypto;
extern crate uuid;
extern crate hyper;
extern crate hyper_native_tls;
extern crate libc;
extern crate ifaces;
extern crate dns_lookup;

use docopt::Docopt;
use std::fs;
use factotum::executor::task_list::{Task, State};
use factotum::factfile::Factfile;
use factotum::factfile::Task as FactfileTask;
use factotum::parser::OverrideResultMappings;
use factotum::parser::TaskReturnCodeMapping;
use factotum::executor::execution_strategy::*;
use factotum::webhook::Webhook;
use factotum::executor::ExecutionUpdate;
use factotum::webhook;
use colored::*;
use std::time::Duration;
use std::process::Command;
use std::io::Write;
use std::fs::OpenOptions;
use std::env;
use hyper::Url;
use std::sync::mpsc;
use std::net;
use rustc_serialize::json::{self, Json, ToJson};
use std::collections::BTreeMap;
#[cfg(test)]
use std::fs::File;
use std::collections::HashMap;
use std::error::Error;

mod factotum;

const PROC_SUCCESS: i32 = 0;
const PROC_PARSE_ERROR: i32 = 1;
const PROC_EXEC_ERROR: i32 = 2;
const PROC_OTHER_ERROR: i32 = 3;

const CONSTRAINT_HOST: &'static str = "host";

const VERSION: &'static str = env!("CARGO_PKG_VERSION");
const USAGE: &'static str =
    "
Factotum.

Usage:
  factotum run <factfile> [--start=<start_task>] [--env=<env>] [--dry-run] [--no-colour] [--webhook=<url>] [--tag=<tag>]... [--constraint=<constraint>]... [--max-stdouterr-size=<bytes>]
  factotum validate <factfile> [--no-colour]
  factotum dot <factfile> [--start=<start_task>] [--output=<output_file>] [--overwrite] [--no-colour]
  factotum (-h | --help) [--no-colour]
  factotum (-v | --version) [--no-colour]

Options:
  -h --help                             Show this screen.
  -v --version                          Display the version of Factotum and exit.
  --start=<start_task>                  Begin at specified task.
  --env=<env>                           Supply JSON to define mustache variables in Factfile.
  --dry-run                             Pretend to execute a Factfile, showing the commands that would be executed. Can be used with other options.
  --output=<output_file>                File to print output to. Used with `dot`.
  --overwrite                           Overwrite the output file if it exists.
  --no-colour                           Turn off ANSI terminal colours/formatting in output.
  --webhook=<url>                       Post updates on job execution to the specified URL.
  --tag=<tag>                           Add job metadata (tags).
  --constraint=<constraint>             Checks for an external constraint that will prevent execution; allowed constraints (host).
  --max-stdouterr-size=<bytes>          The maximum size of the individual stdout/err sent via the webhook functions for job updates.
";

#[derive(Debug, RustcDecodable)]
struct Args {
    flag_start: Option<String>,
    flag_env: Option<String>,
    flag_output: Option<String>,
    flag_webhook: Option<String>,
    flag_overwrite: bool,
    flag_dry_run: bool,
    flag_no_colour: bool,
    flag_tag: Option<Vec<String>>,
    flag_constraint: Option<Vec<String>>,
    flag_max_stdouterr_size: Option<usize>,
    arg_factfile: String,
    flag_version: bool,
    cmd_run: bool,
    cmd_validate: bool,
    cmd_dot: bool,
}

// macro to simplify printing to stderr
// https://github.com/rust-lang/rfcs/issues/1078
macro_rules! print_err {
    ($($arg:tt)*) => (
        {
            use std::io::prelude::*;
            if let Err(e) = write!(&mut ::std::io::stderr(), "{}\n", format_args!($($arg)*)) {
                panic!("Failed to write to stderr.\
                    \nOriginal error output: {}\
                    \nSecondary error writing to stderr: {}", format!($($arg)*), e);
            }
        }
    )
}

fn get_duration_as_string(d: &Duration) -> String {
    // duration doesn't support the normal display format
    // for now lets put together something that produces some sensible output
    // e.g.
    // if it's under a minute, show the number of seconds and nanos
    // if it's under an hour, show the number of minutes, and seconds
    // if it's over an hour, show the number of hours, minutes and seconds
    const NANOS_ONE_SEC: f64 = 1000000000_f64;
    const SECONDS_ONE_HOUR: u64 = 3600;

    if d.as_secs() < 60 {
        let mut seconds: f64 = d.as_secs() as f64;
        seconds += d.subsec_nanos() as f64 / NANOS_ONE_SEC;
        format!("{:.1}s", seconds)
    } else if d.as_secs() >= 60 && d.as_secs() < SECONDS_ONE_HOUR {
        // ignore nanos here..
        let secs = d.as_secs() % 60;
        let minutes = (d.as_secs() / 60) % 60;
        format!("{}m, {}s", minutes, secs)
    } else {
        let secs = d.as_secs() % 60;
        let minutes = (d.as_secs() / 60) % 60;
        let hours = d.as_secs() / SECONDS_ONE_HOUR;
        format!("{}h, {}m, {}s", hours, minutes, secs)
    }
}

fn get_task_summary_str(task_results: &Vec<&Task<&FactfileTask>>) -> String {
    let mut total_run_time = Duration::new(0, 0);
    let mut executed = 0;

    for task in task_results.iter() {
        if let Some(ref run_result) = task.run_result {
            total_run_time = total_run_time + run_result.duration;
            executed += 1;
        }
    }

    format!("{}/{} tasks run in {}\n",
            executed,
            task_results.len(),
            get_duration_as_string(&total_run_time)).green().to_string()
}

fn validate_start_task(job: &Factfile, start_task: &str) -> Result<(), &'static str> {
    // A
    // / \
    // B   C
    // / \ /
    // D   E
    //
    // We cannot start at B because E depends on C, which depends on A (simiar for C)
    //
    // A
    // / \
    // B   C
    // / \   \
    // D   E   F
    //
    // It's fine to start at B here though, causing B, D, and E to be run
    //


    match job.can_job_run_from_task(start_task) {
        Ok(is_good) => {
            if is_good {
                Ok(())
            } else {
                Err("the job cannot be started here without triggering prior tasks")
            }
        }
        Err(msg) => Err(msg),
    }
}

fn dot(factfile: &str, start_from: Option<String>) -> Result<String, String> {
    let ff = try!(factotum::parser::parse(factfile, None, OverrideResultMappings::None));
    if let Some(ref start) = start_from {
        match ff.can_job_run_from_task(&start) {
            Ok(is_good) => {
                if !is_good {
                    return Err("the job cannot be started here.".to_string());
                }
            }
            Err(msg) => return Err(msg.to_string()),
        }
    }

    Ok(ff.as_dotfile(start_from))
}

fn validate(factfile: &str, env: Option<Json>) -> Result<String, String> {
    match factotum::parser::parse(factfile, env, OverrideResultMappings::None) {
        Ok(_) => Ok(format!("'{}' is a valid Factfile!", factfile).green().to_string()),
        Err(msg) => Err(msg.red().to_string()),
    }
}

fn parse_file_and_simulate(factfile: &str, env: Option<Json>, start_from: Option<String>) -> i32 {
    parse_file_and_execute_with_strategy(factfile,
                                         env,
                                         start_from,
                                         factotum::executor::execution_strategy::execute_simulation,
                                         OverrideResultMappings::All(TaskReturnCodeMapping {
                                             continue_job: vec![0],
                                             terminate_early: vec![],
                                         }),
                                         None,
                                         None,
                                         None)
}

fn parse_file_and_execute(factfile: &str,
                          env: Option<Json>,
                          start_from: Option<String>,
                          webhook_url: Option<String>,
                          job_tags: Option<HashMap<String, String>>,
                          max_stdouterr_size: Option<usize>)
                          -> i32 {
    parse_file_and_execute_with_strategy(factfile,
                                         env,
                                         start_from,
                                         factotum::executor::execution_strategy::execute_os,
                                         OverrideResultMappings::None,
                                         webhook_url,
                                         job_tags,
                                         max_stdouterr_size)
}

fn parse_file_and_execute_with_strategy<F>(factfile: &str,
                                           env: Option<Json>,
                                           start_from: Option<String>,
                                           strategy: F,
                                           override_result_map: OverrideResultMappings,
                                           webhook_url: Option<String>,
                                           job_tags: Option<HashMap<String, String>>,
                                           max_stdouterr_size: Option<usize>)
                                           -> i32
    where F: Fn(&str, &mut Command) -> RunResult + Send + Sync + 'static + Copy
{

    match factotum::parser::parse(factfile, env, override_result_map) {
        Ok(job) => {

            if let Some(ref start_task) = start_from {
                if let Err(msg) = validate_start_task(&job, &start_task) {
                    warn!("The job could not be started from '{}' because {}",
                          start_task,
                          msg);
                    println!("The job cannot be started from '{}' because {}",
                             start_task.cyan(),
                             msg);
                    return PROC_OTHER_ERROR;
                }
            }

            let (maybe_updates_channel, maybe_join_handle) = if webhook_url.is_some() {
                let url = webhook_url.unwrap();
                let mut wh = Webhook::new(job.name.clone(), job.raw.clone(), url, job_tags, max_stdouterr_size);
                let (tx, rx) = mpsc::channel::<ExecutionUpdate>();
                let join_handle =
                    wh.connect_webhook(rx, Webhook::http_post, webhook::backoff_rand_1_minute);
                (Some(tx), Some(join_handle))
            } else {
                (None, None)
            };

            let job_res = factotum::executor::execute_factfile(&job,
                                                               start_from,
                                                               strategy,
                                                               maybe_updates_channel);

            let mut has_errors = false;
            let mut has_early_finish = false;

            let mut tasks = vec![];

            for task_group in job_res.tasks.iter() {
                for task in task_group {
                    if let State::Failed(_) = task.state {
                        has_errors = true;
                    } else if let State::SuccessNoop = task.state {
                        has_early_finish = true;
                    }
                    tasks.push(task);
                }
            }

            let normal_completion = !has_errors && !has_early_finish;

            let result = if normal_completion {
                let summary = get_task_summary_str(&tasks);
                print!("{}", summary);
                PROC_SUCCESS
            } else if has_early_finish && !has_errors {
                let summary = get_task_summary_str(&tasks);
                print!("{}", summary);
                let incomplete_tasks = tasks.iter()
                    .filter(|r| !r.run_result.is_some())
                    .map(|r| format!("'{}'", r.name.cyan()))
                    .collect::<Vec<String>>()
                    .join(", ");
                let stop_requesters = tasks.iter()
                    .filter(|r| match r.state {
                        State::SuccessNoop => true,
                        _ => false,
                    })
                    .map(|r| format!("'{}'", r.name.cyan()))
                    .collect::<Vec<String>>()
                    .join(", ");
                println!("Factotum job finished early as a task ({}) requested an early finish. \
                          The following tasks were not run: {}.",
                         stop_requesters,
                         incomplete_tasks);
                PROC_SUCCESS
            } else {
                let summary = get_task_summary_str(&tasks);
                print!("{}", summary);

                let incomplete_tasks = tasks.iter()
                    .filter(|r| !r.run_result.is_some())
                    .map(|r| format!("'{}'", r.name.cyan()))
                    .collect::<Vec<String>>()
                    .join(", ");

                let failed_tasks = tasks.iter()
                    .filter(|r| match r.state {
                        State::Failed(_) => true,
                        _ => false,
                    })
                    .map(|r| format!("'{}'", r.name.cyan()))
                    .collect::<Vec<String>>()
                    .join(", ");

                println!("Factotum job executed abnormally as a task ({}) failed - the following \
                          tasks were not run: {}!",
                         failed_tasks,
                         incomplete_tasks);
                PROC_EXEC_ERROR
            };

            if maybe_join_handle.is_some() {
                print!("Waiting for webhook to finish sending events...");
                let j = maybe_join_handle.unwrap();
                let webhook_res = j.join().ok().unwrap();
                println!("{}", " done!".green());

                if webhook_res.events_received > webhook_res.success_count {
                    println!("{}", "Warning: some events failed to send".red());
                }
            }

            result
        } 
        Err(msg) => {
            println!("{}", msg);
            return PROC_PARSE_ERROR;
        }      
    }
}

fn write_to_file(filename: &str, contents: &str, overwrite: bool) -> Result<(), String> {
    let mut f = if overwrite {
        match OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(filename) {
            Ok(f) => f,
            Err(io) => return Err(format!("couldn't create file '{}' ({})", filename, io)),        
        }
    } else {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(filename) {
            Ok(f) => f,
            Err(io) => return Err(format!("couldn't create file '{}' ({})", filename, io)),        
        }
    };

    match f.write_all(contents.as_bytes()) {
        Ok(_) => Ok(()),
        Err(msg) => Err(format!("couldn't write to file '{}' ({})", filename, msg)),
    }
}

fn is_valid_url(url: &str) -> Result<(), String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        match Url::parse(url) {
            Ok(_) => Ok(()),
            Err(msg) => Err(format!("{}", msg)),
        }
    } else {
        Err("URL must begin with 'http://' or 'https://' to be used with Factotum webhooks".into())
    }
}

fn get_constraint_map(constraints: &Vec<String>) -> HashMap<String, String> {
    get_tag_map(constraints)
}

fn is_valid_host(host: &str) -> Result<(), String> {
    if host == "*" {
        return Ok(());
    }

    let os_hostname = try!(gethostname_safe().map_err(|e| e.to_string()));

    if host == os_hostname {
        return Ok(());
    }

    let external_addrs = try!(get_external_addrs().map_err(|e| e.to_string()));
    let host_addrs = try!(dns_lookup::lookup_host(&host)
        .map_err(|_| "could not find any IPv4 addresses for the supplied hostname"));

    for host_addr in host_addrs {
        if let Ok(good_host_addr) = host_addr {
            if external_addrs.iter().any(|external_addr| external_addr.ip() == good_host_addr) {
                return Ok(());
            }
        }
    }

    Err("failed to match any of the interface addresses to the found host addresses".into())
}

extern "C" {
    pub fn gethostname(name: *mut libc::c_char, size: libc::size_t) -> libc::c_int;
}

fn gethostname_safe() -> Result<String, String> {
    let len = 255;
    let mut buf = Vec::<u8>::with_capacity(len);

    let ptr = buf.as_mut_slice().as_mut_ptr();

    let err = unsafe { gethostname(ptr as *mut libc::c_char, len as libc::size_t) } as libc::c_int;

    match err {
        0 => {
            let mut _real_len = len;
            let mut i = 0;
            loop {
                let byte = unsafe { *(((ptr as u64) + (i as u64)) as *const u8) };
                if byte == 0 {
                    _real_len = i;
                    break;
                }
                i += 1;
            }
            unsafe { buf.set_len(_real_len) }
            Ok(String::from_utf8_lossy(buf.as_slice()).into_owned())
        }
        _ => {
            Err("could not get hostname from system; cannot compare against supplied hostname"
                .into())
        }
    }
}

fn get_external_addrs() -> Result<Vec<net::SocketAddr>, String> {
    let mut external_addrs = vec![];

    for iface in ifaces::Interface::get_all().unwrap().into_iter() {
        if iface.kind == ifaces::Kind::Ipv4 {
            if let Some(addr) = iface.addr {
                if !addr.ip().is_loopback() {
                    external_addrs.push(addr)
                }
            }
        }
    }

    if external_addrs.len() == 0 {
        Err("could not find any non-loopback IPv4 addresses in the network interfaces; do you \
             have a working network interface card?"
            .into())
    } else {
        Ok(external_addrs)
    }
}

fn get_tag_map(args: &Vec<String>) -> HashMap<String, String> {
    let mut arg_map: HashMap<String, String> = HashMap::new();

    for arg in args.iter() {
        let split = arg.split(",").collect::<Vec<&str>>();
        if split.len() >= 2 && split[0].trim().is_empty() == false {
            let key = split[0].trim().to_string();
            let value = split[1..].join("").trim().to_string();
            arg_map.insert(key, value);
        } else if split.len() == 1 && split[0].trim().is_empty() == false {
            let key = split[0].trim().to_string();
            let value = "".to_string();
            arg_map.insert(key, value);
        }
    }

    arg_map
}

#[test]
fn test_tag_map() {
    let easy = get_tag_map(&vec!["hello,world".to_string()]);
    let mut expected_easy = HashMap::new();
    expected_easy.insert("hello".to_string(), "world".to_string());
    assert_eq!(easy, expected_easy);

    let trim_leading_trailing = get_tag_map(&vec!["  hello   ,  world   ".to_string()]);
    assert_eq!(trim_leading_trailing, expected_easy);

    let missing_value = get_tag_map(&vec!["  hello   ".to_string()]);
    let mut expected_missing_value = HashMap::new();
    expected_missing_value.insert("hello".to_string(), "".to_string());
    assert_eq!(missing_value, expected_missing_value);

    let empty = get_tag_map(&vec![" ".to_string()]);
    assert_eq!(empty, HashMap::new());

    let empty_key = get_tag_map(&vec![" , asdas".to_string()]);
    assert_eq!(empty_key, HashMap::new());

    let with_comma = get_tag_map(&vec!["the rain,first,, wow,,".to_string()]);
    let mut expected_comma = HashMap::new();
    expected_comma.insert("the rain".to_string(), "first wow".to_string());
    assert_eq!(with_comma, expected_comma);
}

fn json_str_to_btreemap(j: &str) -> Result<BTreeMap<String, String>, String> {
    json::decode(j).map_err(|err| {
        format!("Supplied string '{}' is not valid JSON: {}",
                j,
                Error::description(&err))
    })
}

fn str_to_json(s: &str) -> Result<Json, String> {
    Json::from_str(s).map_err(|err| {
        format!("Supplied string '{}' is not valid JSON: {}",
                s,
                Error::description(&err))
    })
}

#[test]
fn str_to_json_produces_json() {
    let sample = "{\"hello\":\"world\"}";
    if let Ok(j) = str_to_json(sample) {
        assert_eq!(Json::from_str(sample).unwrap(), j)
    } else {
        panic!("valid json did not produce inflated json")
    }
}

#[test]
fn str_to_json_bad_json() {
    let invalid = "{\"hello\":\"world\""; // missing final }
    if let Err(msg) = str_to_json(invalid) {
        assert_eq!("Supplied string '{\"hello\":\"world\"' is not valid JSON: failed \
                    to parse json",
                   msg)
    } else {
        panic!("invalid json parsed successfully")
    }
}

fn get_log_config() -> Result<log4rs::config::Config, String> {    
    let file_appender = match log4rs::appender::FileAppender::builder(".factotum/factotum.log").build() {
        Ok(fa) => fa,
        Err(e) => {
            let cwd = env::current_dir().expect("Unable to get current working directory");
            let expanded_path = format!("{}{}{}", cwd.display(), std::path::MAIN_SEPARATOR, ".factotum/factotum.log");
            return Err(format!("couldn't create logfile appender to '{}'. Reason: {}", expanded_path, e.description()));
        }
    };

    let root = log4rs::config::Root::builder(log::LogLevelFilter::Info)
        .appender("file".to_string());

    log4rs::config::Config::builder(root.build())
        .appender(log4rs::config::Appender::builder("file".to_string(),
                                                    Box::new(file_appender)).build())
        .build().map_err(|e| format!("error setting logging. Reason: {}", e.description()))
}

fn init_logger() -> Result<(), String> {
    match fs::create_dir(".factotum") {
        Ok(_) => (),
        Err(e) => match e.kind() {
            std::io::ErrorKind::AlreadyExists => (),
            _ => {
                let cwd = env::current_dir().expect("Unable to get current working directory");
                let expected_path =  format!("{}{}{}{}", cwd.display(), std::path::MAIN_SEPARATOR, ".factotum", std::path::MAIN_SEPARATOR);
                return Err(format!("unable to create directory '{}' for logfile. Reason: {}", expected_path, e.description()))
            }
        }
    };
    let log_config = try!(get_log_config());
    log4rs::init_config(log_config).map_err(|e| format!("couldn't initialize log configuration. Reason: {}", e.description()))
}

fn main() {
    std::process::exit(factotum())
}

fn factotum() -> i32 {
    if let Err(log) = init_logger() {
        println!("Log initialization error: {}", log);
        return PROC_OTHER_ERROR;
    }

    let args: Args = match Docopt::new(USAGE).and_then(|d| d.decode()) {
        Ok(a) => a,
        Err(e) => {
            print!("{}", e);
            return PROC_OTHER_ERROR;
        }
    };

    let tag_map = if let Some(tags) = args.flag_tag {
        Some(get_tag_map(&tags))
    } else {
        None
    };

    // Environment should always be present as tags can populate the env
    let env_str: String = if let Some(c) = args.flag_env {
        c
    } else {
        "{}".to_string()
    };

    let env_json: Option<Json> = {
        match json_str_to_btreemap(&env_str) {
            Ok(mut a) => {
                if let Some(tm) = tag_map.as_ref() {
                    for (key, value) in tm {
                        let tag_key = format!("tag:{}", key.to_string());
                        a.insert(tag_key, value.to_string());
                    }
                }

                match str_to_json(&json::encode(&a).unwrap()) {
                    Ok(a) => {
                        Some(a)
                    }
                    Err(e) => {
                        print!("{}", e);
                        return PROC_OTHER_ERROR;
                    }
                }
            }
            Err(e) => {
                print!("{}", e);
                return PROC_OTHER_ERROR;
            }
        }
    };

    if args.flag_no_colour {
        env::set_var("CLICOLOR", "0");
    }

    if args.flag_version {
        println!("Factotum version {}", VERSION);
        return PROC_SUCCESS;
    }

    if args.flag_dry_run && args.flag_webhook.is_some() {
        println!("{}",
                 "Error: --webhook cannot be used with the --dry-run option".red());
        return PROC_OTHER_ERROR;
    }

    if let Some(ref wh) = args.flag_webhook {
        if let Err(msg) = is_valid_url(&wh) {
            println!("{}",
                     format!("Error: the specifed webhook URL \"{}\" is invalid. Reason: {}",
                             wh,
                             msg)
                         .red());
            return PROC_OTHER_ERROR;
        }
    }

    if args.cmd_run {
        if let Some(constraints) = args.flag_constraint {
            let c_map = get_constraint_map(&constraints);

            if let Some(host_value) = c_map.get(CONSTRAINT_HOST) {
                if let Err(msg) = is_valid_host(host_value) {
                    println!("{}",
                             format!("Warn: the specifed host constraint \"{}\" did not match, \
                                      no tasks have been executed. Reason: {}",
                                     host_value,
                                     msg)
                                 .yellow());
                    return PROC_SUCCESS;
                }
            }
        }

        if !args.flag_dry_run {
            parse_file_and_execute(&args.arg_factfile,
                                   env_json,
                                   args.flag_start,
                                   args.flag_webhook,
                                   tag_map,
                                   args.flag_max_stdouterr_size)
        } else {
            parse_file_and_simulate(&args.arg_factfile, env_json, args.flag_start)
        }
    } else if args.cmd_validate {
        match validate(&args.arg_factfile, env_json) {
            Ok(msg) => {
                println!("{}", msg);
                PROC_SUCCESS
            }
            Err(msg) => {
                println!("{}", msg);
                PROC_PARSE_ERROR
            }
        }
    } else if args.cmd_dot {
        match dot(&args.arg_factfile, args.flag_start) {
            Ok(dot) => {
                if let Some(output_file) = args.flag_output {
                    match write_to_file(&output_file, &dot, args.flag_overwrite) {
                        Ok(_) => {
                            println!("{}", "File written successfully".green());
                            PROC_SUCCESS
                        }
                        Err(m) => {
                            print_err!("{}{}", "Error: ".red(), m.red());
                            PROC_OTHER_ERROR
                        }                        
                    }
                } else {
                    print!("{}", dot);
                    PROC_SUCCESS
                }
            }
            Err(msg) => {
                print_err!("{} {}", "Error:".red(), msg.red());
                PROC_OTHER_ERROR
            }
        }
    } else {
        unreachable!("Unknown subcommand!")
    }
}

#[test]
fn test_is_valid_url() {
    match is_valid_url("http://") {
        Ok(_) => panic!("http:// is not a valid url"),
        Err(msg) => assert_eq!(msg, "empty host"),
    }

    match is_valid_url("http://potato.com/") {
        Ok(_) => (),
        Err(_) => panic!("http://potato.com/ is a valid url"),
    }

    match is_valid_url("https://potato.com/") {
        Ok(_) => (),
        Err(_) => panic!("https://potato.com/ is a valid url"),
    }

    match is_valid_url("potato.com/") {
        Ok(_) => panic!("no http/s?"),
        Err(msg) => {
            assert_eq!(msg,
                       "URL must begin with 'http://' or 'https://' to be used with Factotum \
                        webhooks")
        } // this is good
    }
}

#[test]
fn test_write_to_file() {
    use std::env;
    use std::io::Read;

    let test_file = "factotum-write-test.txt";
    let mut dir = env::temp_dir();
    dir.push(test_file);

    let test_path = &str::replace(&format!("{:?}", dir.as_os_str()), "\"", "");
    println!("test file path: {}", test_path);

    fs::remove_file(test_path).ok();

    assert!(match write_to_file(test_path, "helloworld", false) {
        Ok(_) => true,
        Err(msg) => panic!("Unexpected error: {}", msg),
    });
    assert!(write_to_file(test_path, "helloworld", false).is_err());
    assert!(write_to_file(test_path, "helloworld all", true).is_ok());

    let mut file = File::open(test_path).unwrap();
    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();

    assert_eq!(contents, "helloworld all");

    assert!(fs::remove_file(test_path).is_ok());

    // check that overwrite will also write a new file (https://github.com/snowplow/factotum/issues/97)

    assert!(write_to_file(test_path, "overwrite test", true).is_ok());
    assert!(fs::remove_file(test_path).is_ok());
}

#[test]
fn validate_ok_factfile_good() {
    let test_file_path = "./tests/resources/example_ok.factfile";
    let is_valid = validate(test_file_path, None);
    let expected: String = format!("'{}' is a valid Factfile!", test_file_path).green().to_string();
    assert_eq!(is_valid, Ok(expected));
}

#[test]
fn validate_ok_factfile_bad() {
    let test_file_path = "./tests/resources/invalid_json.factfile";
    let is_valid = validate(test_file_path, None);
    match is_valid {
        Ok(_) => panic!("Validation returning valid for invalid file"),
        Err(msg) => {
            let expected = format!("'{}' is not a valid", test_file_path);
            assert!(msg.contains(&expected))
        }
    }
}

#[test]
fn have_valid_config() {
    fs::create_dir(".factotum").ok();
    if let Err(errs) = get_log_config() {
        panic!("config not building correctly! {:?}", errs);
    }
}

#[test]
fn get_duration_under_minute() {
    assert_eq!(get_duration_as_string(&Duration::new(2, 500000099)),
               "2.5s".to_string());
    assert_eq!(get_duration_as_string(&Duration::new(0, 0)),
               "0.0s".to_string());
}

#[test]
fn get_duration_under_hour() {
    assert_eq!(get_duration_as_string(&Duration::new(62, 500000099)),
               "1m, 2s".to_string()); // drop nanos for minute level precision
    assert_eq!(get_duration_as_string(&Duration::new(59 * 60 + 59, 0)),
               "59m, 59s".to_string());
}

#[test]
fn get_duration_with_hours() {
    assert_eq!(get_duration_as_string(&Duration::new(3600, 0)),
               "1h, 0m, 0s".to_string());
    assert_eq!(get_duration_as_string(&Duration::new(3600 * 10 + 63, 0)),
               "10h, 1m, 3s".to_string());
}

#[test]
fn test_start_task_validation_not_present() {
    let mut factfile = Factfile::new("N/A", "test");

    match validate_start_task(&factfile, "something") {
        Err(r) => assert_eq!(r, "the task specified could not be found"),
        _ => unreachable!("validation did not fail"),
    }

    factfile.add_task("something", &vec![], "", "", &vec![], &vec![], &vec![]);
    if let Err(_) = validate_start_task(&factfile, "something") {
        unreachable!("validation failed when task present")
    }
}

#[test]
fn test_start_task_cycles() {

    use factotum::factfile::*;

    let mut factfile = Factfile::new("N/A", "test");

    let task_a = Task {
        name: "a".to_string(),
        depends_on: vec![],
        executor: "".to_string(),
        command: "".to_string(),
        arguments: vec![],
        on_result: OnResult {
            terminate_job: vec![],
            continue_job: vec![],
        },
    };

    let task_b = Task {
        name: "b".to_string(),
        depends_on: vec!["a".to_string()],
        executor: "".to_string(),
        command: "".to_string(),
        arguments: vec![],
        on_result: OnResult {
            terminate_job: vec![],
            continue_job: vec![],
        },
    };

    let task_c = Task {
        name: "c".to_string(),
        depends_on: vec!["a".to_string()],
        executor: "".to_string(),
        command: "".to_string(),
        arguments: vec![],
        on_result: OnResult {
            terminate_job: vec![],
            continue_job: vec![],
        },
    };

    let task_d = Task {
        name: "d".to_string(),
        depends_on: vec!["c".to_string(), "b".to_string()],
        executor: "".to_string(),
        command: "".to_string(),
        arguments: vec![],
        on_result: OnResult {
            terminate_job: vec![],
            continue_job: vec![],
        },
    };

    factfile.add_task_obj(&task_a);
    factfile.add_task_obj(&task_b);
    factfile.add_task_obj(&task_c);
    factfile.add_task_obj(&task_d);

    match validate_start_task(&factfile, "c") {
        Err(r) => {
            assert_eq!(r,
                       "the job cannot be started here without triggering prior tasks")
        }
        _ => unreachable!("the task validated when it shouldn't have"),
    }
}

#[test]
fn test_gethostname_safe() {
    let hostname = gethostname_safe();
    if let Ok(ok_hostname) = hostname {
        assert!(!ok_hostname.is_empty());
    } else {
        panic!("gethostname_safe() must return a Ok(<String>)");
    }
}

#[test]
fn test_get_external_addrs() {
    let external_addrs = get_external_addrs();
    if let Ok(ok_external_addrs) = external_addrs {
        assert!(ok_external_addrs.len() > 0);
    } else {
        panic!("get_external_addrs() must return a Ok(Vec<net::SocketAddr>) that is non-empty");
    }
}

#[test]
fn test_is_valid_host() {
    is_valid_host("*").expect("must be Ok() for wildcard");

    // Test each external addr is_valid_host
    let external_addrs = get_external_addrs()
        .expect("get_external_addrs() must return a Ok(Vec<net::SocketAddr>) that is non-empty");
    for external_addr in external_addrs {
        let ip_str = external_addr.ip().to_string();
        is_valid_host(&ip_str).expect(&format!("must be Ok() for IP {}", &ip_str));
    }
}
