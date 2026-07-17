.PHONY: dmg
dmg:
	cargo build --release && ./scripts/installer/macos/package-dmg.sh "$$(cargo metadata --no-deps --format-version 1 | python3 -c 'import sys,json; print(json.load(sys.stdin)["packages"][0]["version"])')"
	@echo "=== DMG 打包完成 ==="
	@ls -lh dist/macos/*.dmg