.PHONY: install

# the binary into ~/.cargo/bin, nushell completions into a directory nushell autoloads
install:
	cargo install --force --path . --locked
	mkdir -p $(HOME)/.local/share/nushell/vendor/autoload
	$(HOME)/.cargo/bin/viso --completions nushell > $(HOME)/.local/share/nushell/vendor/autoload/viso.nu
	@echo "completions installed; nushell loads them at startup, so run 'exec nu' in open shells"
