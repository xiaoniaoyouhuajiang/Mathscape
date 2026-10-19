//! Persistent pairing and writer leases. Only credential digests cross this
//! boundary; operation validation and durable receipts reuse the P1 writer.
use crate::{
    InkNoteReceipt, InkNoteState, InkOperationReceipt, InkRenameRequest, SqliteArtifactStore,
    SqliteStoreError, StoredInkDocument, StoredInkSession, ink, ink_note,
};
use mathscape_domain::{InkLabDocumentMetadata, InkLabOperation, InkLanOperation};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InkLanSettings {
    pub enabled: bool,
    pub port: u16,
    /// N3a: host the user last picked for the QR / default URL. `None` means
    /// fall back to the first LAN address.
    pub preferred_host: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InkRemoteLease {
    pub document_id: String,
    pub session_id: String,
    pub writer_id: String,
    pub writer_epoch: u64,
    pub expires_at: i64,
    pub device: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InkWriterView {
    pub document_id: String,
    pub head_seq: u64,
    pub writer_epoch: u64,
    pub session: Option<StoredInkSession>,
    pub remote: bool,
    pub pairing_pending: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InkMirrorDelta {
    pub metadata: InkLabDocumentMetadata,
    pub operations: Vec<InkLabOperation>,
    pub head_seq: u64,
    pub server_head: u64,
    pub writer: InkWriterView,
    pub snapshot_count: u64,
    /// Current title and lifecycle, so the desktop follows tablet renames.
    pub note: InkNoteState,
}
#[derive(Clone, Debug)]
pub struct InkPairingOffer {
    pub token_hash: String,
    pub document_id: String,
    pub expected_head: u64,
    pub expected_epoch: u64,
    pub session_id: String,
    pub writer_id: String,
    pub expires_at: i64,
    /// Required when another device holds the lease. The holder keeps writing
    /// until the new device claims the offer; claim checks the epoch again.
    pub allow_transfer: bool,
    /// Audited into `ink_writer_transfers.reason` on claim (`tablet_pairing` or `desktop_push`).
    pub claim_reason: String,
}

pub(crate) fn validate_schema(c: &Connection) -> Result<(), SqliteStoreError> {
    let name: Option<String> = c
        .query_row(
            "SELECT name FROM schema_migrations WHERE version=16",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if name.as_deref() != Some("ink_lan_p3") {
        return Err(SqliteStoreError::InvalidSchema(
            "migration 16 is absent or has an unexpected name",
        ));
    }
    let preferred: Option<String> = c
        .query_row(
            "SELECT name FROM schema_migrations WHERE version=19",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if preferred.as_deref() != Some("ink_lan_preferred_host_n3a") {
        return Err(SqliteStoreError::InvalidSchema(
            "migration 19 is absent or has an unexpected name",
        ));
    }
    c.prepare("SELECT enabled,port,preferred_host FROM ink_lan_settings LIMIT 0")?;
    c.prepare("SELECT token_hash,document_id,expected_head,expected_epoch,session_id,writer_id,expires_at,credential_hash,claim_reason FROM ink_pairings LIMIT 0")?;
    c.prepare("SELECT credential_hash,document_id,session_id,writer_id,writer_epoch,device_json,expires_at,revoked_at FROM ink_lan_devices LIMIT 0")?;
    c.prepare("SELECT request_id,request_fingerprint,document_id,from_epoch,through_seq,session_id,to_epoch,reason,created_at FROM ink_writer_transfers LIMIT 0")?;
    c.prepare("SELECT request_id,request_fingerprint,document_id,original_document_id,imported_at FROM ink_recovery_imports LIMIT 0")?;
    Ok(())
}
fn invalid(message: &str) -> SqliteStoreError {
    SqliteStoreError::InkInvariant(message.into())
}
fn digest(value: &str) -> Result<(), SqliteStoreError> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("invalid credential digest"));
    }
    Ok(())
}
fn number(value: u64) -> Result<i64, SqliteStoreError> {
    i64::try_from(value).map_err(|_| invalid("lease integer exceeds SQLite range"))
}
fn writer_view(
    c: &Connection,
    document: &str,
    now: i64,
) -> Result<InkWriterView, SqliteStoreError> {
    let (head, epoch, session): (i64, i64, Option<String>) = c.query_row(
        "SELECT head_seq,writer_epoch,active_session_id FROM ink_writer_state WHERE document_id=?1",
        [document],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let session = session
        .map(|id| ink::read_session(c, &id))
        .transpose()?
        .flatten();
    let remote = session
        .as_ref()
        .is_some_and(|s| s.writer_id != "desktop-local");
    // A transfer offer stays valid while the remote holder keeps syncing.
    let pending:bool = c.query_row("SELECT EXISTS(SELECT 1 FROM ink_pairings WHERE document_id=?1 AND credential_hash IS NULL AND expires_at>?2 AND expected_epoch=?3 AND (expected_head=?4 OR ?5))",params![document,now,epoch,head,remote],|r|r.get(0))?;
    Ok(InkWriterView {
        document_id: document.into(),
        head_seq: head.try_into().map_err(|_| invalid("negative head"))?,
        writer_epoch: epoch.try_into().map_err(|_| invalid("negative epoch"))?,
        remote,
        session,
        pairing_pending: pending,
    })
}
fn authenticate(
    c: &Connection,
    credential: &str,
    now: i64,
) -> Result<InkRemoteLease, SqliteStoreError> {
    digest(credential)?;
    let row:Option<(String,String,String,i64,String,i64,Option<i64>)> = c.query_row("SELECT document_id,session_id,writer_id,writer_epoch,device_json,expires_at,revoked_at FROM ink_lan_devices WHERE credential_hash=?1",[credential],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional()?;
    let Some((document_id, session_id, writer_id, epoch, device, expires_at, revoked)) = row else {
        return Err(invalid("authentication_failed"));
    };
    // Checked before revocation and epoch so a trashed document reports a
    // specific reason instead of a generic epoch conflict (H4).
    ink_note::require_active(c, &document_id)?;
    if revoked.is_some() {
        return Err(invalid("writer_revoked"));
    }
    if expires_at <= now {
        return Err(invalid("credential_expired"));
    }
    let view = writer_view(c, &document_id, now)?;
    if view.writer_epoch != u64::try_from(epoch).map_err(|_| invalid("negative epoch"))?
        || view.session.as_ref().is_none_or(|s| {
            s.session_id != session_id || s.writer_id != writer_id || s.status != "active"
        })
    {
        return Err(invalid("writer_epoch_conflict"));
    }
    Ok(InkRemoteLease {
        document_id,
        session_id,
        writer_id,
        writer_epoch: view.writer_epoch,
        expires_at,
        device: serde_json::from_str(&device)?,
    })
}
fn transfer(
    tx: &Transaction<'_>,
    document: &str,
    expected_epoch: u64,
    expected_head: u64,
    session: &str,
    writer: &str,
    device: &Value,
    reason: &str,
    request_id: &str,
    now: i64,
) -> Result<StoredInkSession, SqliteStoreError> {
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            document,
            expected_epoch,
            expected_head,
            session,
            writer,
            device,
            reason
        ))?)
    );
    if let Some((old, id)) = tx
        .query_row(
            "SELECT request_fingerprint,session_id FROM ink_writer_transfers WHERE request_id=?1",
            [request_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
    {
        if old != fingerprint {
            return Err(SqliteStoreError::InkRequestConflict(request_id.into()));
        }
        return ink::read_session(tx, &id)?
            .ok_or_else(|| invalid("transferred session is missing"));
    }
    if session.is_empty()
        || session.len() > 200
        || writer.is_empty()
        || writer.len() > 200
        || serde_json::to_vec(device)?.len() > 8192
    {
        return Err(invalid("invalid writer transfer"));
    }
    let view = writer_view(tx, document, now)?;
    if view.writer_epoch != expected_epoch {
        return Err(SqliteStoreError::InkWriterEpochConflict {
            document_id: document.into(),
            expected: expected_epoch,
            actual: view.writer_epoch,
        });
    }
    if view.head_seq != expected_head {
        return Err(SqliteStoreError::InkHeadConflict {
            expected: expected_head,
            actual: view.head_seq,
        });
    }
    let epoch = expected_epoch
        .checked_add(1)
        .ok_or_else(|| invalid("writer epoch overflow"))?;
    let date = format!("unix:{now}");
    tx.execute("UPDATE ink_sessions SET status='closed',last_seen_at=?2 WHERE document_id=?1 AND status='active'",params![document,date])?;
    tx.execute(
        "UPDATE ink_lan_devices SET revoked_at=?2 WHERE document_id=?1 AND revoked_at IS NULL",
        params![document, now],
    )?;
    tx.execute(
        "UPDATE ink_pairings SET expires_at=?2 WHERE document_id=?1 AND credential_hash IS NULL",
        params![document, now],
    )?;
    tx.execute("INSERT INTO ink_sessions(session_id,document_id,writer_id,writer_epoch,status,device_json,started_at,last_seen_at) VALUES(?1,?2,?3,?4,'active',?5,?6,?6)",params![session,document,writer,number(epoch)?,serde_json::to_string(device)?,date])?;
    tx.execute("UPDATE ink_writer_state SET writer_epoch=?2,active_session_id=?3,writer_id=?4,updated_at=?5 WHERE document_id=?1",params![document,number(epoch)?,session,writer,date])?;
    tx.execute("INSERT INTO ink_writer_transfers(request_id,request_fingerprint,document_id,from_epoch,through_seq,session_id,to_epoch,reason,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![request_id,fingerprint,document,number(expected_epoch)?,number(expected_head)?,session,number(epoch)?,reason,date])?;
    ink::read_session(tx, session)?.ok_or_else(|| invalid("new writer is missing"))
}

/// Writes one recovery draft and its request receipt inside `tx` without
/// committing, so a caller can batch several drafts atomically. A completed
/// request with the same fingerprint replays its session.
pub(crate) fn import_recovery_in(
    tx: &Transaction<'_>,
    original: &mathscape_domain::InkLabDocument,
    request: &str,
    now: i64,
) -> Result<StoredInkSession, SqliteStoreError> {
    original
        .validate()
        .map_err(|error| SqliteStoreError::InkValidation(error.to_string()))?;
    if request.len() != 32 || !request.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("invalid recovery request"));
    }
    let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(original)?));
    let session = format!("recovery-session-{request}");
    if let Some(old) = tx
        .query_row(
            "SELECT request_fingerprint FROM ink_recovery_imports WHERE request_id=?1",
            [request],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        if old != fingerprint {
            return Err(SqliteStoreError::InkRequestConflict(request.into()));
        }
        return ink::read_session(tx, &session)?.ok_or_else(|| invalid("recovery session missing"));
    }
    let mut document = original.clone();
    document.document_id = format!("recovery-{request}");
    while document.title.encode_utf16().count() > 180 {
        document.title.pop();
    }
    document.title.push_str(" · 恢复草稿");
    let date = format!("unix:{now}");
    document.created_at = date.clone();
    document
        .validate()
        .map_err(|error| SqliteStoreError::InkValidation(error.to_string()))?;
    let head = document.operations.len() as i64;
    // Recovery drafts carry a meaningful manual title (D4) and start active.
    tx.execute("INSERT INTO ink_documents(document_id,version,title,created_at,exported_at,pages_json,capture_segments_json,head_seq,updated_at,title_mode,title_revision,lifecycle,lifecycle_revision,trashed_at,last_used_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?4,'manual',0,'active',0,NULL,?9)",params![document.document_id,document.version,document.title,date,document.exported_at,serde_json::to_string(&document.pages)?,serde_json::to_string(&document.capture_segments)?,head,now.saturating_mul(1000)])?;
    let device = serde_json::json!({"source":"recovery_import","original_document_id":original.document_id,"original_created_at":original.created_at});
    tx.execute("INSERT INTO ink_sessions(session_id,document_id,writer_id,writer_epoch,status,device_json,started_at,last_seen_at) VALUES(?1,?2,'desktop-local',1,'active',?3,?4,?4)",params![session,document.document_id,serde_json::to_string(&device)?,date])?;
    tx.execute("INSERT INTO ink_writer_state(document_id,writer_epoch,active_session_id,writer_id,head_seq,updated_at) VALUES(?1,1,?2,'desktop-local',?3,?4)",params![document.document_id,session,head,date])?;
    for operation in &document.operations {
        let json = serde_json::to_string(operation)?;
        let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
        tx.execute("INSERT INTO ink_operations(document_id,seq,command_id,operation_json,operation_fingerprint,recorded_at) VALUES(?1,?2,?3,?4,?5,?6)",params![document.document_id,operation.seq(),operation.command_id(),json,hash,date])?;
    }
    tx.execute("INSERT INTO ink_recovery_imports(request_id,request_fingerprint,document_id,original_document_id,imported_at) VALUES(?1,?2,?3,?4,?5)",params![request,fingerprint,document.document_id,original.document_id,date])?;
    ink::read_session(tx, &session)?.ok_or_else(|| invalid("recovery session missing"))
}

