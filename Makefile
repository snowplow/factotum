.PHONY: debug release zip test test-docker test-docker-18 test-docker-22 test-docker-24 check-env clean

# -----------------------------------------------------------------------------
#  CONSTANTS
# -----------------------------------------------------------------------------

version = $(shell cat Cargo.toml | grep "^version = \"" | sed -n 's/^.*version = "\(.*\)".*/\1/p' | xargs)

build_dir    = build
target_dir   = target
factotum_dir = .factotum

compiled_dir = $(build_dir)/compiled

# -----------------------------------------------------------------------------
#  BUILDING
# -----------------------------------------------------------------------------

debug:
	cargo build --verbose

release:
	cargo build --verbose --release

zip: release check-env
ifeq ($(version),$(BUILD_VERSION))
	mkdir -p $(compiled_dir)
	(cd target/release && zip -r staging.zip factotum)
	mv target/release/staging.zip $(compiled_dir)/factotum_$(version)_$(PLATFORM)_$(ARCH).zip
else
	$(error BUILD_VERSION and Cargo.toml version do not match - cannot release)
endif

# -----------------------------------------------------------------------------
#  TESTING
# -----------------------------------------------------------------------------

# Local tests - builds with static linking, then tests on all Ubuntu versions
test: test-docker

# Direct cargo test - for CI/CD environments that already have correct Rust
test-ci:
	@echo "========================================"
	@echo "Running tests in CI environment"
	@rustc --version
	@echo "========================================"
	cargo test --verbose

# Build and test on all Ubuntu versions via Docker
test-docker: test-docker-clean test-docker-build test-docker-18 test-docker-22 test-docker-24
	@echo "========================================"
	@echo "All Docker tests completed successfully!"
	@echo "========================================"

test-docker-clean:
	@docker-compose down --remove-orphans 2>/dev/null || true

test-docker-build:
	@echo "========================================"
	@echo "Building with Rust 1.78.0 + static linking"
	@echo "========================================"
	docker-compose up --build --abort-on-container-exit build

test-docker-18:
	@echo "========================================"
	@echo "Testing binary on Ubuntu 18.04"
	@echo "========================================"
	docker-compose up --build --abort-on-container-exit test-ubuntu-18-04

test-docker-22:
	@echo "========================================"
	@echo "Testing binary on Ubuntu 22.04"
	@echo "========================================"
	docker-compose up --build --abort-on-container-exit test-ubuntu-22-04

test-docker-24:
	@echo "========================================"
	@echo "Testing binary on Ubuntu 24.04"
	@echo "========================================"
	docker-compose up --build --abort-on-container-exit test-ubuntu-24-04

# -----------------------------------------------------------------------------
#  HELPERS
# -----------------------------------------------------------------------------

check-env:
ifndef PLATFORM
	$(error PLATFORM is undefined)
endif
ifndef BUILD_VERSION
	$(error BUILD_VERSION is undefined)
endif
ifndef ARCH
	$(error ARCH is undefined)
endif

# -----------------------------------------------------------------------------
#  CLEANUP
# -----------------------------------------------------------------------------

clean:
	rm -rf $(build_dir)
	rm -rf $(target_dir)
	rm -rf $(factotum_dir)
