PREFIX ?= $(HOME)/.local
CARGO ?= $(HOME)/.cargo/bin/cargo

.PHONY: build test install uninstall
build:
	$(CARGO) build --release --locked

test:
	$(CARGO) test --locked --no-default-features --lib

install: build
	install -Dm755 target/release/cosmic-applet-plex "$(PREFIX)/bin/cosmic-applet-plex"
	install -Dm644 data/plex-symbolic.svg "$(PREFIX)/share/icons/hicolor/scalable/apps/io.github.cosmic-applet-plex.svg"
	install -d "$(PREFIX)/share/applications"
	sed 's|^Exec=.*|Exec="$(PREFIX)/bin/cosmic-applet-plex"|' data/io.github.cosmic-applet-plex.desktop > "$(PREFIX)/share/applications/io.github.cosmic-applet-plex.desktop"

uninstall:
	rm -f "$(PREFIX)/bin/cosmic-applet-plex" "$(PREFIX)/share/applications/io.github.cosmic-applet-plex.desktop" "$(PREFIX)/share/icons/hicolor/scalable/apps/io.github.cosmic-applet-plex.svg"