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
        fs::write(&markdown, SOURCE).unwrap();
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
