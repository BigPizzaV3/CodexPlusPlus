//! 订阅实现：Google 系：Gemini Code Assist、Antigravity。

use crate::gateway::subscriptions::Subscription;
use crate::gateway::subscriptions::antigravity::Antigravity;
use crate::gateway::subscriptions::gemini::Gemini;

pub static ITEMS: &[&dyn Subscription] = &[&Gemini, &Antigravity];
