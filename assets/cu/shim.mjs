// Codex++ Computer Use 直通 shim。
//
// 由 `/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node` 执行，
// 再把 stdio 原样交给原生 CU server（`SkyComputerUseClient mcp`）。
//
// 为什么需要这一层：原生 server 对「谁能当自己的直接父进程」做代码签名校验，
// 非 OpenAI 签名（TeamID 2DC432GLL2）的父进程会被拒为
// `-10000 Sender process is not authenticated`。这个 node 二进制本身就是 OpenAI 签的，
// 让它直接 spawn 即可通过；**中间不能再插 shell 或其它进程**，否则校验照样失败。
//
// 除了拉子进程，本文件不改任何字节：MCP 帧由 Rust 侧直通。
import { spawn } from "node:child_process";

const client = process.env.CODEX_PLUS_CU_CLIENT;
if (!client) {
  process.stderr.write("[codex-plus-cu] 缺少 CODEX_PLUS_CU_CLIENT\n");
  process.exit(2);
}

// stdio: "inherit" 直接把父进程的 0/1/2 交给子进程，不经过任何缓冲或转码。
const child = spawn(client, ["mcp"], { stdio: "inherit" });

child.on("error", (error) => {
  process.stderr.write(`[codex-plus-cu] 无法启动 ${client}：${error.message}\n`);
  process.exit(3);
});

child.on("exit", (code, signal) => {
  process.exit(code ?? (signal ? 1 : 0));
});

// 父进程被终止时要连带子进程，否则会留下占着 CU service 的孤儿。
for (const signal of ["SIGTERM", "SIGINT", "SIGHUP"]) {
  process.on(signal, () => {
    if (!child.killed) child.kill(signal);
  });
}
