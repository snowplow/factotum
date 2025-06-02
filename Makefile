.PHONY: debug release zip test check-env clean

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

test:
	cargo test --verbose

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
