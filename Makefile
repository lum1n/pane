# Pane. Build and run on macOS; `make test` and `make check-mac` also work on Linux.
APP        := build/Pane.app
INSTALL    := /Applications/Pane.app
SUPPORT    := $(HOME)/Library/Application Support/Pane

.PHONY: app release agent test check-mac install run stop logs recover header link-cli clean

app:
	scripts/build-app.sh

release:
	scripts/release.sh

agent:
	cargo build --release --target aarch64-apple-darwin -p pane-agent -p pane-cli
	@echo "Run: target/aarch64-apple-darwin/release/pane-agentd"

test:
	cargo test --workspace

check-mac:
	scripts/ci-linux.sh

install: app stop
	rm -rf "$(INSTALL)"
	cp -R "$(APP)" "$(INSTALL)"
	@echo "Installed $(INSTALL). Optional: make link-cli"

link-cli:
	sudo ln -sf "$(INSTALL)/Contents/Helpers/pane" /usr/local/bin/pane

stop:
	-pkill -TERM -x Pane; sleep 1

run: app stop
	open "$(APP)"

logs:
	tail -F "$(SUPPORT)"/logs/pane*.log

recover:
	"$(APP)/Contents/Helpers/pane" recover --all-apps

header:
	cbindgen --config crates/pane-ffi/cbindgen.toml --crate pane-ffi --output crates/pane-ffi/include/pane.h
	cp crates/pane-ffi/include/pane.h shell/Sources/CPane/include/pane.h

clean:
	cargo clean
	rm -rf build shell/.build
