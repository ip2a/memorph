//! 共享运行时工具。
//!
//! `run_blocking` 把同步任务丢到 `spawn_blocking` 线程池执行,供 async handler 复用。

use anyhow::anyhow;

use crate::config;

/// 在阻塞线程池上运行同步任务并返回其结果。
///
/// 透传 `test_home_dir`,保证隔离环境在 `spawn_blocking` 工作线程可见
/// (跨 crate 测试也需要,故常驻而非 `#[cfg(test)]`)。
pub async fn run_blocking<T, F>(task: F) -> anyhow::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    let test_home = config::test_home_dir();

    tokio::task::spawn_blocking(move || {
        if let Some(path) = test_home {
            config::set_test_home_dir(path);
        }

        let result = task();

        config::reset_test_home_dir();

        result
    })
    .await
    .map_err(|error| anyhow!("blocking API task failed: {error}"))?
}
