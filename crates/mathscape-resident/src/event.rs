use std::sync::Arc;

use serde::Serialize;

/// 生命周期事件的种类。序列化为 snake_case，直接写进 JSONL 日志。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidentEventKind {
    /// 启动了一个子进程（首次或重启）。
    Spawn,
    /// 就绪握手完成。
    Ready,
    /// 握手失败或超时。
    StartupFailed,
    /// 一次请求失败（超时、管道断开、协议错乱），进程已被整组杀掉。
    RequestFailed,
    /// 失败后重新启动（紧跟一条 `spawn`）。
    Restart,
    /// 子进程退出（主动关闭或意外退出）。
    Exit,
    /// 空闲超时被回收。
    IdleReclaim,
    /// 实例身份变化（例如 profile 指纹、索引或可执行文件变了），旧进程被关闭。
    KeyChanged,
    /// 连续失败达到上限，本次运行期间降级（调用方改走一次性路径）。
    Fallback,
    /// 被开关关闭。
    Disabled,
    /// 被开关重新打开。
    Enabled,
    /// 调用方关闭（App 退出或 drop）。
    Shutdown,
}

/// 一条生命周期事件。`key` 是实例身份的可读摘要，`detail` 是人可读的原因。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResidentEvent {
    pub instance: String,
    pub kind: ResidentEventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    pub consecutive_failures: u32,
}

/// 事件出口。调用方决定写到哪里（JSONL 文件、测试里的 Vec……）。
pub type EventSink = Arc<dyn Fn(&ResidentEvent) + Send + Sync>;

/// 丢弃所有事件。
pub fn null_sink() -> EventSink {
    Arc::new(|_| {})
}
