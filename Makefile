.PHONY: test build dev release

test:
	cargo build --release

build:
	cargo build --release

dev:
	cargo run

release:
	bash scripts/release/pushReleaseTag.sh $(RELEASE_FLAGS)
