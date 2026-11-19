//! N2 desktop: Markdown block-range targets, context creation and links.
use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
use mathscape_desktop::{
    CreateInkDocumentRequest, CreateInkNoteFromContextRequest, DesktopService,
    InkLinkTargetRequest, InkMarkdownTargetRequest, LinkInkNoteRequest,
};
use mathscape_domain::{ActorRef, InkLabDocument, MediaType, RightsRecord, RightsStatus};
use mathscape_llm::MemoryCredentialResolver;
use mathscape_store_sqlite::{InkLinkRole, InkLinkTarget, InkTitleMode, InkUnlinkRequest};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

const SOURCE: &str = "# 第一章 极限\n\n**定理 3.2** 设 $f$ 在 $[a,b]$ 上连续，则 $f$ 有界。\n\n{2}------------------------------------------------\n\n例 4. 求 $\\lim_{x\\to0}\\frac{\\sin x}{x}$。\n";

struct Fixture {
    root: PathBuf,
    service: DesktopService,
    artifact: String,
}

impl Fixture {
    fn new(case: &str) -> Self {
        Self::with_source(case, SOURCE)
    }

    fn with_source(case: &str, source: &str) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-runs")
            .join(format!(
                "n2-{case}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&root).unwrap();
        let service = DesktopService::open_with_credential_store(
            &workspace,
            Arc::new(MemoryCredentialResolver::default()),
        )
        .unwrap();
        let markdown = root.join("notes.md");
        fs::write(&markdown, source).unwrap();
        let mut pool = ArtifactPool::open(&workspace).unwrap();
        let imported = pool
            .import_file(
                &markdown,
                ArtifactMetadata::new(
                    None,
                    MediaType::parse("text/markdown").unwrap(),
                    Some("notes.md".into()),
                    None,
                    RightsRecord::new(RightsStatus::NotAssessed, vec![], None).unwrap(),
                ),
                ActorRef::parse("ink-n2-test").unwrap(),
            )
            .unwrap();
        Self {
            root,
            service,
            artifact: imported.stable_id().to_string(),
        }
    }

    fn source_unit(&self, name: &str, ranges: &[(usize, usize)]) -> String {
        let connection =
            rusqlite::Connection::open(self.service.workspace_root().join("mathscape.sqlite3"))
                .unwrap();
        let unit = format!("mathscape:source-unit/{name}");
        let mut spans = Vec::new();
        for (index, &(start, end)) in ranges.iter().enumerate() {
            let span = format!("mathscape:source-span/{name}-{index}");
            let payload = serde_json::json!({"payload": {
                "markdown": {"artifact": self.artifact, "byte_range": {"start": start, "end": end}}
            }});
            seed_object(&connection, &span, "SourceSpan", payload);
            spans.push(span);
        }
        seed_object(
            &connection,
            &unit,
            "SourceSemanticUnit",
            serde_json::json!({"payload": {
                "kind": "statement", "display_title": "仅用于辨认的节点名", "source_spans": spans,
                "surface_content_artifact": self.artifact
            }}),
        );
        unit
    }

    /// (content hash, [(block id, kind)]).
    fn blocks(&self) -> (String, Vec<(String, String)>) {
        let document = self
            .service
            .markdown_document(&self.artifact, None)
            .unwrap();
        (
            document.snapshot.content_hash.to_string(),
            document
                .blocks
                .iter()
                .map(|block| (block.id.clone(), block.kind.clone()))
                .collect(),
        )
    }

    fn target(&self, first: usize, last: usize) -> InkMarkdownTargetRequest {
        let (hash, blocks) = self.blocks();
        InkMarkdownTargetRequest {
            artifact_id: self.artifact.clone(),
            content_hash: hash,
            first_block: blocks[first].0.clone(),
            last_block: blocks[last].0.clone(),
        }
    }

    fn context(
        &self,
        request: &str,
        id: &str,
        target: InkMarkdownTargetRequest,
    ) -> CreateInkNoteFromContextRequest {
        let mut document: InkLabDocument =
            serde_json::from_str(include_str!("../../../testdata/ink/p0/trace.json")).unwrap();
        document.operations.clear();
        document.document_id = id.into();
        CreateInkNoteFromContextRequest {
            request_id: request.into(),
            document,
            session_id: format!("{id}-session"),
            writer_id: "desktop-local".into(),
            device: serde_json::json!({}),
            target: InkLinkTargetRequest::Markdown(target),
            role: InkLinkRole::Attempt,
            title: None,
        }
    }
}

fn seed_object(
    connection: &rusqlite::Connection,
    id: &str,
    kind: &str,
    payload: serde_json::Value,
) {
    let rev = "sha256:0000000000000000000000000000000000000000000000000000000000000001";
    connection.execute(
        "INSERT INTO map_objects(object_id,object_shape,entity_kind,current_revision) VALUES(?1,'entity',?2,?3)",
        rusqlite::params![id, kind, rev],
    ).unwrap();
    connection.execute(
        "INSERT INTO map_object_revisions(object_id,revision_id,schema_version,lifecycle,created_at,created_by,generation_run,payload_json)
         VALUES(?1,?2,'1','active','2026-10-20T00:00:00Z','test',NULL,?3)",
        rusqlite::params![id, rev, payload.to_string()],
    ).unwrap();
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn markdown_targets_are_block_ranges_validated_against_the_content() {
    let fixture = Fixture::new("validate");
    let (_, blocks) = fixture.blocks();
    let kinds: Vec<&str> = blocks.iter().map(|(_, kind)| kind.as_str()).collect();
    assert_eq!(kinds, ["heading", "text", "page_marker", "text"]);

    let theorem = fixture
        .service
        .resolve_ink_markdown_target(&fixture.target(1, 1))
        .unwrap();
    assert_eq!(theorem.label, "定理 3.2");
    assert_eq!(theorem.block_ids.len(), 1);
    assert_eq!(theorem.block_hash.len(), 64);
    assert!(theorem.excerpt.starts_with("定理 3.2 设 $f$"));
    let heading = fixture
        .service
        .resolve_ink_markdown_target(&fixture.target(0, 0))
        .unwrap();
    assert_eq!(heading.label, "第一章 极限");
    // A range starting on a page marker takes its name from the next block.
    let range = fixture
        .service
        .resolve_ink_markdown_target(&fixture.target(2, 3))
        .unwrap();
    assert_eq!(range.label, "例 4");
    assert_eq!(range.block_ids.len(), 2);

    let code = |result: Result<_, mathscape_desktop::InkCommandError>| result.unwrap_err().code;
    assert_eq!(
        code(
            fixture
                .service
                .resolve_ink_markdown_target(&fixture.target(2, 2))
        ),
        "link_target_invalid"
    );
    assert_eq!(
        code(
            fixture
                .service
                .resolve_ink_markdown_target(&fixture.target(3, 1))
        ),
        "link_target_invalid"
    );
    let mut stale = fixture.target(1, 1);
    stale.content_hash = "sha256:0000".into();
    assert_eq!(
        code(fixture.service.resolve_ink_markdown_target(&stale)),
        "link_target_invalid"
    );
    let mut unknown = fixture.target(1, 1);
    unknown.last_block = "block:missing".into();
    assert_eq!(
        code(fixture.service.resolve_ink_markdown_target(&unknown)),
        "link_target_invalid"
    );
}

#[test]
fn context_creation_links_the_block_and_lists_it_for_the_reader() {
    let fixture = Fixture::new("context");
    let request = fixture.context("ctx-1", "note-a", fixture.target(1, 1));
    let receipt = fixture
        .service
        .create_ink_note_from_context(request.clone())
        .unwrap();
    assert_eq!(receipt.note.title, "尝试 · 定理 3.2");
    assert_eq!(receipt.note.title_mode, InkTitleMode::Auto);
    assert!(receipt.link.is_primary);
    let InkLinkTarget::Markdown(target) = &receipt.link.target else {
        panic!("markdown target expected");
    };
    assert_eq!(target.artifact_id, fixture.artifact);
    // Retrying the same request returns the same note.
    assert!(
        fixture
            .service
            .create_ink_note_from_context(request)
            .unwrap()
            .replayed
    );
    assert_eq!(fixture.service.list_ink_documents().unwrap().len(), 1);

    // A second note links to an existing block from the reader menu.
    fixture
        .service
        .create_ink_document(CreateInkDocumentRequest {
            document: {
                let mut document: InkLabDocument =
                    serde_json::from_str(include_str!("../../../testdata/ink/p0/trace.json"))
                        .unwrap();
                document.operations.clear();
                document.document_id = "note-b".into();
                document
            },
            session_id: "note-b-session".into(),
            writer_id: "desktop-local".into(),
            device: serde_json::json!({}),
            request_id: Some("create-b".into()),
            title_mode: Some(InkTitleMode::Auto),
        })
        .unwrap();
    let link = LinkInkNoteRequest {
        request_id: "link-b".into(),
        document_id: "note-b".into(),
        target: InkLinkTargetRequest::Markdown(fixture.target(1, 1)),
        role: InkLinkRole::Check,
        primary: true,
        replaces_link_id: None,
    };
    let linked = fixture.service.link_ink_note(link).unwrap();
    assert!(linked.title_changed);
    assert_eq!(linked.note.title, "验算 · 定理 3.2");

    let markdown = fixture
        .service
        .ink_markdown_links(fixture.artifact.clone())
        .unwrap();
    assert_eq!(markdown.links.len(), 2);
    assert_eq!(markdown.original_note_count, 0);
    let overlay = fixture.service.ink_link_overlay().unwrap();
    assert_eq!(overlay.counts.len(), 1);
    assert_eq!(overlay.counts[0].note_count, 2);

    let link_id = linked.link.unwrap().link_id;
    let removed = fixture
        .service
        .unlink_ink_note(InkUnlinkRequest {
            request_id: "unlink-b".into(),
            document_id: "note-b".into(),
            link_id,
        })
        .unwrap();
    assert!(removed.note.title.starts_with("手稿 · "));
    assert_eq!(
        fixture
            .service
            .ink_markdown_links(fixture.artifact.clone())
            .unwrap()
            .links
            .len(),
        1
    );
    assert_eq!(
        fixture
            .service
            .list_ink_links_to_target("bogus".into(), "x".into())
            .unwrap_err()
            .code,
        "link_target_invalid"
    );
}

#[test]
fn n3b_problem_card_renders_primary_markdown_and_stays_note_scoped() {
    let fixture = Fixture::new("problem-card");
    let (_, blocks) = fixture.blocks();
    let body_index = blocks
        .iter()
        .position(|(_, kind)| kind != "page_marker")
        .unwrap();
    let request = CreateInkNoteFromContextRequest {
        request_id: "card-create".into(),
        document: {
            let mut document: InkLabDocument =
                serde_json::from_str(include_str!("../../../testdata/ink/p0/trace.json")).unwrap();
            document.operations.clear();
            document.document_id = "card-note-a".into();
            document
        },
        session_id: "card-session-a".into(),
        writer_id: "desktop-local".into(),
        device: serde_json::json!({}),
        target: InkLinkTargetRequest::Markdown(fixture.target(body_index, body_index)),
        role: InkLinkRole::Attempt,
        title: Some("卡片手稿".into()),
    };
    fixture
        .service
        .create_ink_note_from_context(request)
        .unwrap();

    let card = fixture
        .service
        .ink_problem_card("card-note-a")
        .unwrap()
        .expect("primary card");
    assert_eq!(card.status, mathscape_desktop::ProblemCardStatus::Ok);
    assert!(!card.body.is_empty(), "expected rendered primary body");
    assert!(
        card.body.contains("极限") || card.body.contains("连续") || card.body.contains("sin"),
        "body={}",
        card.body
    );

    // Note without a primary link must not expose a card (§10 / decision 11).
    fixture
        .service
        .create_ink_document(CreateInkDocumentRequest {
            document: {
                let mut document: InkLabDocument =
                    serde_json::from_str(include_str!("../../../testdata/ink/p0/trace.json"))
                        .unwrap();
                document.operations.clear();
                document.document_id = "card-note-b".into();
                document
            },
            session_id: "card-session-b".into(),
            writer_id: "desktop-local".into(),
            device: serde_json::json!({}),
            request_id: Some("card-b".into()),
            title_mode: Some(InkTitleMode::Auto),
        })
        .unwrap();
    assert!(
        fixture
            .service
            .ink_problem_card("card-note-b")
            .unwrap()
            .is_none()
    );

    // Clearing primary clears the card for A; B still has none (§10).
    fixture
        .service
        .set_ink_note_primary_link(mathscape_store_sqlite::InkSetPrimaryRequest {
            request_id: "card-clear".into(),
            document_id: "card-note-a".into(),
            link_id: None,
        })
        .unwrap();
    assert!(
        fixture
            .service
            .ink_problem_card("card-note-a")
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .service
            .ink_problem_card("card-note-b")
            .unwrap()
            .is_none()
    );
}

#[test]
fn problem_card_source_unit_reads_its_spans_instead_of_its_title() {
    let fixture = Fixture::new("card-source-body");
    let first = SOURCE.find("**定理").unwrap();
    let first_end = SOURCE[first..].find("\n\n").unwrap() + first;
    let last = SOURCE.find("例 4.").unwrap();
    let unit = fixture.source_unit("card-unit", &[(first, first_end), (last, SOURCE.len())]);
    let mut request = fixture.context("source-card", "source-note", fixture.target(1, 1));
    request.target = InkLinkTargetRequest::SourceObject { object_id: unit };
    fixture
        .service
        .create_ink_note_from_context(request)
        .unwrap();
    let card = fixture
        .service
        .ink_problem_card("source-note")
        .unwrap()
        .unwrap();
    assert_eq!(card.status, mathscape_desktop::ProblemCardStatus::Ok);
    assert!(card.body.contains("**定理 3.2**"), "{}", card.body);
    assert!(card.body.contains("\\lim_{x\\to0}"));
    assert!(!card.body.contains("仅用于辨认的节点名"));
    assert!(!card.body.contains("第一章"));
    assert!(!card.body.contains("{2}"));
    assert_eq!(
        serde_json::to_value(&card).unwrap()["body_format"],
        "markdown"
    );
}

#[test]
fn problem_card_keeps_markdown_structure_and_math() {
    let fixture = Fixture::new("card-markdown-body");
    let request = fixture.context("md-card", "md-note", fixture.target(1, 3));
    fixture
        .service
        .create_ink_note_from_context(request)
        .unwrap();
    let card = fixture
        .service
        .ink_problem_card("md-note")
        .unwrap()
        .unwrap();
    assert!(card.body.contains("**定理 3.2**"), "{}", card.body);
    assert!(card.body.contains("\n\n"));
    assert!(card.body.contains("$\\lim_{x\\to0}\\frac{\\sin x}{x}$"));
}

#[test]
fn problem_card_missing_source_is_explicit_and_can_be_read_again() {
    let fixture = Fixture::new("card-source-retry");
    let unit = fixture.source_unit("retry-unit", &[(0, SOURCE.len() + 100)]);
    let mut request = fixture.context("missing-card", "missing-note", fixture.target(1, 1));
    request.target = InkLinkTargetRequest::SourceObject { object_id: unit };
    fixture
        .service
        .create_ink_note_from_context(request)
        .unwrap();
    let card = fixture
        .service
        .ink_problem_card("missing-note")
        .unwrap()
        .unwrap();
    assert_eq!(card.status, mathscape_desktop::ProblemCardStatus::Missing);
    assert!(card.body.is_empty());
    let connection =
        rusqlite::Connection::open(fixture.service.workspace_root().join("mathscape.sqlite3"))
            .unwrap();
    connection.execute(
        "UPDATE map_object_revisions SET payload_json=json_set(payload_json,'$.payload.markdown.byte_range.end',?1) WHERE object_id='mathscape:source-span/retry-unit-0'",
        [SOURCE.len() as i64],
    ).unwrap();
    let recovered = fixture
        .service
        .ink_problem_card("missing-note")
        .unwrap()
        .unwrap();
    assert_eq!(recovered.status, mathscape_desktop::ProblemCardStatus::Ok);
    assert!(recovered.body.contains("\\lim"));
}

#[test]
fn problem_card_bounds_long_utf8_source_without_losing_its_body() {
    let source = format!("# 长题目\n\n{}\n\n结束。", "题😀".repeat(5000));
    let fixture = Fixture::with_source("card-long-body", &source);
    let unit = fixture.source_unit("long-unit", &[(0, source.len())]);
    let mut request = fixture.context("long-card", "long-note", fixture.target(0, 0));
    request.target = InkLinkTargetRequest::SourceObject { object_id: unit };
    fixture
        .service
        .create_ink_note_from_context(request)
        .unwrap();
    let card = fixture
        .service
        .ink_problem_card("long-note")
        .unwrap()
        .unwrap();
    assert_eq!(card.status, mathscape_desktop::ProblemCardStatus::Ok);
    assert!(card.body_truncated);
    assert!(card.body.chars().count() <= 4000);
    assert!(card.body.starts_with("# 长题目"));
    assert!(!card.body.contains('\u{fffd}'));
}

#[test]
fn markdown_errata_updates_ink_links_problem_cards_and_source_detail_reviews() {
    let fixture = Fixture::new("errata-mod2-sync");
    let (_, blocks) = fixture.blocks();
    let theorem_idx = blocks
        .iter()
        .position(|(_, kind)| kind != "page_marker" && kind != "heading")
        .unwrap();
    let theorem_block_id = blocks[theorem_idx].0.clone();

    // 1. Create a Markdown-linked note on the theorem block.
    let md_req = fixture.context(
        "errata-md-note",
        "errata-md-doc",
        fixture.target(theorem_idx, theorem_idx),
    );
    fixture
        .service
        .create_ink_note_from_context(md_req)
        .unwrap();

    // 2. Create a SourceSemanticUnit covering the theorem block and link a second note to it.
    let first = SOURCE.find("**定理").unwrap();
    let first_end = SOURCE[first..].find("\n\n").unwrap() + first;
    let unit_id = fixture.source_unit("errata-unit", &[(first, first_end)]);
    let mut unit_req = fixture.context(
        "errata-unit-note",
        "errata-unit-doc",
        fixture.target(theorem_idx, theorem_idx),
    );
    unit_req.target = InkLinkTargetRequest::SourceObject {
        object_id: unit_id.clone(),
    };
    fixture
        .service
        .create_ink_note_from_context(unit_req)
        .unwrap();

    // Baseline: no unreviewed errata units yet.
    assert!(
        fixture
            .service
            .list_unreviewed_errata_units()
            .unwrap()
            .is_empty()
    );

    // 3. Apply a manual errata correcting the theorem statement.
    let saved = fixture
        .service
        .save_markdown_errata(mathscape_desktop::SaveMarkdownErrataRequest {
            artifact_id: fixture.artifact.clone(),
            base_content_hash: None,
            first_block_id: theorem_block_id.clone(),
            last_block_id: theorem_block_id,
            corrected_text: "**定理 3.2（已人工校对）** 设 $f$ 一致连续且满足 $C_n^r + C_n^{r+1} = C_{n+1}^{r+1}$。".into(),
            note: Some("核对原书修正".into()),
            verified_pdf_page: Some(1),
        })
        .unwrap();
    assert!(saved.affected_unit_ids.contains(&unit_id));

    // 4. Markdown-linked note: both list_ink_note_links chip label/excerpt and ProblemCard sync!
    let links = fixture
        .service
        .list_ink_note_links("errata-md-doc".into())
        .unwrap();
    assert_eq!(links.len(), 1);
    assert!(
        links[0].target_label.contains("已人工校对")
            || matches!(
                &links[0].target,
                mathscape_store_sqlite::InkLinkTarget::Markdown(m) if m.excerpt.contains("一致连续")
            ),
        "expected projected link label/excerpt, got {:?}",
        links[0]
    );
    let md_card = fixture
        .service
        .ink_problem_card("errata-md-doc")
        .unwrap()
        .unwrap();
    assert!(md_card.body.contains("一致连续"), "body={}", md_card.body);
    assert!(
        md_card.body.contains("C_{n+1}^{r+1}"),
        "body={}",
        md_card.body
    );

    // 5. SourceSemanticUnit-linked note: ProblemCard also reflects the errata!
    let unit_card = fixture
        .service
        .ink_problem_card("errata-unit-doc")
        .unwrap()
        .unwrap();
    assert!(
        unit_card.body.contains("一致连续"),
        "unit_card={}",
        unit_card.body
    );

    // 6. Inspector SourceNodeDetail returns effective excerpt + unreviewed errata review.
    let detail = fixture.service.source_node_detail(&unit_id).unwrap();
    assert!(
        detail.excerpt.as_deref().unwrap_or("").contains("一致连续"),
        "excerpt={:?}",
        detail.excerpt
    );
    let review = detail.errata_review.expect("expected errata_review");
    assert_eq!(review.items.len(), 1);
    assert_eq!(review.items[0].errata_id, saved.errata.errata_id);
    assert!(!review.items[0].sent_to_sandbox);

    // Unreviewed map badge index includes this unit.
    let unreviewed = fixture.service.list_unreviewed_errata_units().unwrap();
    assert_eq!(unreviewed.get(&unit_id), Some(&1));

    // 7. Marking `sent_to_sandbox` preserves badge but sets `sent_to_sandbox = true`.
    fixture
        .service
        .mark_errata_unit_reviewed(
            &unit_id,
            std::slice::from_ref(&saved.errata.errata_id),
            "sent_to_sandbox",
        )
        .unwrap();
    let after_send = fixture.service.source_node_detail(&unit_id).unwrap();
    assert!(after_send.errata_review.unwrap().items[0].sent_to_sandbox);
    assert_eq!(
        fixture
            .service
            .list_unreviewed_errata_units()
            .unwrap()
            .get(&unit_id),
        Some(&1)
    );

    // 8. Marking `acknowledged` clears both Inspector review card and map badge.
    fixture
        .service
        .mark_errata_unit_reviewed(
            &unit_id,
            std::slice::from_ref(&saved.errata.errata_id),
            "acknowledged",
        )
        .unwrap();
    let after_ack = fixture.service.source_node_detail(&unit_id).unwrap();
    assert!(after_ack.errata_review.is_none());
    assert!(
        fixture
            .service
            .list_unreviewed_errata_units()
            .unwrap()
            .is_empty()
    );

    // 9. Reverting errata restores original ProblemCard and Inspector excerpt.
    fixture
        .service
        .revert_markdown_errata(&saved.errata.errata_id)
        .unwrap();
    let reverted_card = fixture
        .service
        .ink_problem_card("errata-md-doc")
        .unwrap()
        .unwrap();
    assert!(!reverted_card.body.contains("一致连续"));
    let reverted_detail = fixture.service.source_node_detail(&unit_id).unwrap();
    assert!(
        !reverted_detail
            .excerpt
            .as_deref()
            .unwrap_or("")
            .contains("一致连续")
    );
}
