# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Factotum is a DAG (Directed Acyclic Graph) running tool designed for efficiently executing complex jobs with non-trivial dependency trees. It's written in Rust and focuses on composable, deterministic job execution.

### Core Philosophy
1. A Turing-complete job is not a job, it's a program
2. A job must be composable from other jobs
3. A job exists independently of any job schedule

## Build Commands

### Development Commands
- `cargo build --verbose` - Debug build
- `cargo build --verbose --release` - Release build
- `cargo test --verbose` - Run all tests
- `cargo run -- run samples/echo.factfile` - Run a sample factfile
- `RUSTFLAGS="-C target-feature=+crt-static" cargo build --release` - Static linking build (Linux only)

### Makefile Commands
- `make debug` - Debug build
- `make release` - Release build
- `make test` - Run tests in Docker across Ubuntu 18.04, 22.04, and 24.04
- `make test-docker` - Same as `make test` (runs all Ubuntu versions)
- `make test-docker-18` - Run tests on Ubuntu 18.04 only
- `make test-docker-22` - Run tests on Ubuntu 22.04 only
- `make test-docker-24` - Run tests on Ubuntu 24.04 only
- `make zip` - Create release zip (requires BUILD_VERSION, PLATFORM, ARCH env vars)
- `make clean` - Clean build artifacts

### Running Factotum
- `./factotum run <factfile>` - Execute a factfile
- `./factotum run <factfile> --start <task_name>` - Start from specific task
- `./factotum run <factfile> --env '{"key": "value"}'` - Pass JSON environment variables
- `./factotum run <factfile> --dry-run` - Simulate execution without running
- `./factotum validate <factfile>` - Validate factfile format
- `./factotum dot <factfile>` - Generate GraphViz dot output
- `./factotum --help` - Show all options

## Architecture

### Core Components

**Factfile Parser (`src/factotum/parser/`)**
- Parses JSON factfiles using Iglu schema validation
- Handles Mustache templating for variable substitution
- Located in `parser/mod.rs`, `parser/schemavalidator/`, `parser/templater/`

**DAG Management (`src/factotum/factfile/`)**
- Uses daggy crate for directed acyclic graph representation
- Factfile struct contains DAG of tasks with dependency relationships
- Task execution order determined by dependency graph traversal

**Executor (`src/factotum/executor/`)**
- `execution_strategy/` - Different execution modes (OS execution, simulation)
- `task_list/` - Manages task states and parallel execution groups
- Executes tasks in dependency order, supports parallel execution of independent tasks

**Sequencer (`src/factotum/sequencer/`)**
- Converts DAG into executable task sequence
- Handles dependency resolution and task ordering

**Webhook Integration (`src/factotum/webhook/`)**
- Sends job execution updates to configured webhook URLs
- Includes job context and execution updates
- Supports backoff retry logic

### Key Data Structures

**Factfile Format**
- Self-describing JSON with Iglu schema
- Tasks have: name, executor, command, arguments, dependencies, result handlers
- `onResult` specifies which return codes continue/terminate the job

**Task States**
- Pending, Running, Success, Failed, Skipped, SuccessNoop
- Tasks can request early job termination via specific return codes

**Execution Flow**
1. Parse and validate factfile JSON
2. Build DAG from task dependencies
3. Resolve execution order via topological sort
4. Execute task groups in parallel where possible
5. Handle task results and propagate failures/early termination

## Dependencies

Key external crates:
- `daggy` - DAG data structure and algorithms
- `docopt` - CLI argument parsing
- `valico` - JSON schema validation
- `mustache` - Template variable substitution
- `chrono` - Date/time handling
- `hyper` - HTTP client for webhooks
- `colored` - Terminal color output
- `log4rs` - Logging framework
- `native-tls` = "=0.2.13" - TLS/SSL support (pinned version)
- `hyper-native-tls` - HTTPS client integration

### OpenSSL/TLS Compatibility Notes
- Uses native-tls which links to system OpenSSL
- Issue #136: Ubuntu Jammy+ uses OpenSSL 3.0 (`libssl.so.3`) vs older versions
- GitHub Actions builds with static linking (`RUSTFLAGS="-C target-feature=+crt-static"`) to avoid dependency issues
- Static linking ensures binaries work across Ubuntu 18.04, 20.04, 22.04, 24.04+

## Testing

Tests are embedded throughout modules using `#[cfg(test)]`. Sample factfiles in `samples/` directory provide integration test cases.

### Running Tests

**Primary Test Method (Docker - Recommended):**
```bash
make test                    # Builds once, then tests binary on Ubuntu 18.04, 22.04, and 24.04
make test-docker-build       # Build binary with Rust 1.78.0 only
make test-docker-18          # Test binary on Ubuntu 18.04 only
make test-docker-22          # Test binary on Ubuntu 22.04 only
make test-docker-24          # Test binary on Ubuntu 24.04 only
```

**Docker Test Workflow:**
1. **Build** (`Dockerfile.build`):
   - Runs unit tests WITHOUT static linking (proc-macros incompatible with `crt-static`)
   - Builds release binary WITH static linking for x86_64
2. **Test** (`Dockerfile.test-ubuntu-*.04`):
   - Runs functional tests of the statically-linked binary on each Ubuntu version
   - Validates binary works on Ubuntu 18.04 (glibc 2.27), 22.04 (glibc 2.35), 24.04 (glibc 2.39)

**Why Docker Testing:**
- **Target Environment Accuracy**: Tests run in the actual Ubuntu environments where Factotum is deployed (18.04, 22.04, 24.04)
- **OpenSSL Compatibility**: Validates against both OpenSSL 1.1.1 (Ubuntu 18.04) and OpenSSL 3.0 (Ubuntu 22.04+)
- **Build Once, Test Everywhere**: Binary is built once and tested on all three Ubuntu versions
- **No macOS Testing**: macOS is not a deployment target, so local macOS tests are not maintained

