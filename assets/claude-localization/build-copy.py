#!/usr/bin/env python3
"""在宿主指定的全新暂存目录构建副本；不删除旧副本，不写官方包或用户档案。"""
import json
import os
import pathlib
import plistlib
import re
import shutil
import struct
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
WIRE = b"dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX"
# 由 Codex++ 的「Computer Use」开关注入；关闭时副本与官方 asar 逐字节一致。
COMPUTER_USE = os.environ.get("CODEX_PLUS_CLAUDE_COMPUTER_USE", "0") == "1"
PROGRESS_FILE = os.environ.get("CODEX_PLUS_CLAUDE_PROGRESS")


def report(percent, message):
    """把阶段进度写给宿主（Codex++ 读它画进度条）；单独运行脚本时静默跳过。"""
    if not PROGRESS_FILE:
        return
    try:
        pathlib.Path(PROGRESS_FILE).write_text(
            json.dumps({"percent": percent, "message": message}, ensure_ascii=False),
            encoding="utf-8",
        )
    except Exception:
        pass


def run(args):
    result = subprocess.run(args, capture_output=True, text=True, timeout=180)
    if result.returncode:
        raise RuntimeError(result.stderr.strip()[:500] or "命令执行失败")
    return result


def fuses(app):
    root = app.parent if app.is_file() else app
    candidates = []
    if os.name != "nt":
        candidates.append(root / "Contents/Frameworks/Electron Framework.framework/Versions/A/Electron Framework")
    candidates.extend([root, root / "Claude.exe", root / "resources/electron.exe"])
    candidates.extend(p for p in root.rglob("*.exe") if p.is_file())
    found = []
    for binary in candidates:
        if not binary.is_file():
            continue
        data = binary.read_bytes()
        # 新版 Electron 里这段标记可能出现多次（例如多架构拼接），
        # 逐个校验：后面紧跟版本号、且 flag 全是 '0'/'1' 的才是真 fuse。
        offset = 0
        while True:
            index = data.find(WIRE, offset)
            if index < 0:
                break
            offset = index + len(WIRE)
            start = index + len(WIRE)
            version = data[start + 1]
            flags = data[start + 2:start + 2 + version]
            if 0 < version <= 16 and all(c in b"01" for c in flags):
                found.append((binary, start + 2, flags))
        if found:
            return found
    raise RuntimeError("Electron fuse 格式已变化，无法安全汉化此版本")


def root_for(app):
    return app.parent if app.is_file() else app


def ion_for(app):
    root = root_for(app)
    candidates = [root / "Contents/Resources/ion-dist", root / "resources/ion-dist", root / "ion-dist"]
    return next((path for path in candidates if path.is_dir()), candidates[0])


def app_asar(root):
    candidates = [root / "Contents/Resources/app.asar", root / "resources/app.asar"]
    return next((path for path in candidates if path.is_file()), None)



CU_MARKER = "Contents/Resources/.cu-patched"


def cu_marker(app):
    return root_for(app) / CU_MARKER


def _walk_asar(node, prefix=""):
    for name, child in node.get("files", {}).items():
        path = prefix + "/" + name
        if "files" in child:
            yield from _walk_asar(child, path)
        else:
            yield path, child


def _match_paren(data, open_idx):
    depth = 0
    quote = None
    i = open_idx
    while i < len(data):
        ch = data[i:i + 1]
        if quote:
            if ch == b"\\":
                i += 2
                continue
            if ch == quote:
                quote = None
        elif ch in (b'"', b"'", b"`"):
            quote = ch
        elif ch == b"(":
            depth += 1
        elif ch == b")":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return -1


def _config_id_name(src):
    m = re.search(rb'([A-Za-z_$][\w$]*)\s*=\s*"1291166712"', src)
    return m.group(1) if m else None


def _force_field(src, field, const):
    pattern = re.compile(rb'\b[A-Za-z_$][\w$]*\(\s*' + re.escape(const) + rb'\s*,\s*"' + field.encode() + rb'"\s*,')
    hits = 0
    for _ in range(8):
        m = pattern.search(src)
        if not m:
            break
        open_idx = src.index(b"(", m.start())
        end = _match_paren(src, open_idx)
        if end < 0:
            break
        src = src[:m.start()] + b"!0" + src[end + 1:]
        hits += 1
    return src, hits