impl SqliteArtifactStore {
    pub fn import_ink_recovery(
        &mut self,
        original: &mathscape_domain::InkLabDocument,
        request: &str,
        now: i64,
    ) -> Result<StoredInkSession, SqliteStoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = import_recovery_in(&tx, original, request, now)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn ink_lan_settings(&self) -> Result<InkLanSettings, SqliteStoreError> {
        Ok(self
            .connection
            .query_row(
                "SELECT enabled,port,preferred_host FROM ink_lan_settings WHERE singleton=1",
                [],
                |r| {
                    Ok(InkLanSettings {
                        enabled: r.get(0)?,
                        port: r.get(1)?,
                        preferred_host: r.get(2)?,
                    })
                },
            )
            .optional()?
            .unwrap_or(InkLanSettings {
                enabled: false,
                port: 1443,
                preferred_host: None,
            }))
    }
    pub fn save_ink_lan_settings(
        &mut self,
        settings: &InkLanSettings,
    ) -> Result<(), SqliteStoreError> {
        if let Some(host) = &settings.preferred_host {
            if host.is_empty() || host.len() > 200 || host.chars().any(char::is_control) {
                return Err(invalid("invalid preferred_host"));
            }
        }
        self.connection.execute(
            "INSERT INTO ink_lan_settings(singleton,enabled,port,preferred_host) VALUES(1,?1,?2,?3)
             ON CONFLICT(singleton) DO UPDATE SET enabled=excluded.enabled,port=excluded.port,preferred_host=excluded.preferred_host",
            params![settings.enabled, settings.port, settings.preferred_host],
        )?;
        Ok(())
    }
    pub fn ink_writer_view(
        &self,
        document: &str,
        now: i64,
    ) -> Result<InkWriterView, SqliteStoreError> {
        writer_view(&self.connection, document, now)
    }
    pub fn issue_ink_pairing(
        &mut self,
        offer: &InkPairingOffer,
        now: i64,
    ) -> Result<(), SqliteStoreError> {
        digest(&offer.token_hash)?;
        if offer.expires_at <= now || offer.expires_at > now + 300 {
            return Err(invalid("invalid pairing expiry"));
        }
        if offer.claim_reason != "tablet_pairing" && offer.claim_reason != "desktop_push" {
            return Err(invalid("invalid claim_reason"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ink_note::require_active(&tx, &offer.document_id)?;
        let view = writer_view(&tx, &offer.document_id, now)?;
        if view.remote && !offer.allow_transfer {
            return Err(invalid("takeover_required"));
        }
        if !view.remote && view.head_seq != offer.expected_head {
            return Err(SqliteStoreError::InkHeadConflict {
                expected: offer.expected_head,
                actual: view.head_seq,
            });
        }
        if view.writer_epoch != offer.expected_epoch {
            return Err(invalid("writer_epoch_conflict"));
        }
        tx.execute("UPDATE ink_pairings SET expires_at=?2 WHERE document_id=?1 AND credential_hash IS NULL",params![offer.document_id,now])?;
        tx.execute("INSERT INTO ink_pairings(token_hash,document_id,expected_head,expected_epoch,session_id,writer_id,expires_at,claim_reason) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![offer.token_hash,offer.document_id,number(offer.expected_head)?,number(offer.expected_epoch)?,offer.session_id,offer.writer_id,offer.expires_at,offer.claim_reason])?;
        tx.commit()?;
        Ok(())
    }
    pub fn cancel_ink_pairing(&mut self, document: &str, now: i64) -> Result<(), SqliteStoreError> {
        self.connection.execute("UPDATE ink_pairings SET expires_at=?2 WHERE document_id=?1 AND credential_hash IS NULL",params![document,now])?;
        Ok(())
    }
    pub fn claim_ink_pairing(
        &mut self,
        token: &str,
        credential: &str,
        device: &Value,
        now: i64,
    ) -> Result<InkRemoteLease, SqliteStoreError> {
        self.claim_ink_pairing_detailed(token, credential, device, now)
            .map(|(lease, _)| lease)
    }
    /// Also reports whether this call transferred the lease, so the caller can
    /// close the previous holder's connection exactly once.
    pub fn claim_ink_pairing_detailed(
        &mut self,
        token: &str,
        credential: &str,
        device: &Value,
        now: i64,
    ) -> Result<(InkRemoteLease, bool), SqliteStoreError> {
        digest(token)?;
        digest(credential)?;
        if token == credential || serde_json::to_vec(device)?.len() > 8192 {
            return Err(invalid("invalid device credential"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let offer:Option<(String,i64,i64,String,String,i64,Option<String>,String)>=tx.query_row("SELECT document_id,expected_head,expected_epoch,session_id,writer_id,expires_at,credential_hash,claim_reason FROM ink_pairings WHERE token_hash=?1",[token],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional()?;
        let Some((document, head, epoch, session, writer, expires, claimed, claim_reason)) = offer
        else {
            return Err(invalid("pairing_invalid"));
        };
        // Trash already expires unclaimed pairings; this keeps the rule explicit.
        ink_note::require_active(&tx, &document)?;
        if let Some(previous) = claimed {
            if previous != credential {
                return Err(invalid("pairing_already_used"));
            }
            let lease = authenticate(&tx, credential, now)?;
            tx.commit()?;
            return Ok((lease, false));
        }
        if expires <= now {
            return Err(invalid("pairing_expired"));
        }
        let epoch: u64 = epoch.try_into().map_err(|_| invalid("negative epoch"))?;
        let mut head: u64 = head.try_into().map_err(|_| invalid("negative head"))?;
        let current = writer_view(&tx, &document, now)?;
        if current.remote && current.writer_epoch == epoch {
            // The previous tablet's acknowledged operations move with the lease.
            head = current.head_seq;
        }
        let assigned = transfer(
            &tx,
            &document,
            epoch,
            head,
            &session,
            &writer,
            device,
            &claim_reason,
            &format!("pair:{token}"),
            now,
        )?;
        let expires_at = now + 30 * 24 * 60 * 60;
        tx.execute("INSERT INTO ink_lan_devices(credential_hash,document_id,session_id,writer_id,writer_epoch,device_json,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![credential,document,session,writer,number(assigned.writer_epoch)?,serde_json::to_string(device)?,expires_at])?;
        tx.execute(
            "UPDATE ink_pairings SET credential_hash=?2 WHERE token_hash=?1",
            params![token, credential],
        )?;
        let lease = authenticate(&tx, credential, now)?;
        tx.commit()?;
        Ok((lease, true))
    }
    /// Checks a device credential digest without changing any lease state.
    pub fn authenticate_ink_device(
        &self,
        credential: &str,
        now: i64,
    ) -> Result<InkRemoteLease, SqliteStoreError> {
        authenticate(&self.connection, credential, now)
    }
    pub fn ink_remote_context(
        &self,
        credential: &str,
        now: i64,
    ) -> Result<(InkRemoteLease, StoredInkDocument), SqliteStoreError> {
        let tx = self.connection.unchecked_transaction()?;
        let lease = authenticate(&tx, credential, now)?;
        let doc = ink::open_document(&tx, &lease.document_id)?
            .ok_or_else(|| invalid("document_missing"))?;
        tx.commit()?;
        Ok((lease, doc))
    }
    pub fn append_remote_ink_operation(
        &mut self,
        credential: &str,
        message: &InkLanOperation,
        now: i64,
    ) -> Result<InkOperationReceipt, SqliteStoreError> {
        message
            .validate_envelope()
            .map_err(|e| SqliteStoreError::InkValidation(e.to_string()))?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let lease = authenticate(&tx, credential, now)?;
        if lease.document_id != message.document_id
            || lease.session_id != message.session_id
            || lease.writer_epoch != message.writer_epoch
        {
            return Err(invalid("operation_lease_conflict"));
        }
        let previous: Option<String> = tx
            .query_row(
                "SELECT command_id FROM ink_operations WHERE document_id=?1 AND seq=?2",
                params![message.document_id, i64::from(message.client_seq) - 1],
                |r| r.get(0),
            )
            .optional()?;
        if previous != message.previous_operation_id {
            return Err(invalid("operation_predecessor_conflict"));
        }
        let replay: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM ink_requests WHERE request_id=?1)",
            [&message.request_id],
            |r| r.get(0),
        )?;
        if !replay {
            let (bytes,count):(i64,i64)=tx.query_row("SELECT COALESCE(sum(length(CAST(operation_json AS BLOB))),0),count(*) FROM ink_operations WHERE document_id=?1",[&message.document_id],|r|Ok((r.get(0)?,r.get(1)?)))?;
            let next = usize::try_from(bytes).map_err(|_| invalid("invalid stored byte size"))?
                + usize::try_from(count).map_err(|_| invalid("invalid operation count"))?
                + serde_json::to_vec(&message.metadata)?.len()
                + serde_json::to_vec(&message.operation)?.len()
                + 16;
            if next > mathscape_domain::INK_LAN_DOCUMENT_BYTES {
                return Err(invalid("document_byte_limit"));
            }
        }
        let receipt = ink::append_operation_in(
            &tx,
            &message.request_id,
            &message.document_id,
            &message.session_id,
            message.writer_epoch,
            u64::from(message.client_seq) - 1,
            &message.metadata,
            &message.operation,
        )?;
        tx.commit()?;
        Ok(receipt)
    }
    /// Tablet rename (§5.3): only the current lease holder may rename, and
    /// only its own document. Renaming does not need the writer epoch to be
    /// unchanged beyond what authentication already enforces.
    pub fn rename_remote_ink_note(
        &mut self,
        credential: &str,
        request_id: &str,
        title: &str,
        expected_title_revision: u64,
        now: i64,
    ) -> Result<InkNoteReceipt, SqliteStoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let lease = authenticate(&tx, credential, now)?;
        let receipt = ink_note::rename_in(
            &tx,
            &InkRenameRequest {
                request_id: request_id.into(),
                document_id: lease.document_id,
                title: title.into(),
                expected_title_revision,
            },
            now.saturating_mul(1000),
        )?;
        tx.commit()?;
        Ok(receipt)
    }
    pub fn take_ink_writer(
        &mut self,
        document: &str,
        epoch: u64,
        head: u64,
        session: &str,
        request: &str,
        now: i64,
    ) -> Result<StoredInkSession, SqliteStoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = transfer(
            &tx,
            document,
            epoch,
            head,
            session,
            "desktop-local",
            &serde_json::json!({"source":"desktop","reason":"explicit_takeover"}),
            "desktop_takeover",
            request,
            now,
        )?;
        tx.commit()?;
        Ok(result)
    }

    pub fn reserve_remote_ink_snapshot(
        &mut self,
        credential: &str,
        request_id: &str,
        head: u64,
        now: i64,
    ) -> Result<crate::InkSnapshotReservation, SqliteStoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let lease = authenticate(&tx, credential, now)?;
        let reservation = ink::reserve_snapshot_in(
            &tx,
            request_id,
            &lease.document_id,
            head,
            &format!("unix:{now}"),
        )?;
        tx.commit()?;
        Ok(reservation)
    }
    pub fn revoke_ink_lan(&mut self, now: i64) -> Result<(), SqliteStoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE ink_pairings SET expires_at=?1 WHERE credential_hash IS NULL",
            [now],
        )?;
        let mut query=tx.prepare("SELECT DISTINCT d.document_id FROM ink_lan_devices d JOIN ink_writer_state w ON d.document_id=w.document_id AND d.session_id=w.active_session_id WHERE d.revoked_at IS NULL")?;
        let docs = query
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(query);
        for doc in docs {
            tx.execute(
                "UPDATE ink_sessions SET status='closed' WHERE document_id=?1 AND status='active'",
                [&doc],
            )?;
            tx.execute("UPDATE ink_writer_state SET writer_epoch=writer_epoch+1,active_session_id=NULL,writer_id=NULL WHERE document_id=?1",[&doc])?;
        }
        tx.execute(
            "UPDATE ink_lan_devices SET revoked_at=?1 WHERE revoked_at IS NULL",
            [now],
        )?;
        tx.execute(
            "UPDATE ink_lan_settings SET enabled=0 WHERE singleton=1",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn ink_mirror_delta(
        &self,
        document: &str,
        after_seq: u64,
        now: i64,
    ) -> Result<InkMirrorDelta, SqliteStoreError> {
        let tx = self.connection.unchecked_transaction()?;
        let writer = writer_view(&tx, document, now)?;
        if after_seq > writer.head_seq {
            return Err(invalid("mirror_prefix_conflict"));
        }
        let metadata=tx.query_row("SELECT version,title,created_at,exported_at,pages_json,capture_segments_json FROM ink_documents WHERE document_id=?1",[document],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?)))?;
        let mut query=tx.prepare("SELECT operation_json FROM ink_operations WHERE document_id=?1 AND seq>?2 ORDER BY seq LIMIT 128")?;
        let jsons = query
            .query_map(params![document, number(after_seq)?], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(query);
        let operations = jsons
            .into_iter()
            .map(|json| serde_json::from_str::<InkLabOperation>(&json))
            .collect::<Result<Vec<_>, _>>()?;
        let head_seq = operations
            .last()
            .map_or(after_seq, |op| u64::from(op.seq()));
        let snapshot_count: i64 = tx.query_row(
            "SELECT count(*) FROM ink_snapshots WHERE document_id=?1",
            [document],
            |r| r.get(0),
        )?;
        let note = ink_note::read_note(&tx, document)?
            .ok_or_else(|| SqliteStoreError::InkDocumentNotFound(document.into()))?;
        let delta = InkMirrorDelta {
            metadata: InkLabDocumentMetadata {
                version: metadata.0,
                document_id: document.into(),
                title: metadata.1,
                created_at: metadata.2,
                exported_at: metadata.3,
                pages: serde_json::from_str(&metadata.4)?,
                capture_segments: serde_json::from_str(&metadata.5)?,
            },
            operations,
            head_seq,
            server_head: writer.head_seq,
            writer,
            snapshot_count: snapshot_count
                .try_into()
                .map_err(|_| invalid("negative snapshot count"))?,
            note,
        };
        tx.commit()?;
        Ok(delta)
    }
}

const PREFERRED_HOST_MIGRATION: &str =
    include_str!("../../../migrations/0019_ink_lan_preferred_host_n3a.sql");

/// v18 → v19: preferred LAN host column on ink_lan_settings.
pub(crate) fn migrate_v19(connection: &Connection) -> Result<(), SqliteStoreError> {
    let tx = connection.unchecked_transaction()?;
    tx.execute_batch(PREFERRED_HOST_MIGRATION)?;
    tx.commit()?;
    Ok(())
}
