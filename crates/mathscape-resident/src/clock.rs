use std::{
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

/// 时间来源。生产环境用 [`SystemClock`]；测试用 [`ManualClock`] 拨动时间，
/// 验证空闲回收和退避而不必真的等待。
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
    /// 等待一段时间（退避用）。`ManualClock` 只把时间往前拨。
    fn sleep(&self, duration: Duration);
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) {
        thread::sleep(duration);
    }
}

/// 手动拨动的时钟：`now()` = 创建时刻 + 累计偏移。
#[derive(Debug)]
pub struct ManualClock {
    origin: Instant,
    offset: Mutex<Duration>,
}

impl ManualClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
            offset: Mutex::new(Duration::ZERO),
        }
    }

    pub fn advance(&self, duration: Duration) {
        let mut offset = self
            .offset
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *offset += duration;
    }

    pub fn elapsed(&self) -> Duration {
        *self
            .offset
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        self.origin + self.elapsed()
    }

    fn sleep(&self, duration: Duration) {
        self.advance(duration);
    }
}
