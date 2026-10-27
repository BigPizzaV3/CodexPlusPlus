//! 网关模式：一个本地网关同时服务 Codex 之外的 Agent（Claude Code、Grok）。
//!
//! 可选开启，与「直连切换」并存：没开网关的 Agent 完全按原来的方式写配置文件。
//! 开启后 Agent 的配置指向本地 helper 的 `/gw/<agent>/...`，请求在这里按绑定选上游，
//! 复用 Codex 代理已有的协议转换、官方登录与请求日志，并支持多 Key 轮换与路由组。
//!
//! - [`store`]：`gateway.json`，各 Agent 的绑定；
//! - [`route`]：绑定 → 模型 → 候选上游；
//! - [`key_pool`]：多 Key 冷却与熔断；[`accounts`]：在网关里登录订阅账号；
//! - [`activity`]：路由动态（最近一次请求怎么走的、计数），供界面实时展示；
//! - [`usage`]：按天汇总的用量统计；[`balance`]：供应商余额；[`sessions`]：各 Agent 的本地会话；
//! - [`pipeline`]：候选循环；
//! - [`anthropic_inbound`] / [`anthropic_outbound`]：Claude Code 的协议转换；
//! - [`server`]：HTTP 入口；
//! - [`apply`]：开启 / 关闭时改写 Agent 配置。

pub mod accounts;
pub mod activity;
pub mod anthropic_inbound;
pub mod anthropic_outbound;
pub mod apply;
pub mod balance;
pub mod chatgpt_backend;
pub mod drift;
pub mod key_pool;
pub mod opencode;
pub mod pipeline;
pub mod route;
pub mod server;
pub mod sessions;
pub mod store;
pub mod usage;

/// helper 收到的路径是否归网关处理。
pub fn is_gateway_path(path: &str) -> bool {
    path == "/gw" || path.starts_with("/gw/")
}

/// 有 Agent 开着网关模式时，常驻 helper 必须一直在跑。
pub fn needs_helper() -> bool {
    store::GatewayStore::default()
        .load()
        .is_ok_and(|settings| settings.any_enabled())
}
