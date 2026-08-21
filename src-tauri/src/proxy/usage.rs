//! 全局 usage 存储：Proxy 转换/透传路径写入检测到的 usage 数据，
//! ACP Client 在会话结束事件时读取并附加（部分 agent 不上报 usage）。

use std::sync::Mutex;
use std::time::Instant;

/// 全局 usage 存储，用于 Proxy 存储检测到的 usage 数据，ACP Client 可以读取
struct UsageRecord {
    session_id: String,
    model: String,
    input_tokens: i64,
    output_tokens: i64,
    cached_tokens: i64,
    timestamp: Instant,
}

fn usage_store() -> &'static Mutex<Vec<UsageRecord>> {
    static STORE: std::sync::OnceLock<Mutex<Vec<UsageRecord>>> = std::sync::OnceLock::new();
    STORE.get_or_init(|| Mutex::new(Vec::new()))
}

/// 存储 usage 数据到全局存储（使用最近的请求作为默认 session）
pub(crate) fn store_usage_for_latest(model: String, input_tokens: i64, output_tokens: i64, cached_tokens: i64) {
    let mut store = usage_store().lock().unwrap();
    // 只保留最近 100 条记录
    if store.len() >= 100 {
        store.remove(0);
    }
    // 使用空字符串作为 session_id，后续通过 last_usage 获取
    store.push(UsageRecord {
        session_id: String::new(),
        model,
        input_tokens,
        output_tokens,
        cached_tokens,
        timestamp: Instant::now(),
    });
}

/// 获取并清除最近的 usage 数据（优先按 model 匹配，回退到最近一条）
///
/// 并发场景下，多个会话可能共用同一批上游模型名（前端模型 id 与 agent 实际
/// 请求的 model 名还可能不一致，如 deepseek-mrivjyj7 vs deepseek-v4-pro），
/// 所以先按 model 精确匹配；匹配不到时取最近一条，避免单会话场景拿不到。
pub fn take_last_usage(model: &str) -> Option<(String, i64, i64, i64)> {
    let mut store = usage_store().lock().unwrap();
    if store.is_empty() {
        return None;
    }
    let idx = store.iter().rposition(|r| r.model == model)
        .unwrap_or(store.len() - 1);
    let record = store.remove(idx);
    // 清理超过 5 分钟的旧记录
    store.retain(|r| r.timestamp.elapsed() < std::time::Duration::from_secs(300));
    Some((record.model, record.input_tokens, record.output_tokens, record.cached_tokens))
}

