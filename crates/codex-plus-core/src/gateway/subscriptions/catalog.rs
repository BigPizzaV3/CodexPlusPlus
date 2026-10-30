//! 内置的订阅：Google 系的 Gemini Code Assist、Antigravity。
//!
//! 只内置直连官方公开登录的这两家；其它要冒充各家客户端调用私有接口的订阅（Cursor、Kiro、Qoder 等）
//! 违反厂商条款的风险大，不内置。

use crate::gateway::subscriptions::Subscription;
use crate::gateway::subscriptions::antigravity::Antigravity;
use crate::gateway::subscriptions::gemini::Gemini;

pub static ITEMS: &[&dyn Subscription] = &[&Gemini, &Antigravity];