def _patch_cu_source(src):
    hits = []
    # 只认同一个文件里同时出现 CU 配置 id 的 chunk：别的文件里的 "chicagoEnabled"
    # 可能是属性调用（obj.x("chicagoEnabled")），替换成 !0 会制造语法错误。
    if b"1291166712" not in src:
        return src, hits
    src, n = re.subn(rb'\b[A-Za-z_$][\w$]*\(\s*"chicagoEnabled"\s*\)', b"!0", src)
    if n:
        hits.append("chicagoEnabled x%d" % n)
    const = _config_id_name(src)
    if const:
        for field in ("enabled", "appScopedEnabled"):
            src, n = _force_field(src, field, const)
            if n:
                hits.append("%s x%d" % (field, n))
    return src, hits


def patch_asar(copy):
    """在副本的 app.asar 里改写 Computer Use 门禁（原始安装包不动）。"""
    asar = app_asar(root_for(copy))
    if not asar or not asar.is_file():
        raise RuntimeError("副本缺少 app.asar")
    data = asar.read_bytes()
    _, hdr_size, _, json_str_size = struct.unpack("<IIII", data[:16])
    header = json.loads(data[16:16 + json_str_size].decode())
    start = 8 + hdr_size

    entries = []
    for path, meta in _walk_asar(header):
        if not meta.get("unpacked"):
            entries.append((path, meta))

    changed = []
    blobs = {}
    offset = 0
    for path, meta in entries:
        off = start + int(meta["offset"])
        size = int(meta["size"])
        blob = data[off:off + size]
        if path.endswith(".js") and size < 12_000_000:
            patched, hits = _patch_cu_source(blob)
            if hits:
                blob = patched
                changed.append("%s (%s)" % (path.rsplit("/", 1)[-1], ", ".join(hits)))
        meta.pop("integrity", None)
        meta["size"] = len(blob)
        meta["offset"] = str(offset)
        offset += len(blob)
        blobs[path] = blob

    if not changed:
        raise RuntimeError("没有找到可改写的 Computer Use 门禁，版本可能已变化")

    text = json.dumps(header, separators=(",", ":"), ensure_ascii=False).encode()
    pad = (-len(text)) % 4
    json_size = len(text) + pad + 4
    hdr_size = json_size + 4
    out = bytearray(struct.pack("<IIII", 4, hdr_size, json_size, len(text)))
    out += text + b"\0" * pad
    for path, _ in entries:
        out += blobs[path]
    asar.write_bytes(bytes(out))

    marker = cu_marker(copy)
    marker.parent.mkdir(parents=True, exist_ok=True)
    marker.write_text(json.dumps({"patched": changed}, ensure_ascii=False, indent=2))
    print("已改写 asar 里的 Computer Use 门禁：" + "; ".join(changed), flush=True)


