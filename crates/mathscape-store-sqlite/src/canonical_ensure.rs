//! Canonical markdown ensure receipts, per-PDF leases and the global slot.
//!
//! A `running` row is both the cross-process per-PDF lock (unique partial
//! index) and the map's 「解析中」 signal. Live `running` rows across the
//! workspace are the global Marker slots. A lease whose deadline passed is
//! marked `abandoned` before a new claim, so a crashed MCP process cannot hold
//! a PDF forever. The owner renews its lease on a short heartbeat while
//! Marker runs, so a dead owner stops being 「解析中」 within one lease TTL. Nothing here reads or writes artifact bytes.

use mathscape_domain::{InclusivePageRange, StableId};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::SqliteStoreError;

/// One ensure attempt to claim. `idempotency_key` is
/// `(pdf, canonical key, normalized range, profile, policy)`.
#[derive(Clone, Debug)]
pub struct CanonicalEnsureClaim {
    pub ensure_id: String,
    pub idempotency_key: String,
    pub source_pdf_artifact_id: StableId,
    pub canonical_id: StableId,
    pub parser_profile: String,
    pub parser_policy_version: String,
    pub pages: InclusivePageRange,
    pub owner: String,
}

/// Limits checked inside the claim transaction.
#[derive(Clone, Copy, Debug)]
pub struct CanonicalEnsureLimits {
    /// Global Marker semaphore: live `running` rows in this workspace.
    pub max_running: u32,
    /// Rolling page budget over `window_ms` (running + finished attempts).
    pub window_pages: u32,
    pub window_ms: i64,
    pub lease_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanonicalEnsureClaimOutcome {
    Acquired,
    /// Another live attempt already holds this PDF.
    PdfBusy(CanonicalEnsureLease),
    /// Every global Marker slot is taken.
    SlotsBusy {
        running: u32,
    },
    /// The rolling page budget would be exceeded.
    BudgetExhausted {
        used_pages: u32,
        window_pages: u32,
    },
}

/// A live (`running`, unexpired) ensure. Map and tool signal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalEnsureLease {
    pub ensure_id: String,
    pub source_pdf_artifact_id: StableId,
    pub canonical_id: StableId,
    pub pages: InclusivePageRange,
    pub started_at_ms: i64,
    pub lease_expires_at_ms: i64,
}

/// Terminal state of an ensure receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanonicalEnsureOutcome {
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalEnsureReceipt {
    pub ensure_id: String,
    pub idempotency_key: String,
    pub source_pdf_artifact_id: StableId,
    pub canonical_id: StableId,
    pub pages: InclusivePageRange,
    pub status: String,
    pub conversion_id: Option<String>,
    pub markdown_artifact_id: Option<String>,
    pub error: Option<String>,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
}

pub(super) fn claim(
    connection: &mut Connection,
    claim: &CanonicalEnsureClaim,
    limits: CanonicalEnsureLimits,
    now_ms: i64,
) -> Result<CanonicalEnsureClaimOutcome, SqliteStoreError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    expire(&transaction, now_ms)?;
    if let Some(lease) = live_leases(&transaction, now_ms)?
        .into_iter()
        .find(|lease| lease.source_pdf_artifact_id == claim.source_pdf_artifact_id)
    {
        transaction.commit()?;
        return Ok(CanonicalEnsureClaimOutcome::PdfBusy(lease));
    }
    let running: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM canonical_markdown_ensure WHERE status = 'running'",
        [],
        |row| row.get(0),
    )?;
    let running = u32::try_from(running).unwrap_or(u32::MAX);
    if running >= limits.max_running {
        transaction.commit()?;
        return Ok(CanonicalEnsureClaimOutcome::SlotsBusy { running });
    }
    let used: i64 = transaction.query_row(
        "SELECT COALESCE(SUM(page_to - page_from + 1), 0) FROM canonical_markdown_ensure
         WHERE started_at_ms > ?1 AND status IN ('running', 'succeeded', 'failed')",
        [now_ms.saturating_sub(limits.window_ms)],
        |row| row.get(0),
    )?;
    let used_pages = u32::try_from(used).unwrap_or(u32::MAX);
    if used_pages.saturating_add(claim.pages.page_count()) > limits.window_pages {
        transaction.commit()?;
        return Ok(CanonicalEnsureClaimOutcome::BudgetExhausted {
            used_pages,
            window_pages: limits.window_pages,
        });
    }
    transaction.execute(
        "INSERT INTO canonical_markdown_ensure (
            ensure_id, idempotency_key, source_pdf_artifact_id, canonical_id,
            parser_profile, parser_policy_version, page_from, page_to, status, owner,
            started_at_ms, lease_expires_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'running', ?9, ?10, ?11)",
        params![
            claim.ensure_id,
            claim.idempotency_key,
            claim.source_pdf_artifact_id.as_str(),
            claim.canonical_id.as_str(),
            claim.parser_profile,
            claim.parser_policy_version,
            i64::from(claim.pages.from),
            i64::from(claim.pages.to),
            claim.owner,
            now_ms,
            now_ms.saturating_add(limits.lease_ms),
        ],
    )?;
    transaction.commit()?;
    Ok(CanonicalEnsureClaimOutcome::Acquired)
}

