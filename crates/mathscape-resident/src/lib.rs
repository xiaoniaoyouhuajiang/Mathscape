//! 常驻子进程的生命周期层（resident process lifecycle）。
//!
//! 这个 crate 只管「进程活着这件事」：懒启动、就绪握手（带启动超时）、
//! 状态机（Stopped / Starting / Ready / Failed / Disabled）、按退避重启、
//! 连续失败后降级、空闲回收、整组杀进程（Unix 上子进程自成进程组），以及
//! 把 spawn / ready / restart / exit / idle_reclaim 等事件交给调用方记录。
//!
//! 请求与响应怎么编帧由各实例自己实现：Loogle 是「一行查询、一行 JSON」，
//! 以后的 Codex app-server 是带 id 的 JSON-RPC。实例通过
//! [`Concurrency`] 声明自己的并发策略：`Serial` 的实例由调用方用一把锁
//! 串行化请求；`Multiplexed` 的实例可以在同一个子进程上按 id 分发多路请求
//! （由实例自己的读线程完成，生命周期层不关心）。
//!
//! 这里不引入异步运行时：子进程的 stdout 由一个后台线程按行读出，经
//! channel 交给调用方，所以单次读取可以带超时；空闲回收和退避用可注入的
//! [`Clock`]，测试里可以拨快时间而不必真的等待。

mod child;
mod clock;
mod event;
mod supervisor;

pub use child::{ProcessSpec, ReadLineError, ResidentChild};
pub use clock::{Clock, ManualClock, SystemClock};
pub use event::{EventSink, ResidentEvent, ResidentEventKind, null_sink};
pub use supervisor::{AcquireError, Concurrency, ResidentPolicy, ResidentState, Supervisor};
