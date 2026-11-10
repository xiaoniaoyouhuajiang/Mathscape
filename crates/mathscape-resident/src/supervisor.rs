use std::{
    fmt::Debug,
    sync::Arc,
    time::{Duration, Instant},
};

use serde::Serialize;

use crate::{Clock, EventSink, ProcessSpec, ResidentChild, ResidentEvent, ResidentEventKind};

/// 常驻实例的生命周期状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidentState {
    /// 没有子进程；下一次 `acquire` 会懒启动。
    Stopped,
    /// 正在启动、等待就绪握手。
    Starting,
    /// 子进程已就绪。
    Ready,
    /// 连续失败达到上限：本次运行期间不再启动，调用方走一次性路径。
    Failed,
    /// 被开关关闭。
    Disabled,
}

/// 实例声明的并发策略。生命周期层只记录它；真正的排队或多路分发由实例
/// 的请求层实现。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Concurrency {
    /// 一次只处理一个请求（Loogle 交互模式）。调用方用一把锁排队。
    Serial,
    /// 同一进程上可并行多路请求，按请求 id 分发（以后的 Codex app-server）。
    Multiplexed,
}

/// 生命周期参数。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResidentPolicy {
    /// 从 spawn 到握手完成的上限。
    pub startup_timeout: Duration,
    /// 单次请求的上限（由实例的请求层使用）。
    pub request_timeout: Duration,
    /// 空闲多久回收；`None` 表示不回收。
    pub idle_timeout: Option<Duration>,
    /// 连续失败多少次后进入 [`ResidentState::Failed`]。
    pub max_consecutive_failures: u32,
    /// 第一次失败后的重启等待；之后每次翻倍，封顶 `backoff_max`。
    pub backoff_initial: Duration,
    pub backoff_max: Duration,
    /// 优雅关闭的等待时间。
    pub shutdown_grace: Duration,
    pub concurrency: Concurrency,
    /// 关闭前写给子进程的告别行（Loogle 读到空行就退出）。
    pub farewell: Option<String>,
}

impl Default for ResidentPolicy {
    fn default() -> Self {
        Self {
            startup_timeout: Duration::from_secs(90),
            request_timeout: Duration::from_secs(30),
            idle_timeout: Some(Duration::from_secs(15 * 60)),
            max_consecutive_failures: 3,
            backoff_initial: Duration::from_millis(500),
            backoff_max: Duration::from_secs(10),
            shutdown_grace: Duration::from_secs(2),
            concurrency: Concurrency::Serial,
            farewell: None,
        }
    }
}

/// `acquire` 拿不到就绪子进程的原因。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AcquireError {
    /// 开关关闭。
    Disabled,
    /// 已降级；`reason` 是最后一次失败的原因。
    Fallback { reason: String },
    /// 本次启动失败（还没到降级上限，下一次请求会按退避重试）。
    Startup { reason: String, stderr: String },
}

impl std::fmt::Display for AcquireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.write_str("resident mode is disabled"),
            Self::Fallback { reason } => write!(f, "resident mode fell back: {reason}"),
            Self::Startup { reason, .. } => write!(f, "resident process failed to start: {reason}"),
        }
    }
}

/// 一个常驻实例的监督者：持有至多一个子进程和它的身份 `K`。
///
/// 所有方法都要求 `&mut self`；`Serial` 实例直接把整个监督者放进
/// `Mutex`，锁就是请求队列。
pub struct Supervisor<K> {
    name: String,
    policy: ResidentPolicy,
    clock: Arc<dyn Clock>,
    sink: EventSink,
    state: ResidentState,
    key: Option<K>,
    child: Option<ResidentChild>,
    failures: u32,
    retry_at: Option<Instant>,
    last_used: Instant,
    restart_pending: bool,
    fallback_reason: Option<String>,
}

impl<K> std::fmt::Debug for Supervisor<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Supervisor")
            .field("name", &self.name)
            .field("state", &self.state)
            .field("failures", &self.failures)
            .finish_non_exhaustive()
    }
}

impl<K: Clone + Eq + Debug> Supervisor<K> {
    pub fn new(
        name: impl Into<String>,
        policy: ResidentPolicy,
        clock: Arc<dyn Clock>,
        sink: EventSink,
    ) -> Self {
        let now = clock.now();
        Self {
            name: name.into(),
            policy,
            clock,
            sink,
            state: ResidentState::Stopped,
            key: None,
            child: None,
            failures: 0,
            retry_at: None,
            last_used: now,
            restart_pending: false,
            fallback_reason: None,
        }
    }

    pub const fn state(&self) -> ResidentState {
        self.state
    }

    pub const fn policy(&self) -> &ResidentPolicy {
        &self.policy
    }

    pub const fn consecutive_failures(&self) -> u32 {
        self.failures
    }