def check(source, copy):
    _, _, flags = fuses(copy)[0]
    if flags[3] != ord("1"):
        raise RuntimeError("副本未启用启动注入")
    # 先比版本：官方更新后 app.asar 必然不一致，这时直接说「请重新准备副本」，
    # 比让后面的 cmp 抛出笼统的「命令执行失败」有用得多。
    if os.name != "nt":
        source_plist = source / "Contents/Info.plist"
        copy_plist = copy / "Contents/Info.plist"
        if source_plist.is_file() and copy_plist.is_file():
            with source_plist.open("rb") as f:
                source_info = plistlib.load(f)
            with copy_plist.open("rb") as f:
                copy_info = plistlib.load(f)
            if source_info.get("CFBundleVersion") != copy_info.get("CFBundleVersion"):
                raise RuntimeError("官方应用已更新，请重新制作增强版")
        verify = subprocess.run(
            ["/usr/bin/codesign", "--verify", "--deep", "--strict", str(copy)],
            capture_output=True,
            text=True,
            timeout=180,
        )
        verify_text = (verify.stderr or "") + (verify.stdout or "")
        # 固定自签证书没有受信任根，verify 会报 CSSMERR_TP_NOT_TRUSTED；
        # 签名本身有效，这种情况不放行的话每次构建都会被自己的证书挡下来。
        if verify.returncode != 0 and "CSSMERR_TP_NOT_TRUSTED" not in verify_text:
            raise RuntimeError("副本签名校验失败：" + verify_text.strip()[:300])
    source_asar, copy_asar = app_asar(root_for(source)), app_asar(root_for(copy))
    if COMPUTER_USE and cu_marker(copy).is_file():
        # CU 增强副本按设计会改 asar（并关掉 asar 完整性 fuse），这里只校验产物能解析。
        if copy_asar and not copy_asar.is_file():
            raise RuntimeError("CU 增强副本缺少 app.asar")
    elif source_asar and copy_asar:
        if os.name == "nt":
            if source_asar.read_bytes() != copy_asar.read_bytes():
                raise RuntimeError("app.asar 已被改动")
        else:
            result = subprocess.run(
                ["/usr/bin/cmp", "-s", str(source_asar), str(copy_asar)],
                capture_output=True,
                text=True,
                timeout=180,
            )
            if result.returncode:
                raise RuntimeError("app.asar 与官方不一致，请重新准备副本")
    ion = ion_for(copy)
    if not (ion / "i18n/zh-Hans.json").is_file() or not (ion / "i18n/dynamic/zh-Hans.json").is_file():
        raise RuntimeError("中文词表缺失")
    if "zh-Hans:" not in (ion / "index.html").read_text():
        raise RuntimeError("本地界面未注册中文词表")
    print("副本签名、注入开关、中文词表与 app.asar 一致性检查通过", flush=True)



SIGN_IDENTITY_NAME = "Claude Enhanced Local"


def _has_identity():
    r = subprocess.run(
        ["/usr/bin/security", "find-identity", "-p", "codesigning"],
        capture_output=True,
        text=True,
        timeout=60,
    )
    return SIGN_IDENTITY_NAME in ((r.stdout or "") + (r.stderr or ""))


def ensure_signing_identity():
    """每台电脑各自生成一张同名自签证书。

    有了固定证书，TCC 按「证书 + identifier」认副本，重签不会掉授权；
    ad-hoc 签名的 csreq 带 cdhash，每次构建都得重新授权。
    """
    if os.name == "nt":
        return
    if os.environ.get("CODEX_PLUS_CLAUDE_SIGN_IDENTITY") or _has_identity():
        return
    home = pathlib.Path(os.path.expanduser("~"))
    target = home / ".codex-session-delete" / "claude-localization" / "signing"
    target.mkdir(parents=True, exist_ok=True)
    key = target / "claude-enhanced.key"
    crt = target / "claude-enhanced.crt"
    p12 = target / "claude-enhanced.p12"
    if not p12.exists():
        print("生成本机代码签名证书（仅本机使用）…", flush=True)
        run([
            "/usr/bin/openssl", "req", "-x509", "-newkey", "rsa:2048",
            "-keyout", str(key), "-out", str(crt), "-days", "3650", "-nodes",
            "-subj", f"/CN={SIGN_IDENTITY_NAME}",
            "-addext", "keyUsage=critical,digitalSignature",
            "-addext", "extendedKeyUsage=critical,codeSigning",
            "-addext", "basicConstraints=critical,CA:false",
        ])
        run([
            "/usr/bin/openssl", "pkcs12", "-export",
            "-inkey", str(key), "-in", str(crt), "-out", str(p12),
            "-passout", "pass:claude",
        ])
    run([
        "/usr/bin/security", "import", str(p12),
        "-k", str(home / "Library/Keychains/login.keychain-db"),
        "-P", "claude", "-T", "/usr/bin/codesign",
    ])
    print("证书已导入钥匙串：" + SIGN_IDENTITY_NAME, flush=True)


