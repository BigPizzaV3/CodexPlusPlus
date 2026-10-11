#!/usr/bin/env python3
"""在宿主指定的全新暂存目录构建副本；不删除旧副本，不写官方包或用户档案。"""
import os
import pathlib
import plistlib
import re
import shutil
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
WIRE = b"dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX"


def run(args):
    result = subprocess.run(args, capture_output=True, text=True, timeout=180)
    if result.returncode:
        raise RuntimeError(result.stderr.strip()[:500] or "命令执行失败")
    return result


def fuses(app):
    binary = app / "Contents/Frameworks/Electron Framework.framework/Versions/A/Electron Framework"
    data = binary.read_bytes()
    if data.count(WIRE) != 1:
        raise RuntimeError("Electron fuse 格式已变化，无法安全汉化此版本")
    start = data.index(WIRE) + len(WIRE)
    if data[start + 1] < 4:
        raise RuntimeError("不支持的 Electron fuse 版本")
    return binary, start + 2, data[start + 2:start + 2 + data[start + 1]]


def check(source, copy):
    _, _, flags = fuses(copy)
    if flags[3] != ord("1"):
        raise RuntimeError("副本未启用启动注入")
    run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(copy)])
    run(["/usr/bin/cmp", "-s", str(source / "Contents/Resources/app.asar"),
         str(copy / "Contents/Resources/app.asar")])
    ion = copy / "Contents/Resources/ion-dist"
    if not (ion / "i18n/zh-Hans.json").is_file() or not (ion / "i18n/dynamic/zh-Hans.json").is_file():
        raise RuntimeError("中文词表缺失")
    if "zh-Hans:" not in (ion / "index.html").read_text():
        raise RuntimeError("本地界面未注册中文词表")
    with (source / "Contents/Info.plist").open("rb") as f:
        source_info = plistlib.load(f)
    with (copy / "Contents/Info.plist").open("rb") as f:
        copy_info = plistlib.load(f)
    if source_info.get("CFBundleVersion") != copy_info.get("CFBundleVersion"):
        raise RuntimeError("官方应用已更新，请重新准备副本")
    print("副本签名、注入开关、中文词表与 app.asar 一致性检查通过", flush=True)


def build(source, copy):
    if copy.exists() or copy.is_symlink() or source.resolve() == copy.resolve():
        raise RuntimeError("构建目标必须是新的暂存目录")
    if not (source / "Contents/Resources/ion-dist/index.html").is_file():
        raise RuntimeError("此版本缺少 ion-dist 本地界面，暂不支持此版本汉化")
    run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(source)])
    copy.parent.mkdir(parents=True, exist_ok=True)
    print("克隆官方应用到暂存副本…", flush=True)
    try:
        run(["/bin/cp", "-Rc", str(source), str(copy)])
    except RuntimeError:
        # ditto 可补全非 APFS 上的部分复制；不删除任何现有目录。
        run(["/usr/bin/ditto", str(source), str(copy)])
    binary, offset, flags = fuses(copy)
    if flags[3] not in (ord("0"), ord("1")):
        raise RuntimeError("此版本禁用了 Electron inspect fuse，无法汉化")
    with binary.open("r+b") as f:
        f.seek(offset + 3)
        f.write(b"1")

    ion = copy / "Contents/Resources/ion-dist"
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
    print("安装中文词表并重签副本…", flush=True)
    run([sys.executable, str(HERE / "sign-copy.py"), str(source), str(copy)])
    check(source, copy)


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
