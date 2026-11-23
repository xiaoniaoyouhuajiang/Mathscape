//! ACP stdio transport and `agent_events v0` mapping, independent of provider policy.

pub(crate) mod map;
pub(crate) mod transport;

#[cfg(test)]
mod map_tests;
