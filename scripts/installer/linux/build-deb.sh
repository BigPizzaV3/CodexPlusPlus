#!/usr/bin/env bash
set -euo pipefail

# Codex++ Linux .deb 打包脚本
# 用法: sudo bash scripts/installer/linux/build-deb.sh [VERSION]
#       或分步执行下面的各个函数

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
PROJECT_DIR="$REPO_DIR/apps/codex-plus-manager"
BUILD_DIR="$REPO_DIR/.linux-build"
DEB_DIR="$BUILD_DIR/deb"
PACKAGE_NAME="codex-plus-plus"
VERSION="${1:-1.3.0}"
ARCH="$(dpkg --print-architecture)"
# Node / 包管理器：优先从环境变量取，其次探测 PATH。
# 不要写死绝对路径——这里是给所有贡献者和 CI 用的，不是某台机器的构建脚本。
NODE_BIN="${NODE_BIN:-$(command -v node || true)}"
PNPM_BIN="${PNPM_BIN:-$(command -v pnpm || true)}"

echo "=== Codex++ Linux .deb 构建 ==="
echo "版本: $VERSION"
echo "架构: $ARCH"
echo "仓库: $REPO_DIR"

# ── 1. 安装系统依赖（需要 sudo）───────────────────────────────
install_system_deps() {
    echo ">>> 安装系统依赖..."
    sudo apt-get update -qq
    sudo apt-get install -y --no-install-recommends \
        build-essential \
        curl \
        git \
        pkg-config \
        libssl-dev \
        libsqlite3-dev \
        libdbus-1-dev \
        libglib2.0-dev \
        libgtk-3-dev \
        libayatana-appindicator3-dev \
        libjavascriptcoregtk-4.1-dev \
        libwebkit2gtk-4.1-dev \
        libsoup-3.0-dev \
        libz3-dev
    echo ">>> 系统依赖安装完成"
}

# ── 2. 安装 Rust toolchain（无需 sudo）───────────────────────
install_rust() {
    if command -v cargo &>/dev/null; then
        echo ">>> Rust 已安装: $(cargo --version)"
        return
    fi
    echo ">>> 安装 Rust toolchain (rustup)..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal
    echo ">>> Rust 安装完成: $(cargo --version)"
}

# ── 3. 构建前端 ──────────────────────────────────────────────
build_frontend() {
    echo ">>> 构建前端..."
    cd "$PROJECT_DIR"
    # vite 需要 node 在 PATH 上；允许通过 NODE_BIN 指定非 PATH 中的安装。
    if [ -n "$NODE_BIN" ] && [ -x "$NODE_BIN" ]; then
        export PATH="$(dirname "$NODE_BIN"):$PATH"
    fi
    local PM="$PNPM_BIN"
    if [ -z "$PM" ] || [ ! -x "$PM" ]; then
        PM="$(command -v pnpm || true)"
    fi
    if [ -z "$PM" ]; then
        echo "error: 找不到 pnpm。请安装（npm i -g pnpm）或用 PNPM_BIN=/path/to/pnpm 指定。" >&2
        return 1
    fi
    export PATH="$(dirname "$PM"):$PATH"
    "$PM" install
    # Build vite directly (skip npm/pnpm wrapper issues)
    node_modules/.bin/vite build
    echo ">>> 前端构建完成"
}

# ── 4. 构建 Rust 二进制 ──────────────────────────────────────
build_rust() {
    echo ">>> 构建 Rust 二进制..."
    export PATH="$HOME/.cargo/bin:$PATH"
    cargo build --release -p codex-plus-launcher -p codex-plus-manager
    echo ">>> Rust 构建完成"
}

