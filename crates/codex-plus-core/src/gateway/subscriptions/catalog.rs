//! 内置的订阅：海外的编码套餐（Cursor、Kiro、Zed、Command Code、Devin、Factory）与 Google 系
//! （Gemini Code Assist、Antigravity）。
//!
//! 国内厂商的编码套餐（Qoder、WorkBuddy、ZCode、Xiaomi MiMo 等）不内置。

use crate::gateway::subscriptions::Subscription;
use crate::gateway::subscriptions::antigravity::Antigravity;
use crate::gateway::subscriptions::gemini::Gemini;
use crate::gateway::subscriptions::{commandcode, cursor, devin, factory, kiro, zed};

pub static ITEMS: &[&dyn Subscription] = &[
    &cursor::CURSOR,
    &kiro::KIRO,
    &zed::ZED,
    &commandcode::COMMAND_CODE,
    &devin::DEVIN,
    &factory::FACTORY,
    &Gemini,
    &Antigravity,
];
