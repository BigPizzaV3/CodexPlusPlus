from pathlib import Path


def test_macos_package_dmg_normalizes_arch_and_adds_applications_link():
    text = Path("scripts/installer/macos/package-dmg.sh").read_text(encoding="utf-8")

    assert 'arm64|aarch64) ARCH="arm64"' in text
    assert 'x86_64|amd64|x64) ARCH="x64"' in text
    assert 'ln -s /Applications "$STAGE/Applications"' in text
    assert 'CodexPlusPlus-${VERSION}-macos-${ARCH}.dmg' in text