**Local vs GitHub Testing:**
- **Local**: Runs `make test` which uses Docker to build and test
- **GitHub CI**: Runs `make test` which uses Docker to build and test
- Both use identical Docker-based workflow - consistent results across environments

**Prerequisites:**
- Docker and docker-compose installed
- Dockerfiles: `Dockerfile.build`, `Dockerfile.test-ubuntu-18.04`, `Dockerfile.test-ubuntu-22.04`, `Dockerfile.test-ubuntu-24.04`

### Rust Version Requirements

**Important:** This project uses **Rust 1.78.0** for all builds and tests.

**Why Rust 1.78.0:**
- Compatible with `rustc-serialize 0.3.24` (deprecated 2018)
- Newer versions (1.79+) have breaking changes with this dependency
- Standardized across GitHub Actions CI/CD and local Docker tests
- Works with both OpenSSL 1.1.1 and OpenSSL 3.0

**Docker automatically uses Rust 1.78.0** - no local Rust installation needed for testing.

**For GitHub Actions or local cargo builds:**
The Makefile automatically configures the correct Rust version via `PATH="$HOME/.cargo/bin:$PATH"` which prioritizes rustup over Homebrew Rust.

## Sample Factfiles

The `samples/` directory contains example factfiles demonstrating:
- Basic task sequences (`echo.factfile`)
- Variable substitution (`variables.factfile`, `echo-with-variables.factfile`)
- Tag-based dynamic naming (`echo-with-dynamic-name.factfile`)
- Early termination (`noop.factfile`)

## Logging

Factotum creates a `.factotum/factotum.log` file in the working directory for execution logs.

## GitHub Actions Build Configuration

### Static Linking for Linux Builds

The GitHub Actions workflows (`.github/workflows/cd.yml` and `.github/workflows/ci.yml`) use static linking for Linux builds to ensure compatibility across Ubuntu versions:

```yaml
env:
  RUSTFLAGS: "-C target-feature=+crt-static"
```

This resolves issue #136 where binaries failed on Ubuntu Jammy+ with OpenSSL 3.0 compatibility issues.

### Rust Version

- **Standardized**: Uses Rust 1.78.0 for all builds (CI, CD, and Docker tests)
- **Reason**: Required for compatibility with deprecated `rustc-serialize 0.3.24` dependency
- **Consistency**: Same version across GitHub Actions and Docker test environments
- GitHub Actions uses `ubuntu-latest` (Ubuntu 24.04 as of Dec 2024/Jan 2025)
- Supports cross-compilation for x86_64 and arm64 architectures

### Why Rust 1.78.0 is Standardized

**Background**: The codebase uses `rustc-serialize 0.3.24` (deprecated 2018), which requires careful Rust version selection. After testing, Rust 1.78.0 provides the best balance of compatibility and features.

**Solution**: Standardize on Rust 1.78.0 everywhere for:

#### Consistency Benefits

- Identical behavior between Docker tests and CI/CD
- Same dependency resolution and compilation across Ubuntu 18.04, 22.04, and 24.04
- Single set of setup instructions for contributors

#### Reliability Benefits

- Cargo.lock compatibility across all environments
- Works with both OpenSSL 1.1.1 (Ubuntu 18.04) and OpenSSL 3.0 (Ubuntu 22.04+)
- Predictable build and test behavior in target deployment environments

#### Maintenance Benefits

- Simpler documentation and contributor onboarding
- Single Rust version to track and maintain
- Reduced complexity in build configuration

**Trade-off**: Using Rust 1.78.0 (June 2024) means missing the very latest language features, but provides stability and compatibility. The project's dependencies are already from 2018-2021 era, so this aligns with the existing technical constraints.

**Future**: To use modern Rust versions (1.79+), the project would need significant refactoring to replace `rustc-serialize` with `serde` and update other deprecated dependencies.

### CI/CD Testing Strategy

**CI Workflow (`ci.yml`):**
- Runs on every push to any branch
- Executes `make test` which uses Docker to:
  - Build binary once with Rust 1.78.0 and static linking (using `--platform=linux/amd64`)
  - Run unit tests WITHOUT static linking (proc-macros incompatible with `crt-static`)
  - Build release binary WITH static linking for x86_64
  - Test the statically-linked binary on Ubuntu 18.04, 22.04, and 24.04
- Uses Docker to avoid proc-macro build issues with native `crt-static` on GitHub Actions
- Validates that static linking works across all target Ubuntu versions

**CD Workflow (`cd.yml`):**
- Triggered by git tags
- Builds release binaries for: Linux x86_64, Linux arm64, macOS x86_64, macOS arm64
- Tests are run on Ubuntu builds using `make test-ci` (not macOS builds)
- Static linking applied to Linux builds for cross-version compatibility (`RUSTFLAGS="-C target-feature=+crt-static"`)
- macOS builds compile but do not run tests (not a deployment target)

**Build Configuration:**

- CI uses `make test` which runs Docker-based build and test workflow
- CD uses `make test-ci` (direct cargo test) and `make zip` for release builds
- Docker builds use `--platform=linux/amd64` to emulate x86_64 on Mac and to enable `crt-static` in GitHub Actions
- Static linking (`RUSTFLAGS="-C target-feature=+crt-static"`) ensures binaries work across Ubuntu 18.04 (glibc 2.27), 22.04 (glibc 2.35), and 24.04 (glibc 2.39)
- Unit tests always run WITHOUT `crt-static` due to proc-macro incompatibility
