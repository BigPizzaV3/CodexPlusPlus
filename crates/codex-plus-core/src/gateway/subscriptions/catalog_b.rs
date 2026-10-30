//! 订阅实现：国内外编码套餐：Devin、Factory、Qoder、WorkBuddy、ZCode、Xiaomi MiMo。

use crate::gateway::subscriptions::Subscription;
use crate::gateway::subscriptions::{devin, factory, mimo, qoder, workbuddy, zcode};

pub static ITEMS: &[&dyn Subscription] = &[
    &devin::DEVIN,
    &factory::FACTORY,
    &qoder::QODER,
    &qoder::QODER_CN,
    &workbuddy::WORKBUDDY,
    &workbuddy::WORKBUDDY_AI,
    &zcode::ZCODE_SUB,
    &mimo::MIMO,
];
