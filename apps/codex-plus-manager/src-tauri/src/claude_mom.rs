#![cfg_attr(windows, windows_subsystem = "windows")]
//! ClaudeMom 入口：与 Codex++ 共用同一个 main（含 MCP、钩子等全部子命令入口），
//! 只是在最开始标记产品变体，并把状态目录、端口换成 ClaudeMom 自己的一套。
//! 见 `flavor.rs`。

// main.rs 顶部的 windows_subsystem 是 crate 级属性，作为子模块引入时只是无效属性；本文件已经自己声明。
#[allow(unused_attributes)]
#[path = "main.rs"]
mod codex_plus_main;

fn main() {
    codex_plus_manager_lib::flavor::init_claude_mom();
    codex_plus_main::main();
}
