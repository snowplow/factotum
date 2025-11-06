# Factotum

[![Release][release-image]][releases] [![Apache License 2.0][license-image]][license]

A dag running tool designed for efficiently running complex jobs with non-trivial dependency trees.

## The zen of Factotum

1. A Turing-complete job is not a job, it's a program
2. A job must be composable from other jobs
3. A job exists independently of any job schedule

## User quickstart

Assuming you're running **64 bit Linux**:

```{bash}
wget https://github.com/snowplow/factotum/releases/download/0.7.0/factotum_0.7.0_linux_x86_64.zip
unzip factotum_0.7.0_linux_x86_64.zip
./factotum --version
```

Factotum requires one argument, which is a **[factotum factfile](/README.md#factfile-format)** that describes the job to run. For example, to run the sample **[sleep.factfile](https://raw.githubusercontent.com/snowplow/factotum/master/samples/sleep.factfile)**:

```{bash}
wget https://raw.githubusercontent.com/snowplow/factotum/master/samples/sleep.factfile
./factotum run sleep.factfile
```
Specifying variables in the job file can be done using `--env JSON` (or `-e JSON`). The `JSON` here is free-form and needs to correspond to the placeholders you've set in your job.

For example, the following will print "hello world!":

```{bash}
wget https://raw.githubusercontent.com/snowplow/factotum/master/samples/variables.factfile
./factotum run variables.factfile --env '{ "message": "hello world!" }'
```

Starting from an arbitrary task can be done using the `--start TASK` or `-s TASK` arguments, where TASK is the name of the task you'd like to start at.

For example, to start at the "echo beta" task in [this job](https://raw.githubusercontent.com/snowplow/factotum/master/samples/echo.factfile), you can run the following:

```{bash}
wget https://raw.githubusercontent.com/snowplow/factotum/master/samples/echo.factfile
./factotum run echo.factfile --start "echo beta"
```

To get a quick overview of the options provided, you can use the `--help` or `-h` argument:

```{bash}
./factotum --help
```

For more information on this file format and how to write your own jobs, see the **[Factfile format](/README.md#factfile-format)** section below.

## Factfile format

Factfiles are self-describing JSON which declare a series of tasks and their dependencies. For example:

```{json}
{
    "schema": "iglu:com.snowplowanalytics.factotum/factfile/jsonschema/1-0-0",
    "data": {
        "name": "Factotum demo",
        "tasks": [
            {
                "name": "echo alpha",
                "executor": "shell",
                "command": "echo",
                "arguments": [ "alpha" ],
                "dependsOn": [],
                "onResult": {
                    "terminateJobWithSuccess": [],
                    "continueJob": [ 0 ]
                }
            },
            {
                "name": "echo beta",
                "executor": "shell",
                "command": "echo",
                "arguments": [ "beta" ],
                "dependsOn": [ "echo alpha" ],
                "onResult": {
                    "terminateJobWithSuccess": [],
                    "continueJob": [ 0 ]
                }
            },
            {
                "name": "echo omega",
                "executor": "shell",
                "command": "echo",
                "arguments": [ "and omega!" ],
                "dependsOn": [ "echo beta" ],
                "onResult": {
                    "terminateJobWithSuccess": [],
                    "continueJob": [ 0 ]
                }
            }
        ]
    }
}
```

This example defines three tasks that run shell commands - `echo alpha`, `echo beta` and `echo omega`. `echo alpha` has no dependencies - it will run immediately. `echo beta` depends
on the completion of the `echo alpha` task, and so will wait for `echo alpha` to complete. `echo omega` depends on the `echo beta` task, and so will wait for `echo beta` to be complete before
executing.

Given the above, the tasks will be executed in the following sequence: `echo alpha`, `echo beta` and finally, `echo omega`. Tasks can have multiple dependencies in factotum, and tasks that are parallelizable will
be run concurrently. Check out **[the samples](/samples)** for more sample factfiles or **[the wiki](https://github.com/snowplow/factotum/wiki#creating-a-job)** for a more complete description of the factfile format.

## Developer quickstart

Factotum is written in **[Rust](https://www.rust-lang.org/)**.

### Using Vagrant

* Clone this repository - `git clone git@github.com:snowplow/factotum.git`
* `cd factotum`
* Set up a Vagrant box and ssh into it - `vagrant up && vagrant ssh`
   * This will take a few minutes
* `cd /vagrant`
* Compile and run a demo - `cargo run -- run samples/echo.factfile`

### Using stable Rust without Vagrant

* **[Install Rust](https://www.rust-lang.org/downloads.html)**
   * on Linux/Mac - `curl -sSf https://static.rust-lang.org/rustup.sh | sh`
* Clone this repository - `git clone git@github.com:snowplow/factotum.git`
* `cd factotum`
* Compile and run a demo - `cargo run -- run samples/echo.factfile`

## Copyright and license

Factotum is copyright 2016-2021 Snowplow Analytics Ltd.

Licensed under the **[Apache License, Version 2.0][license]** (the "License");
you may not use this software except in compliance with the License.

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.

[license-image]: http://img.shields.io/badge/license-Apache--2-blue.svg?style=flat
[license]: http://www.apache.org/licenses/LICENSE-2.0

[release-image]: http://img.shields.io/badge/release-0.7.0-blue.svg?style=flat
[releases]: https://github.com/snowplow/factotum/releases