    pub fn key(&self) -> Option<&K> {
        self.key.as_ref()
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(ResidentChild::pid)
    }

    pub fn fallback_reason(&self) -> Option<&str> {
        self.fallback_reason.as_deref()
    }

    /// 打开或关闭常驻模式。关闭会立即停掉子进程；重新打开会清掉失败计数
    /// （也就是说，用户手动关再开可以从降级中恢复）。
    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled {
            if self.state == ResidentState::Disabled {
                self.state = ResidentState::Stopped;
                self.failures = 0;
                self.retry_at = None;
                self.restart_pending = false;
                self.fallback_reason = None;
                self.emit(ResidentEventKind::Enabled, None, None, None);
            }
        } else if self.state != ResidentState::Disabled {
            self.stop_child("disabled by setting");
            self.state = ResidentState::Disabled;
            self.emit(ResidentEventKind::Disabled, None, None, None);
        }
    }

    /// 拿到一个属于 `key` 的就绪子进程；需要时懒启动。
    ///
    /// - `key` 与当前进程不同：先关掉旧进程（`key_changed`），失败计数清零；
    /// - 当前进程已意外退出：记一次失败，然后重启；
    /// - 处于退避期：用 [`Clock::sleep`] 等到可重试时刻；
    /// - `spec` 构造启动参数，`handshake` 在 `startup_timeout` 内完成就绪握手。
    pub fn acquire(
        &mut self,
        key: &K,
        spec: impl FnOnce() -> Result<ProcessSpec, String>,
        handshake: impl FnOnce(&mut ResidentChild, Duration) -> Result<(), String>,
    ) -> Result<&mut ResidentChild, AcquireError> {
        match self.state {
            ResidentState::Disabled => return Err(AcquireError::Disabled),
            ResidentState::Failed => {
                return Err(AcquireError::Fallback {
                    reason: self.fallback_reason.clone().unwrap_or_default(),
                });
            }
            _ => {}
        }
        if self.key.as_ref() != Some(key) {
            if self.key.is_some() {
                let detail = format!("{:?} -> {key:?}", self.key);
                let pid = self.pid();
                self.emit(ResidentEventKind::KeyChanged, pid, Some(detail), None);
                self.stop_child("key changed");
            }
            self.key = Some(key.clone());
            self.failures = 0;
            self.retry_at = None;
            self.restart_pending = false;
        }
        if let Some(child) = self.child.as_mut()
            && let Some(status) = child.exit_status()
        {
            let pid = child.pid();
            let stderr = child.stderr_tail();
            self.child = None;
            self.emit(
                ResidentEventKind::Exit,
                Some(pid),
                Some(format!("exited unexpectedly: {status}; {stderr}")),
                None,
            );
            if self.fail(format!("process exited unexpectedly: {status}")) {
                return Err(AcquireError::Fallback {
                    reason: self.fallback_reason.clone().unwrap_or_default(),
                });
            }
        }
        if self.child.is_some() && self.state == ResidentState::Ready {
            self.last_used = self.clock.now();
            return Ok(self.child.as_mut().expect("child checked above"));
        }

        if let Some(retry_at) = self.retry_at.take() {
            let now = self.clock.now();
            if retry_at > now {
                self.clock.sleep(retry_at - now);
            }
        }
        if self.restart_pending {
            self.restart_pending = false;
            self.emit(
                ResidentEventKind::Restart,
                None,
                Some(format!("after {} consecutive failure(s)", self.failures)),
                None,
            );
        }
        self.state = ResidentState::Starting;
        let spec = match spec() {
            Ok(spec) => spec,
            Err(reason) => return Err(self.startup_failed(None, reason, String::new())),
        };
        let started = Instant::now();
        let mut child = match ResidentChild::spawn(&spec) {
            Ok(child) => child,
            Err(error) => {
                return Err(self.startup_failed(
                    None,
                    format!("failed to spawn {}: {error}", spec.program.display()),
                    String::new(),
                ));
            }
        };
        let pid = child.pid();
        self.emit(
            ResidentEventKind::Spawn,
            Some(pid),
            Some(spec.program.display().to_string()),
            None,
        );
        if let Err(reason) = handshake(&mut child, self.policy.startup_timeout) {
            let stderr = child.stderr_tail();
            child.kill();
            return Err(self.startup_failed(Some(pid), reason, stderr));
        }
        let elapsed = started.elapsed();
        self.state = ResidentState::Ready;
        self.last_used = self.clock.now();
        self.emit(
            ResidentEventKind::Ready,
            Some(pid),
            None,
            Some(elapsed.as_millis() as u64),
        );
        Ok(self.child.insert(child))
    }

    /// 一次请求成功：失败计数清零，刷新空闲计时。
    pub fn report_success(&mut self) {
        self.failures = 0;
        self.retry_at = None;
        self.last_used = self.clock.now();
    }

    /// 一次请求失败（超时、管道断开、协议错乱）：整组杀掉子进程，记一次
    /// 失败，下次请求按退避重启。返回 true 表示已经降级。
    pub fn report_failure(&mut self, reason: impl Into<String>) -> bool {
        let reason = reason.into();
        let pid = self.pid();
        self.emit(
            ResidentEventKind::RequestFailed,
            pid,
            Some(reason.clone()),
            None,
        );
        if let Some(mut child) = self.child.take() {
            let status = child.kill();
            self.emit(
                ResidentEventKind::Exit,
                pid,
                Some(format!("killed after failure: {status:?}")),
                None,
            );
        }
        self.fail(reason)
    }

    /// 空闲超过 `idle_timeout` 就关闭子进程。返回 true 表示回收了。
    pub fn reap_idle(&mut self) -> bool {
        let Some(idle) = self.policy.idle_timeout else {
            return false;
        };
        let Some(child) = self.child.as_mut() else {
            return false;
        };
        if let Some(status) = child.exit_status() {
            // 空闲期间自己退出：不算请求失败，下次请求重新启动。
            let pid = child.pid();
            self.child = None;
            self.state = ResidentState::Stopped;
            self.emit(
                ResidentEventKind::Exit,
                Some(pid),
                Some(format!("exited while idle: {status}")),
                None,
            );
            return false;
        }
        let idle_for = self.clock.now().saturating_duration_since(self.last_used);
        if self.state != ResidentState::Ready || idle_for < idle {
            return false;
        }
        let pid = self.pid();
        self.emit(
            ResidentEventKind::IdleReclaim,
            pid,
            Some(format!("idle for {}s", idle_for.as_secs())),
            None,
        );
        self.stop_child("idle reclaim");
        true
    }

    /// 关闭子进程（App 退出或调用方 drop）。状态回到 Stopped，但
    /// Disabled / Failed 保持不变。
    pub fn shutdown(&mut self) {
        if self.child.is_some() {
            let pid = self.pid();
            self.emit(ResidentEventKind::Shutdown, pid, None, None);
            self.stop_child("shutdown");
        }
    }

    fn stop_child(&mut self, why: &str) {
        if let Some(mut child) = self.child.take() {
            let pid = child.pid();
            let status =
                child.shutdown(self.policy.farewell.as_deref(), self.policy.shutdown_grace);
            self.emit(
                ResidentEventKind::Exit,
                Some(pid),
                Some(format!("{why}: {status:?}")),
                None,
            );
        }
        if matches!(self.state, ResidentState::Ready | ResidentState::Starting) {
            self.state = ResidentState::Stopped;
        }
    }

    fn startup_failed(&mut self, pid: Option<u32>, reason: String, stderr: String) -> AcquireError {
        self.emit(
            ResidentEventKind::StartupFailed,
            pid,
            Some(if stderr.is_empty() {
                reason.clone()
            } else {
                format!("{reason}; stderr: {stderr}")
            }),
            None,
        );
        if self.fail(reason.clone()) {
            AcquireError::Fallback {
                reason: self.fallback_reason.clone().unwrap_or(reason),
            }
        } else {
            AcquireError::Startup { reason, stderr }
        }
    }

    /// 记一次失败；返回 true 表示进入了 Failed。
    fn fail(&mut self, reason: String) -> bool {
        self.failures = self.failures.saturating_add(1);
        self.restart_pending = true;
        if self.failures >= self.policy.max_consecutive_failures.max(1) {
            self.state = ResidentState::Failed;
            self.retry_at = None;
            self.fallback_reason = Some(reason.clone());
            self.emit(ResidentEventKind::Fallback, None, Some(reason), None);
            return true;
        }
        self.state = ResidentState::Stopped;
        let exponent = self.failures.saturating_sub(1).min(16);
        let backoff = self
            .policy
            .backoff_initial
            .saturating_mul(1_u32 << exponent)
            .min(self.policy.backoff_max);
        self.retry_at = Some(self.clock.now() + backoff);
        false
    }

    fn emit(
        &self,
        kind: ResidentEventKind,
        pid: Option<u32>,
        detail: Option<String>,
        elapsed_ms: Option<u64>,
    ) {
        (self.sink)(&ResidentEvent {
            instance: self.name.clone(),
            kind,
            pid,
            key: self.key.as_ref().map(|key| format!("{key:?}")),
            detail,
            elapsed_ms,
            consecutive_failures: self.failures,
        });
    }
}

impl<K> Drop for Supervisor<K> {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.shutdown(self.policy.farewell.as_deref(), self.policy.shutdown_grace);
        }
    }
}
