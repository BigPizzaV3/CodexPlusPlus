//! 各测试模块共享的辅助。

use std::sync::{Mutex, MutexGuard};

static GLOBAL_STATE: Mutex<()> = Mutex::new(());

/// 串行化会改进程级状态的用例：设置路径覆盖、诊断日志路径、环境变量等。
///
/// 合并前这些用例分散在不同测试程序里，靠进程隔离互不干扰；合并后同一进程内并行，
/// 各文件必须共用这一把锁。锁不可重入，同一用例里不要嵌套获取。
/// 中毒容忍：一个用例失败不应把后续用例一起带崩。
pub fn global_state_lock() -> MutexGuard<'static, ()> {
    GLOBAL_STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
