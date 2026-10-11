#!/usr/bin/env python3
"""给「Claude CN.app」副本做 ad-hoc 重签名（自底向上，DR 只给外层）。

为什么不能直接 `codesign --force --deep`：
  --deep 会把 `-r`（自定义 designated requirement）也套到每个嵌套组件上，
  而这些组件的 identifier 各不相同（permission-fixer-…、com.github.Electron.framework …），
  于是它们全部「does not satisfy its designated Requirement」，父级随之报
  "nested code is modified or invalid"。

做法：
  1. 逐层（深的先签）签名散装 Mach-O（dylib / .so / .node / 可执行文件）；
  2. 再签 .framework、.app 包；
  3. 最后签外层 .app，并且只在这里写自定义 DR。
  entitlements 以官方原包同名组件的为准，去掉 team 绑定项，补上
  com.apple.security.cs.disable-library-validation（ad-hoc 无 Team ID，否则加载自带 framework 会被拦）。
"""

import os
import plistlib
import shutil
import subprocess
import sys
import tempfile

SRC_DEFAULT = "/Applications/Claude.app"
COPY_DEFAULT = os.path.expanduser("~/Library/Application Support/ClaudeCN/Claude CN.app")
DR = 'designated => (identifier "com.anthropic.claudefordesktop" and ! anchor apple generic)'
STRIP = {
    "com.apple.application-identifier",
    "com.apple.developer.team-identifier",
    "keychain-access-groups",
}
MACHO_MAGICS = {
    b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"\xce\xfa\xed\xfe", b"\xfe\xed\xfa\xce",
    b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca", b"\xca\xfe\xba\xbf", b"\xbf\xba\xfe\xca",
}


def run(args, **kw):
    return subprocess.run(args, capture_output=True, text=True, **kw)


def is_macho(path):
    try:
        if os.path.islink(path) or not os.path.isfile(path):
            return False
        with open(path, "rb") as f:
            return f.read(4) in MACHO_MAGICS
    except OSError:
        return False


def original_entitlements(src_app, rel):
    p = os.path.join(src_app, rel)
    if not os.path.exists(p):
        return {}
    out = run(["codesign", "-d", "--entitlements", ":-", "--xml", p]).stdout
    if not out.strip():
        return {}
    try:
        ent = plistlib.loads(out.encode())
    except Exception:
        return {}
    for k in STRIP:
        ent.pop(k, None)
    return ent


def sign(path, ent_path, extra=None):
    args = ["codesign", "--force", "--sign", "-", "--options", "runtime", "--timestamp=none"]
    if ent_path:
        args += ["--entitlements", ent_path]
    if extra:
        args += extra
    args.append(path)
    r = run(args)
    if r.returncode != 0:
        print(f"  ✗ {os.path.basename(path)}: {r.stderr.strip()[:300]}")
        return False
    return True


def main():
    src_app = sys.argv[1] if len(sys.argv) > 1 else SRC_DEFAULT
    copy_app = sys.argv[2] if len(sys.argv) > 2 else COPY_DEFAULT
    if not os.path.isdir(copy_app):
        sys.exit(f"副本不存在: {copy_app}")

    contents = os.path.join(copy_app, "Contents")

    loose, frameworks, apps = [], [], []
    for root, dirs, files in os.walk(contents):
        # 不能剪枝 .framework/.app：里面的 Helpers/、Versions/ 也要走到
        dirs[:] = [d for d in dirs if not os.path.islink(os.path.join(root, d))]
        for d in dirs:
            full = os.path.join(root, d)
            if d.endswith(".framework"):
                frameworks.append(full)
            elif d.endswith(".app"):
                apps.append(full)
        for f in files:
            full = os.path.join(root, f)
            if is_macho(full):
                loose.append(full)

    depth = lambda p: p.count(os.sep)
    loose.sort(key=depth, reverse=True)
    frameworks.sort(key=depth, reverse=True)
    apps.sort(key=depth, reverse=True)

    tmpdir = tempfile.mkdtemp(prefix="cn-sign-")
    cache = {}

    def ent_file(rel):
        if rel not in cache:
            ent = original_entitlements(src_app, rel)
            ent["com.apple.security.cs.disable-library-validation"] = True
            p = os.path.join(tmpdir, str(len(cache)) + ".plist")
            with open(p, "wb") as f:
                plistlib.dump(ent, f)
            cache[rel] = p
        return cache[rel]

    total = ok = 0
    for group, label in ((loose, "散装 Mach-O"), (frameworks, "framework"), (apps, "嵌套 .app")):
        print(f"--- 签名{label}：{len(group)} 个")
        for p in group:
            rel = os.path.relpath(p, copy_app)
            total += 1
            ok += 1 if sign(p, ent_file(rel)) else 0

    print("--- 签名外层 app（写入自定义 DR）")
    rel = "."
    ent = original_entitlements(src_app, rel)
    ent["com.apple.security.cs.disable-library-validation"] = True
    outer_ent = os.path.join(tmpdir, "outer.plist")
    with open(outer_ent, "wb") as f:
        plistlib.dump(ent, f)
    total += 1
    ok += 1 if sign(copy_app, outer_ent, extra=[f"-r={DR}"]) else 0

    # 签名诊断保留在临时目录，不批量删除文件。
    print(f"\n完成：{ok}/{total}")

    print("--- 校验")
    v = run(["codesign", "--verify", "--deep", "--strict", "--verbose=2", copy_app])
    if v.returncode == 0 and not v.stderr.strip():
        print("✓ codesign --verify --deep --strict 通过")
    else:
        print("verify 输出：")
        print((v.stderr or v.stdout).strip()[:2000])
    d = run(["codesign", "-d", "-r-", copy_app])
    dr = [l.strip() for l in (d.stderr or d.stdout).splitlines() if "designated" in l]
    print("DR：", dr[-1] if dr else "(未读到)")
    if ok != total or v.returncode != 0:
        raise SystemExit("签名校验失败")


if __name__ == "__main__":
    main()
