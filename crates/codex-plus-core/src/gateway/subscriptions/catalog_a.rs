//! 订阅实现：编辑器与 IDE 订阅：Cursor、Kiro、Zed、Command Code。

use crate::gateway::subscriptions::Subscription;

pub static ITEMS: &[&dyn Subscription] = &[
    &super::cursor::CURSOR,
    &super::kiro::KIRO,
    &super::zed::ZED,
    &super::commandcode::COMMAND_CODE,
];