pub(super) fn finish(
    connection: &Connection,
    ensure_id: &str,
    outcome: CanonicalEnsureOutcome,
    conversion_id: Option<&str>,
    markdown_artifact_id: Option<&str>,
    error: Option<&str>,
    now_ms: i64,
) -> Result<(), SqliteStoreError> {
    let status = match outcome {
        CanonicalEnsureOutcome::Succeeded => "succeeded",
        CanonicalEnsureOutcome::Failed => "failed",
    };
    connection.execute(
        "UPDATE canonical_markdown_ensure
         SET status = ?2, conversion_id = ?3, markdown_artifact_id = ?4, error = ?5,
             finished_at_ms = ?6
         WHERE ensure_id = ?1 AND status IN ('running', 'abandoned')",
        params![
            ensure_id,
            status,
            conversion_id,
            markdown_artifact_id,
            error,
            now_ms
        ],
    )?;
    Ok(())
}

/// Heartbeat: pushes a live lease's deadline to `lease_expires_at_ms`.
/// Returns false when the row is no longer `running` (finished, abandoned or
/// expired at `now_ms`); the caller has lost its lock.
pub(super) fn renew(
    connection: &Connection,
    ensure_id: &str,
    lease_expires_at_ms: i64,
    now_ms: i64,
) -> Result<bool, SqliteStoreError> {
    let changed = connection.execute(
        "UPDATE canonical_markdown_ensure SET lease_expires_at_ms = ?2
         WHERE ensure_id = ?1 AND status = 'running' AND lease_expires_at_ms > ?3",
        params![ensure_id, lease_expires_at_ms, now_ms],
    )?;
    Ok(changed > 0)
}

fn expire(connection: &Connection, now_ms: i64) -> Result<(), SqliteStoreError> {
    connection.execute(
        "UPDATE canonical_markdown_ensure
         SET status = 'abandoned', finished_at_ms = ?1, error = 'lease expired'
         WHERE status = 'running' AND lease_expires_at_ms <= ?1",
        [now_ms],
    )?;
    Ok(())
}

pub(super) fn live_leases(
    connection: &Connection,
    now_ms: i64,
) -> Result<Vec<CanonicalEnsureLease>, SqliteStoreError> {
    let mut statement = connection.prepare(
        "SELECT ensure_id, source_pdf_artifact_id, canonical_id, page_from, page_to,
                started_at_ms, lease_expires_at_ms
         FROM canonical_markdown_ensure
         WHERE status = 'running' AND lease_expires_at_ms > ?1
         ORDER BY started_at_ms, ensure_id",
    )?;
    let rows = statement.query_map([now_ms], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, i64>(6)?,
        ))
    })?;
    let mut leases = Vec::new();
    for row in rows {
        let (ensure_id, pdf, canonical, from, to, started, expires) = row?;
        leases.push(CanonicalEnsureLease {
            ensure_id,
            source_pdf_artifact_id: StableId::parse(pdf)?,
            canonical_id: StableId::parse(canonical)?,
            pages: InclusivePageRange::new(page(from)?, page(to)?)?,
            started_at_ms: started,
            lease_expires_at_ms: expires,
        });
    }
    Ok(leases)
}

pub(super) fn receipt(
    connection: &Connection,
    ensure_id: &str,
) -> Result<Option<CanonicalEnsureReceipt>, SqliteStoreError> {
    let row = connection
        .query_row(
            "SELECT ensure_id, idempotency_key, source_pdf_artifact_id, canonical_id,
                    page_from, page_to, status, conversion_id, markdown_artifact_id, error,
                    started_at_ms, finished_at_ms
             FROM canonical_markdown_ensure WHERE ensure_id = ?1",
            [ensure_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                ))
            },
        )
        .optional()?;
    let Some((
        id,
        key,
        pdf,
        canonical,
        from,
        to,
        status,
        conversion,
        markdown,
        error,
        started,
        finished,
    )) = row
    else {
        return Ok(None);
    };
    Ok(Some(CanonicalEnsureReceipt {
        ensure_id: id,
        idempotency_key: key,
        source_pdf_artifact_id: StableId::parse(pdf)?,
        canonical_id: StableId::parse(canonical)?,
        pages: InclusivePageRange::new(page(from)?, page(to)?)?,
        status,
        conversion_id: conversion,
        markdown_artifact_id: markdown,
        error,
        started_at_ms: started,
        finished_at_ms: finished,
    }))
}

pub(super) fn validate_schema(connection: &Connection) -> Result<(), SqliteStoreError> {
    connection.prepare(
        "SELECT ensure_id, idempotency_key, source_pdf_artifact_id, canonical_id,
                parser_profile, parser_policy_version, page_from, page_to, status, owner,
                conversion_id, markdown_artifact_id, error, started_at_ms,
                lease_expires_at_ms, finished_at_ms
         FROM canonical_markdown_ensure LIMIT 0",
    )?;
    Ok(())
}

fn page(value: i64) -> Result<u32, SqliteStoreError> {
    u32::try_from(value).map_err(|_| SqliteStoreError::IntegerRange {
        field: "canonical_markdown_ensure.page",
        value: value.unsigned_abs(),
    })
}

/// Current wall clock in unix milliseconds.
pub fn unix_millis_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