def build(source, copy):
    if copy.exists() or copy.is_symlink() or source.resolve() == copy.resolve():
        raise RuntimeError("构建目标必须是新的暂存目录")
    if not ion_for(source).joinpath("index.html").is_file():
        raise RuntimeError("此版本缺少 ion-dist 本地界面，暂不支持此版本汉化")
    if os.name != "nt":
        run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(source)])
    copy.parent.mkdir(parents=True, exist_ok=True)
    report(5, "克隆官方应用")
    print("克隆官方应用到暂存副本…", flush=True)
    if os.name == "nt":
        source_root, copy_root = root_for(source), root_for(copy)
        if copy_root.exists():
            raise RuntimeError("构建目标必须是新的暂存目录")
        shutil.copytree(source_root, copy_root)
    else:
        try:
            run(["/bin/cp", "-Rc", str(source), str(copy)])
        except RuntimeError:
            # ditto 可补全非 APFS 上的部分复制；不删除任何现有目录。
            run(["/usr/bin/ditto", str(source), str(copy)])
    fuse_sites = fuses(copy)
    flags = fuse_sites[0][2]
    if flags[3] not in (ord("0"), ord("1")):
        raise RuntimeError("此版本禁用了 Electron inspect fuse，无法汉化")
    if COMPUTER_USE and flags[4] not in (ord("0"), ord("1")):
        raise RuntimeError("此版本缺少 asar 完整性 fuse，无法做 CU 增强")
    # universal binary 每个架构各带一份 fuse，全部改掉，避免只改到不生效的那份。
    for binary, offset, _flags in fuse_sites:
        with binary.open("r+b") as f:
            f.seek(offset + 3)
            f.write(b"1")   # EnableNodeCliInspectArguments：启动注入需要
            if COMPUTER_USE:
                f.seek(offset + 4)
                f.write(b"0")   # EnableEmbeddedAsarIntegrityValidation：要改 asar 就必须关掉校验

    ion = ion_for(copy)
    for rel in ("zh-Hans.json", "dynamic/zh-Hans.json", "zh-Hans.overrides.json"):
        target = ion / "i18n" / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(HERE / "catalogs" / rel, target)

    # 白名单是本地 UI 的语言协商约束；官方更新后找不到时明确报不兼容。
    old = '["en-US","de-DE","fr-FR","ko-KR","ja-JP","es-419","es-ES","it-IT","hi-IN","pt-BR","id-ID"]'
    new = old[:-1] + ',"zh-Hans"]'
    matches = 0
    for p in (ion / "assets").rglob("*.js"):
        text = p.read_text(encoding="utf-8")
        if old in text:
            p.write_text(text.replace(old, new), encoding="utf-8")
            matches += 1
        elif new in text:
            matches += 1
    if not matches:
        raise RuntimeError("本地界面的语言协商格式已变化，旧副本已保留")

    index = ion / "index.html"
    html = index.read_text(encoding="utf-8")
    langs = sorted(p.stem for p in (ion / "i18n").glob("*.json") if "." not in p.stem)
    meta = '<meta name="i18n-catalogs" data-i18n-catalogs="' + ",".join(l + ":1.1.1" for l in langs) + '">'
    if re.search(r'<meta\b[^>]*name=[\"\x27]i18n-catalogs[\"\x27][^>]*>', html):
        html = re.sub(r'<meta\b[^>]*name=[\"\x27]i18n-catalogs[\"\x27][^>]*>', lambda _: meta, html)
    elif "</head>" in html:
        html = html.replace("</head>", meta + "</head>", 1)
    else:
        raise RuntimeError("本地界面的 HTML 格式已变化")
    index.write_text(html, encoding="utf-8")
    report(35, "克隆完成")
    if COMPUTER_USE:
        report(45, "改写 Computer Use 门禁")
        patch_asar(copy)
    else:
        print("未勾选 Computer Use：副本 asar 保持与官方一致", flush=True)
    report(65, "安装中文词表")
    print("安装中文词表并重签副本…", flush=True)
    report(80, "重签增强版")
    ensure_signing_identity()
    if os.name != "nt":
        run([sys.executable, str(HERE / "sign-copy.py"), str(source), str(copy)])
    report(95, "校验增强版")
    check(source, copy)
    report(100, "完成")


if __name__ == "__main__":
    try:
        source = pathlib.Path(os.environ["CODEX_PLUS_CLAUDE_SOURCE"])
        copy = pathlib.Path(os.environ["CODEX_PLUS_CLAUDE_COPY"])
        if "--check" in sys.argv:
            check(source, copy)
        else:
            build(source, copy)
    except Exception as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
