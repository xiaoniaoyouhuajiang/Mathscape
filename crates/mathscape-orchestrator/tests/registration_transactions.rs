//! Real SQLite migrations/transactions with the production pure planners.
use mathscape_domain::*;
use mathscape_orchestrator::{map_edit::plan_map_change, registration::*};
use mathscape_store_sqlite::{
    AcceptedRegistrationPlanRecord, SqliteArtifactStore, StoredAcceptanceAuthority,
};
use rusqlite::{Connection, params};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);
fn id(s: &str) -> StableId {
    StableId::parse(format!("mathscape:test/{s}")).unwrap()
}
fn hash(n: u32) -> String {
    format!("sha256:{n:064x}")
}
fn actor() -> ActorRef {
    ActorRef::parse("test-user").unwrap()
}
fn time() -> Timestamp {
    Timestamp::parse("unix:1").unwrap()
}
struct Fixture {
    root: PathBuf,
    store: SqliteArtifactStore,
}
impl Fixture {
    fn new() -> Self {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.test-runs")
            .join(format!(
                "registration-{n}-{}-{}",
                std::process::id(),
                TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&root).unwrap();
        let store = SqliteArtifactStore::prepare(&root.join("catalog.sqlite3")).unwrap();
        let f = Self { root, store };
        f.sql().execute("INSERT INTO artifacts(artifact_id,content_hash,byte_size,current_revision) VALUES(?1,?2,3,?2)",params![id("md").as_str(),hash(1)]).unwrap();
        f
    }
    fn sql(&self) -> Connection {
        let c = Connection::open(self.root.join("catalog.sqlite3")).unwrap();
        c.pragma_update(None, "foreign_keys", true).unwrap();
        c
    }
    fn decision(&mut self, name: &str) -> SourceRegistrationCommitRequest {
        let c = self.sql();
        let run = id(&format!("run-{name}"));
        let fp = ContentHash::parse(hash(name.bytes().map(u32::from).sum::<u32>() + 100)).unwrap();
        c.execute("INSERT INTO discovery_runs(run_id,artifact_id,pipeline_version,input_fingerprint,selection_fingerprint,result_fingerprint,status,admission,profile_id,snapshot_json) VALUES(?1,?2,'test',?3,?3,?3,'completed','review_before_publish','test','{}')",params![run.as_str(),id("md").as_str(),fp.as_str()]).unwrap();
        self.store
            .save_registration_plan(&AcceptedRegistrationPlanRecord {
                decision_id: id(name),
                run_id: run,
                actor: actor(),
                decided_at: time(),
                authority: StoredAcceptanceAuthority::HumanReview,
                expected_result_fingerprint: fp.clone(),
                selection: json!({"kind":"explicit_units","unit_refs":["u1"]}),
                plan_fingerprint: fp.clone(),
                plan: json!({"version":"fixture","units":[]}),
            })
            .unwrap();
        SourceRegistrationCommitRequest {
            request_id: id(&format!("request-{name}")).to_string(),
            decision_id: id(name).to_string(),
            acceptance_plan_fingerprint: fp.to_string(),
            base_commit: self.store.map_edit_snapshot().unwrap().base_commit,
            replacement_approvals: vec![],
        }
    }
    fn register(
        &mut self,
        r: &SourceRegistrationCommitRequest,
        p: &MapPatch<SourcePayload>,
    ) -> mathscape_store_sqlite::SourceRegistrationReceipt {
        let plan =
            plan_registration_commit(&self.store.registration_snapshot().unwrap(), r, p, &actor());
        assert_eq!(
            plan.plan.impact.status,
            ImpactPlanStatus::Ready,
            "{:?}",
            plan.plan.impact.diagnostics
        );
        self.store
            .commit_bound_source_registration(
                r,
                &plan.plan.impact.plan_fingerprint,
                &actor(),
                time(),
                |s| {
                    let p = plan_registration_commit(s, r, p, &actor());
                    (p.plan, p.patch)
                },
            )
            .unwrap()
    }
    fn reversal(&self, decision: &str) -> ReverseDiscoveryRequest {
        let receipt = self
            .store
            .source_registration_receipt(&id(decision))
            .unwrap()
            .unwrap();
        ReverseDiscoveryRequest {
            request_id: id(&format!("withdraw-{decision}")).to_string(),
            decision_id: id(decision).to_string(),
            registration_commit_id: receipt.commit_id.to_string(),
            base_commit: self.store.map_edit_snapshot().unwrap().base_commit,
            reason: "整理".into(),
        }
    }
    fn withdraw(&mut self, r: &ReverseDiscoveryRequest) -> DiscoveryReversalReceipt {
        let p =
            plan_registration_reversal(&self.store.registration_snapshot().unwrap(), r, &actor());
        assert_eq!(
            p.plan.impact.status,
            ImpactPlanStatus::Ready,
            "{:?}",
            p.plan.impact.diagnostics
        );
        self.store
            .commit_registration_reversal(
                r,
                &p.plan.impact.plan_fingerprint,
                &actor(),
                time(),
                |s| {
                    let e = plan_registration_reversal(s, r, &actor());
                    (e.plan, e.patch)
                },
            )
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn candidate(title: &str) -> MapPatch<SourcePayload> {
    let span = SourceSpan::new(
        SpanGranularity::Block,
        LayoutKind::Paragraph,
        None,
        vec![],
        vec!["block:one".into()],
        Some(MarkdownLocator::new(
            id("md"),
            ByteRange::new(0, 3).unwrap(),
            None,
        )),
        None,
        Some(ContentHash::parse(hash(1)).unwrap()),
    )
    .unwrap();
    let trust = TrustRecord::new(
        ProvenanceStatus::Located,
        ParseStatus::Parsed,
        SegmentationStatus::Checked,
        TypingStatus::Unchecked,
        ProofStatus::None,
        AlignmentStatus::Unmatched,
        CurationStatus::Accepted,
        Confidence::new(0.8).unwrap(),
        vec![id("span")],
        vec![actor()],
    );
    let unit = SourceSemanticUnit::new(
        SourceEditionBinding::Unresolved,
        DiscoveryUnitKind::Statement.into(),
        Some(title.into()),
        None,
        vec![],
        None,
        vec![id("span")],
        DialectScopeBinding::Uncontextualized,
        None,
        Some(id("md")),
        vec![],
        UnitCompleteness::Complete,
        false,
        trust,
    )
    .unwrap();
    let header = |name, kind| {
        EntityHeader::new(
            id(name),
            RevisionId::parse(hash(1)).unwrap(),
            kind,
            SchemaVersion::parse("mathscape-source-1").unwrap(),
            LifecycleStatus::Active,
            time(),
            actor(),
            Some(id("run-source")),
        )
    };
    MapPatch::new(
        None,
        actor(),
        None,
        None,
        vec![
            MapOperation::CreateEntity {
                entity: EntityEnvelope::new(
                    header("span", EntityKind::SourceSpan),
                    SourcePayload::Span(span),
                )
                .unwrap(),
            },
            MapOperation::CreateEntity {
                entity: EntityEnvelope::new(
                    header("unit", EntityKind::SourceSemanticUnit),
                    SourcePayload::SemanticUnit(unit),
                )
                .unwrap(),
            },
        ],
    )
    .unwrap()
}
#[test]
fn withdraw_retry_history_and_reactivate_new_revision() {
    let mut f = Fixture::new();
    let p = candidate("定理 $a_n\\to a$");
    let a = f.decision("a");
    let original = f.register(&a, &p);
    let r = f.reversal("a");
    let receipt = f.withdraw(&r);
    assert_eq!(receipt.retired_objects.len(), 2);
    assert!(receipt.map_commit.is_some());
    let repeated = f
        .store
        .commit_registration_reversal(&r, &receipt.plan_fingerprint, &actor(), time(), |_| {
            panic!("replay must precede planner/head checks")
        })
        .unwrap();
    assert_eq!(repeated, receipt);
    let mut wrong = r.clone();
    wrong.reason = "不同请求内容".into();
    assert!(
        f.store
            .commit_registration_reversal(
                &wrong,
                &receipt.plan_fingerprint,
                &actor(),
                time(),
                |_| panic!()
            )
            .unwrap_err()
            .to_string()
            .contains("idempotency_conflict")
    );
    let history = f.store.registration_history(id("a").as_str()).unwrap();
    assert!(history.objects.iter().all(|o| o.lifecycle == "active"));
    assert!(
        f.store
            .map_objects()
            .unwrap()
            .iter()
            .all(|o| o.lifecycle == LifecycleStatus::Deprecated)
    );
    let b = f.decision("b");
    let planned =
        plan_registration_commit(&f.store.registration_snapshot().unwrap(), &b, &p, &actor());
    assert!(planned.plan.requires_confirmation);
    assert!(
        planned
            .plan
            .objects
            .iter()
            .all(|o| o.action == RegistrationObjectAction::Reactivate)
    );
    let second = f.register(&b, &p);
    assert_eq!(second.reactivated_objects.len(), 2);
    let now = f.store.map_objects().unwrap();
    assert!(
        now.iter()
            .all(|o| o.lifecycle == LifecycleStatus::Active && o.revision_id.as_str() != hash(1))
    );
    assert_eq!(original.unit_ids, second.unit_ids);
    assert_eq!(
        f.store
            .list_discovery_registrations(id("run-a").as_str())
            .unwrap()[0]
            .state,
        RegistrationState::Withdrawn
    );
    assert!(
        f.store
            .source_registration_receipt(&id("a"))
            .unwrap()
            .is_some()
    );
}
#[test]
fn shared_adoption_without_edges_protects_objects_and_zero_patch_is_a_real_withdrawal() {
    let mut f = Fixture::new();
    let p = candidate("定理");
    let a = f.decision("a");
    f.register(&a, &p);
    let b = f.decision("b");
    let second = f.register(&b, &p);
    assert_eq!(second.reused_objects.len(), 2);
    let before = f.store.map_edit_snapshot().unwrap();
    let r = f.reversal("a");
    let withdrawn = f.withdraw(&r);
    assert!(withdrawn.map_commit.is_none());
    assert_eq!(withdrawn.preserved_objects.len(), 2);
    assert_eq!(
        before.base_commit,
        f.store.map_edit_snapshot().unwrap().base_commit
    );
    let r2 = f.reversal("b");
    let w2 = f.withdraw(&r2);
    assert!(w2.map_commit.is_none());
    assert_eq!(w2.preserved_objects.len(), 2); // b only reused; never owns an inverse delete.
}
#[test]
fn sql_failure_rolls_back_revisions_pointers_and_receipt_then_retry_succeeds() {
    let mut f = Fixture::new();
    let p = candidate("定理");
    let a = f.decision("a");
    f.register(&a, &p);
    let r = f.reversal("a");
    let s = f.store.registration_snapshot().unwrap();
    let plan = plan_registration_reversal(&s, &r, &actor());
    f.sql().execute_batch("CREATE TRIGGER fail_withdrawal BEFORE INSERT ON discovery_registration_reversals BEGIN SELECT RAISE(ABORT,'injected audit failure'); END;").unwrap();
    assert!(
        f.store
            .commit_registration_reversal(
                &r,
                &plan.plan.impact.plan_fingerprint,
                &actor(),
                time(),
                |s| {
                    let e = plan_registration_reversal(s, &r, &actor());
                    (e.plan, e.patch)
                }
            )
            .is_err()
    );
    assert_eq!(
        f.store.map_edit_snapshot().unwrap().base_commit,
        s.map.base_commit
    );
    assert!(
        f.store
            .map_objects()
            .unwrap()
            .iter()
            .all(|o| o.lifecycle == LifecycleStatus::Active)
    );
    assert!(
        f.store
            .discovery_registration_reversal(&r.decision_id)
            .unwrap()
            .is_none()
    );
    f.sql()
        .execute_batch("DROP TRIGGER fail_withdrawal")
        .unwrap();
    assert_eq!(f.withdraw(&r).retired_objects.len(), 2);
}
#[test]
fn adoption_only_change_invalidates_preview_even_when_map_head_is_unchanged() {
    let mut f = Fixture::new();
    let p = candidate("定理");
    let a = f.decision("a");
    f.register(&a, &p);
    let b = f.decision("b");
    f.register(&b, &p);
    let r = f.reversal("a");
    let plan = plan_registration_reversal(&f.store.registration_snapshot().unwrap(), &r, &actor());
    let other = f.reversal("b");
    f.withdraw(&other);
    assert_eq!(
        r.base_commit,
        f.store.map_edit_snapshot().unwrap().base_commit
    );
    assert!(
        f.store
            .commit_registration_reversal(
                &r,
                &plan.plan.impact.plan_fingerprint,
                &actor(),
                time(),
                |s| {
                    let e = plan_registration_reversal(s, &r, &actor());
                    (e.plan, e.patch)
                }
            )
            .unwrap_err()
            .to_string()
            .contains("plan_conflict")
    );
    assert!(
        f.store
            .discovery_registration_reversal(&r.decision_id)
            .unwrap()
            .is_none()
    );
}
#[test]
fn content_replacement_requires_exact_approval_and_shared_replacement_stays_blocked() {
    let mut f = Fixture::new();
    let p = candidate("旧定理");
    let a = f.decision("a");
    f.register(&a, &p);
    let r = f.reversal("a");
    f.withdraw(&r);
    let mut b = f.decision("b");
    let new = candidate("新定理");
    let plan = plan_registration_commit(
        &f.store.registration_snapshot().unwrap(),
        &b,
        &new,
        &actor(),
    );
    assert_eq!(plan.plan.impact.status, ImpactPlanStatus::Blocked);
    let rev = f
        .store
        .map_objects()
        .unwrap()
        .into_iter()
        .find(|o| o.stable_id == id("unit"))
        .unwrap()
        .revision_id
        .to_string();
    b.replacement_approvals
        .push(RegistrationReplacementApproval {
            object_id: id("unit").to_string(),
            expected_revision: rev,
        });
    let registered = f.register(&b, &new);
    assert_eq!(registered.reactivated_objects.len(), 2);
    let mut c = f.decision("c");
    c.replacement_approvals
        .push(RegistrationReplacementApproval {
            object_id: id("unit").to_string(),
            expected_revision: f
                .store
                .map_objects()
                .unwrap()
                .into_iter()
                .find(|o| o.stable_id == id("unit"))
                .unwrap()
                .revision_id
                .to_string(),
        });
    let blocked =
        plan_registration_commit(&f.store.registration_snapshot().unwrap(), &c, &p, &actor());
    assert!(
        blocked
            .plan
            .impact
            .diagnostics
            .iter()
            .any(|d| d.code == "shared_replacement")
    );
}
#[test]
fn stale_map_head_and_later_manual_revision_are_protected() {
    let mut f = Fixture::new();
    let p = candidate("定理");
    let a = f.decision("a");
    f.register(&a, &p);
    let r = f.reversal("a");
    let plan = plan_registration_reversal(&f.store.registration_snapshot().unwrap(), &r, &actor());
    let s = f.store.map_edit_snapshot().unwrap();
    let unit = s
        .objects
        .iter()
        .find(|o| o.object_id == id("unit").as_str())
        .unwrap();
    let request = UserChangeRequest {
        request_id: id("manual").to_string(),
        base_commit: s.base_commit,
        reason: "人工改名".into(),
        operations: vec![UserChangeOperation::EditSourceUnit {
            object_id: unit.object_id.clone(),
            expected_revision: unit.revision.clone(),
            changes: SourceUnitChanges {
                display_title: FieldChange::Set("人工标题".into()),
                ..Default::default()
            },
        }],
    };
    let edit = plan_map_change(&f.store.map_edit_snapshot().unwrap(), &request, &actor());
    f.store
        .commit_map_change(
            &request,
            &edit.impact.plan_fingerprint,
            &actor(),
            time(),
            |s| {
                let e = plan_map_change(s, &request, &actor());
                (e.impact, e.patch)
            },
        )
        .unwrap();
    assert!(
        f.store
            .commit_registration_reversal(
                &r,
                &plan.plan.impact.plan_fingerprint,
                &actor(),
                time(),
                |_| panic!()
            )
            .unwrap_err()
            .to_string()
            .contains("base_commit_conflict")
    );
    let updated = f.reversal("a");
    let w = f.withdraw(&updated);
    assert_eq!(w.preserved_objects.len(), 2);
    assert!(w.map_commit.is_none());
}
#[test]
fn accepted_unregistered_can_be_discarded_and_writer_checks_disposition() {
    let mut f = Fixture::new();
    let p = candidate("定理");
    let a = f.decision("a");
    let plan =
        plan_registration_commit(&f.store.registration_snapshot().unwrap(), &a, &p, &actor());
    f.store
        .discard_discovery_run(
            &id("run-a"),
            &id("discard"),
            &ContentHash::parse(&a.acceptance_plan_fingerprint).unwrap(),
            &actor(),
            "不采用",
            &time(),
        )
        .unwrap();
    assert!(
        f.store
            .commit_bound_source_registration(
                &a,
                &plan.plan.impact.plan_fingerprint,
                &actor(),
                time(),
                |_| panic!("disposition check before planner")
            )
            .unwrap_err()
            .to_string()
            .contains("run_discarded")
    );
    assert!(f.store.map_objects().unwrap().is_empty());
}

#[test]
fn manual_retirement_cascades_one_endpoint_relation_and_supports_span_lifecycle() {
    let mut f = Fixture::new();
    let p = candidate("定理");
    let a = f.decision("a");
    f.register(&a, &p);
    let create = UserChangeRequest {
        request_id: id("manual-create").to_string(),
        base_commit: f.store.map_edit_snapshot().unwrap().base_commit,
        reason: "添加解释".into(),
        operations: vec![
            UserChangeOperation::CreateSourceUnit {
                object_id: id("explanation").to_string(),
                draft: SourceUnitDraft {
                    kind: DiscoveryUnitKind::Remark,
                    display_title: Some("解释".into()),
                    parent: None,
                    source_spans: vec![id("span").to_string()],
                    completeness: UnitCompleteness::Complete,
                },
            },
            UserChangeOperation::CreateSourceRelation {
                object_id: id("relation").to_string(),
                draft: SourceRelationDraft {
                    relation: DiscoveryRelationKind::Explains,
                    source: id("explanation").to_string(),
                    target: id("unit").to_string(),
                    evidence_spans: vec![id("span").to_string()],
                },
            },
        ],
    };
    let planned = plan_map_change(&f.store.map_edit_snapshot().unwrap(), &create, &actor());
    assert_eq!(
        planned.impact.status,
        ImpactPlanStatus::Ready,
        "{:?}",
        planned.impact.diagnostics
    );
    f.store
        .commit_map_change(
            &create,
            &planned.impact.plan_fingerprint,
            &actor(),
            time(),
            |s| {
                let p = plan_map_change(s, &create, &actor());
                (p.impact, p.patch)
            },
        )
        .unwrap();
    let snapshot = f.store.map_edit_snapshot().unwrap();
    let retire = UserChangeRequest {
        request_id: id("manual-retire-unit").to_string(),
        base_commit: snapshot.base_commit,
        reason: "清理定理".into(),
        operations: vec![UserChangeOperation::RetireObject {
            object_id: id("unit").to_string(),
            expected_revision: hash(1),
        }],
    };
    let planned = plan_map_change(&f.store.map_edit_snapshot().unwrap(), &retire, &actor());
    assert_eq!(planned.impact.status, ImpactPlanStatus::Ready);
    assert!(
        planned
            .impact
            .affected_objects
            .iter()
            .any(|o| o.object_id == id("relation").as_str() && o.action == "retire")
    );
    f.store
        .commit_map_change(
            &retire,
            &planned.impact.plan_fingerprint,
            &actor(),
            time(),
            |s| {
                let p = plan_map_change(s, &retire, &actor());
                (p.impact, p.patch)
            },
        )
        .unwrap();
    let snapshot = f.store.map_edit_snapshot().unwrap();
    assert!(
        snapshot
            .objects
            .iter()
            .any(|o| o.object_id == id("explanation").as_str() && o.lifecycle == "active")
    );
    let span = snapshot
        .objects
        .iter()
        .find(|o| o.object_id == id("span").as_str())
        .unwrap();
    assert!(
        mathscape_orchestrator::map_edit::get_edit_context(&snapshot, &span.object_id)
            .unwrap()
            .can_retire
    );
    let cleanup = UserChangeRequest {
        request_id: id("manual-retire-span").to_string(),
        base_commit: snapshot.base_commit.clone(),
        reason: "清理证据及唯一使用者".into(),
        operations: snapshot
            .objects
            .iter()
            .filter(|o| {
                [id("span").as_str(), id("explanation").as_str()].contains(&o.object_id.as_str())
            })
            .map(|o| UserChangeOperation::RetireObject {
                object_id: o.object_id.clone(),
                expected_revision: o.revision.clone(),
            })
            .collect(),
    };
    let planned = plan_map_change(&snapshot, &cleanup, &actor());
    assert_eq!(planned.impact.status, ImpactPlanStatus::Ready);
    f.store
        .commit_map_change(
            &cleanup,
            &planned.impact.plan_fingerprint,
            &actor(),
            time(),
            |s| {
                let p = plan_map_change(s, &cleanup, &actor());
                (p.impact, p.patch)
            },
        )
        .unwrap();
    assert!(
        f.store
            .map_objects()
            .unwrap()
            .iter()
            .all(|o| o.lifecycle == LifecycleStatus::Deprecated)
    );
}

#[test]
fn upgrade_v10_preserves_registered_revisions_endpoints_and_reads_v1_receipts() {
    let mut f = Fixture::new();
    let p = candidate("定理");
    let a = f.decision("a");
    let receipt = f.register(&a, &p);
    let mut old = serde_json::to_value(&receipt).unwrap();
    old["version"] = json!("mathscape-source-registration-receipt-1");
    old.as_object_mut().unwrap().remove("reactivated_objects");
    old.as_object_mut().unwrap().remove("revised_objects");
    let c = f.sql();
    c.execute(
        "UPDATE source_registration_commits SET receipt_json=?2 WHERE decision_id=?1",
        params![id("a").as_str(), serde_json::to_string(&old).unwrap()],
    )
    .unwrap();
    c.execute_batch("DROP TABLE discovery_run_dispositions; DROP TABLE discovery_run_disposition_events; CREATE TABLE discovery_run_dispositions(run_id TEXT PRIMARY KEY,disposition TEXT NOT NULL,reason TEXT NOT NULL,actor TEXT NOT NULL,request_id TEXT NOT NULL UNIQUE,decided_at TEXT NOT NULL); CREATE INDEX discovery_run_dispositions_by_decision ON discovery_run_dispositions(request_id); DROP TABLE source_registration_requests; DROP TABLE discovery_registration_reversals; DROP INDEX map_commit_objects_by_object; DELETE FROM schema_migrations WHERE version IN (11,12); PRAGMA user_version=10;").unwrap();
    drop(c);
    let upgraded = SqliteArtifactStore::prepare(&f.root.join("catalog.sqlite3")).unwrap();
    let old = upgraded
        .source_registration_receipt(&id("a"))
        .unwrap()
        .unwrap();
    assert!(old.reactivated_objects.is_empty());
    assert_eq!(old.created_objects, receipt.created_objects);
    assert_eq!(
        upgraded
            .registration_history(id("a").as_str())
            .unwrap()
            .objects
            .len(),
        2
    );
    let count: i64 = f
        .sql()
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    let r = f.reversal("a");
    assert_eq!(f.withdraw(&r).retired_objects.len(), 2);
}