# ── 5. 打包 .deb ─────────────────────────────────────────────
package_deb() {
    echo ">>> 打包 .deb..."
    local stage="$DEB_DIR/usr"
    local deb_name="${PACKAGE_NAME}_${VERSION}_${ARCH}.deb"
    local deb_path="$REPO_DIR/dist/linux/$deb_name"

    mkdir -p "$BUILD_DIR"
    mkdir -p "$(dirname "$deb_path")"

    # Clean previous build
    rm -rf "$DEB_DIR"
    mkdir -p "$DEB_DIR"

    # ── 目录结构 ──
    mkdir -p "$stage/bin"
    mkdir -p "$stage/share/applications"
    mkdir -p "$stage/share/icons/hicolor/128x128/apps"
    mkdir -p "$stage/share/doc/$PACKAGE_NAME"
    mkdir -p "$DEB_DIR/DEBIAN"
    mkdir -p "$stage/lib/${PACKAGE_NAME}"

    # ── 复制二进制 ──
    cp "$REPO_DIR/target/release/codex-plus-plus"          "$stage/bin/"
    cp "$REPO_DIR/target/release/codex-plus-plus-manager"  "$stage/bin/"
    chmod 755 "$stage/bin/codex-plus-plus"
    chmod 755 "$stage/bin/codex-plus-plus-manager"

    # ── 复制前端资源 ──
    if [ -d "$PROJECT_DIR/dist" ]; then
        cp -r "$PROJECT_DIR/dist" "$stage/lib/${PACKAGE_NAME}/"
    fi

    # ── 复制图标 ──
    local icon_src="$PROJECT_DIR/src-tauri/icons/icon.png"
    if [ -f "$icon_src" ]; then
        cp "$icon_src" "$stage/share/icons/hicolor/128x128/apps/${PACKAGE_NAME}-manager.png"
    fi

    # ── Desktop 文件 ──
    cat > "$stage/share/applications/${PACKAGE_NAME}-launcher.desktop" <<'EOF'
[Desktop Entry]
Name=Codex++
Comment=Codex++ Launcher - silently launch the official Codex desktop app
Exec=codex-plus-plus
Icon=codex-plus-plus
Type=Application
Categories=Development;Utility;
Terminal=false
StartupNotify=true
EOF

    cat > "$stage/share/applications/${PACKAGE_NAME}-manager.desktop" <<'EOF'
[Desktop Entry]
Name=Codex++ Manager
Comment=Codex++ Manager - configure providers, models, enhancements
Exec=codex-plus-plus-manager
Icon=codex-plus-plus-manager
Type=Application
Categories=Development;Utility;
Terminal=false
StartupNotify=true
EOF

    # ── 复制文档 ──
    cp "$REPO_DIR/README.md"      "$stage/share/doc/${PACKAGE_NAME}/"
    cp "$REPO_DIR/CHANGELOG.md"   "$stage/share/doc/${PACKAGE_NAME}/"
    cp "$REPO_DIR/LICENSE"        "$stage/share/doc/${PACKAGE_NAME}/"

    # ── control 文件 ──
    local installed_size
    installed_size=$(du -sk "$stage" | awk '{print $1}')
    cat > "$DEB_DIR/DEBIAN/control" <<EOF
Package: ${PACKAGE_NAME}
Version: ${VERSION}
Section: misc
Priority: optional
Architecture: ${ARCH}
Maintainer: BigPizzaV3 <1727732@qq.com>
Installed-Size: 70620
Depends: libc6 (>= 2.34), libgcc-s1 (>= 4.2), libstdc++6 (>= 13), libdbus-1-3, libglib2.0-0t64, libgtk-3-0t64, libwebkit2gtk-4.1-0, libsoup-3.0-0, libjavascriptcoregtk-4.1-0, libssl3t64 (>= 3.0.0), libz3-4, libatspi2.0-0t64, libasound2t64

Recommends: codex
Description: Codex++ is an external launcher and manager for OpenAI Codex / ChatGPT desktop apps.
 It provides provider switching, protocol conversion, session management and UI enhancements
 via Chromium DevTools Protocol and a local helper service, without modifying the official
 app.asar or writing patches to the installation directory.
 .
 Includes two components:
  - codex-plus-plus: silently launches the official Codex desktop app
  - codex-plus-plus-manager: manages providers, models, plugins, sessions, enhancements
EOF

    # ── postinst 脚本 ──
    cat > "$DEB_DIR/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if [ -d "/usr/share/icons/hicolor" ]; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor/ 2>/dev/null || true
fi
if [ -d "/usr/share/applications" ]; then
    update-desktop-database -q /usr/share/applications/ 2>/dev/null || true
fi
exit 0
EOF
    chmod 755 "$DEB_DIR/DEBIAN/postinst"

    # ── prerm 脚本 ──
    cat > "$DEB_DIR/DEBIAN/prerm" <<'EOF'
#!/bin/sh
set -e
if [ -d "/usr/share/icons/hicolor" ]; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor/ 2>/dev/null || true
fi
exit 0
EOF
    chmod 755 "$DEB_DIR/DEBIAN/prerm"

    # ── 创建 deb ──
    dpkg-deb --build "$DEB_DIR" "$deb_path"
    echo ">>> .deb 构建完成: $deb_path"
}

# ── 主流程 ─────────────────────────────────────────────────
main() {
    install_system_deps
    install_rust
    build_frontend
    build_rust
    package_deb

    echo ""
    echo "=== 构建完成 ==="
    local deb_name="${PACKAGE_NAME}_${VERSION}_${ARCH}.deb"
    echo "Deb 包路径: $REPO_DIR/dist/linux/$deb_name"
    echo ""
    echo "安装命令: sudo dpkg -i $REPO_DIR/dist/linux/$deb_name"
}

main "$@"
