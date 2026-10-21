//! N3d search: providers, grouping, filters and the extension point.
use mathscape_query::search::{
    SearchCandidate, SearchKind, SearchProvider, SearchQuery, SearchRegistry, SearchTarget, search,
};
use mathscape_store_sqlite::{
    SearchArtifactRow, SearchCorpus, SearchNoteRow, SearchSnapshotRow, SearchSourceUnitRow,
};

fn corpus() -> SearchCorpus {
    SearchCorpus {
        notes: vec![
            SearchNoteRow {
                document_id: "note-1".into(),
                title: "一致连续 练习".into(),
                trashed: false,
                last_used_at: 10,
            },
            SearchNoteRow {
                document_id: "note-2".into(),
                title: "一致连续 旧稿".into(),
                trashed: true,
                last_used_at: 20,
            },
        ],
        snapshots: vec![SearchSnapshotRow {
            snapshot_id: "snap-1".into(),
            artifact_id: "art-snap".into(),
            document_id: "note-1".into(),
            through_seq: 7,
            label: "整理稿".into(),
            created_at: "unix:100".into(),
            note_title: "一致连续 练习".into(),
            note_trashed: false,
        }],
        artifacts: vec![
            SearchArtifactRow {
                artifact_id: "art-pdf".into(),
                media_type: "application/pdf".into(),
                original_filename: Some("数学分析.pdf".into()),
                lifecycle: "active".into(),
                origin_filenames: vec![],
            },
            SearchArtifactRow {
                artifact_id: "art-md".into(),
                media_type: "text/markdown".into(),
                original_filename: Some(format!("{}.md", "a".repeat(64))),
                lifecycle: "active".into(),
                origin_filenames: vec!["数学分析.pdf".into()],
            },
            SearchArtifactRow {
                artifact_id: "art-img".into(),
                media_type: "image/png".into(),
                original_filename: Some("数学分析图.png".into()),
                lifecycle: "active".into(),
                origin_filenames: vec!["数学分析.pdf".into()],
            },
            SearchArtifactRow {
                artifact_id: "art-old".into(),
                media_type: "application/pdf".into(),
                original_filename: Some("数学分析旧版.pdf".into()),
                lifecycle: "superseded".into(),
                origin_filenames: vec![],
            },
        ],
        source_units: vec![
            SearchSourceUnitRow {
                object_id: "unit-1".into(),
                lifecycle: "active".into(),
                unit_kind: Some("definition".into()),
                display_title: Some("一致连续".into()),
                source_title: Some("$\\epsilon$-$\\delta$ 定义".into()),
                source_labels: vec!["定义 2.1".into()],
            },
            SearchSourceUnitRow {
                object_id: "unit-2".into(),
                lifecycle: "rejected".into(),
                unit_kind: Some("statement".into()),
                display_title: Some("一致连续 被拒".into()),
                source_title: None,
                source_labels: vec![],
            },
        ],
    }
}

fn run(text: &str, include_trashed: bool) -> mathscape_query::search::SearchResponse {
    search(
        &SearchRegistry::with_defaults(),
        &corpus(),
        &SearchQuery {
            text: text.into(),
            include_trashed,
            ..SearchQuery::default()
        },
    )
}

fn ids(response: &mathscape_query::search::SearchResponse, kind: SearchKind) -> Vec<String> {
    response
        .groups
        .iter()
        .find(|group| group.kind == kind)
        .map(|group| group.hits.iter().map(|hit| hit.id.clone()).collect())
        .unwrap_or_default()
}

#[test]
fn groups_by_type_with_best_group_first_and_chinese_labels() {
    let response = run("一致连续", false);
    let kinds: Vec<_> = response.groups.iter().map(|group| group.kind).collect();
    // The exact source-unit title beats the note prefix match.
    assert_eq!(kinds, vec![SearchKind::SourceUnit, SearchKind::Note]);
    assert_eq!(response.groups[0].label, "来源单元");
    assert_eq!(response.groups[1].label, "手稿");
    assert_eq!(
        ids(&response, SearchKind::SourceUnit),
        vec!["unit-1"],
        "retired units hidden"
    );
    assert_eq!(
        ids(&response, SearchKind::Note),
        vec!["note-1"],
        "trash hidden by default"
    );
    assert_eq!(response.total, 2);
    let unit = &response.groups[0].hits[0];
    assert_eq!(
        unit.target,
        SearchTarget::SourceObject {
            object_id: "unit-1".into()
        }
    );
    assert_eq!(unit.subtitle.as_deref(), Some("定义 · 定义 2.1"));
}

#[test]
fn include_trashed_adds_trashed_notes_ranked_below_equal_active_ones() {
    let response = run("一致连续", true);
    assert_eq!(ids(&response, SearchKind::Note), vec!["note-1", "note-2"]);
    let notes = response
        .groups
        .iter()
        .find(|group| group.kind == SearchKind::Note)
        .unwrap();
    assert!(notes.hits[1].trashed);
    assert_eq!(notes.hits[1].subtitle.as_deref(), Some("手稿 · 回收站"));
}

#[test]
fn source_units_match_source_labels_and_latex_titles_without_bodies() {
    let by_label = run("定义2.1", false);
    assert_eq!(ids(&by_label, SearchKind::SourceUnit), vec!["unit-1"]);
    assert_eq!(
        by_label.groups[0].hits[0].matched_alias.as_deref(),
        Some("定义 2.1")
    );
    let by_latex = run("\\epsilon \\delta", false);
    assert_eq!(ids(&by_latex, SearchKind::SourceUnit), vec!["unit-1"]);
}

#[test]
fn materials_use_readable_names_and_skip_images_and_retired() {
    let response = run("数学分析", false);
    let materials = response
        .groups
        .iter()
        .find(|group| group.kind == SearchKind::Material)
        .unwrap();
    let titles: Vec<_> = materials
        .hits
        .iter()
        .map(|hit| hit.title.as_str())
        .collect();
    assert_eq!(titles, vec!["数学分析", "数学分析 · 解析正文"]);
    assert_eq!(
        materials.hits[1].subtitle.as_deref(),
        Some("解析正文 · 来自 数学分析")
    );
    assert_eq!(
        materials.hits[0].target,
        SearchTarget::Artifact {
            artifact_id: "art-pdf".into()
        }
    );
    assert_eq!(materials.hits[0].highlights, vec![[0, 4]]);
}

#[test]
fn kinds_filter_limits_and_empty_queries() {
    let response = search(
        &SearchRegistry::with_defaults(),
        &corpus(),
        &SearchQuery {
            text: "一致".into(),
            kinds: vec![SearchKind::Note],
            include_trashed: true,
            limit_per_group: Some(1),
        },
    );
    assert_eq!(response.groups.len(), 1);
    assert_eq!(response.groups[0].total, 2);
    assert_eq!(response.groups[0].hits.len(), 1);
    // Recency breaks the score tie only among equals: active note-1 has the
    // higher score (trash penalty), so it stays first.
    assert_eq!(response.groups[0].hits[0].id, "note-1");
    assert_eq!(run("", false).groups.len(), 0);
    assert_eq!(run(" 　，", false).total, 0);
    let snapshot = run("整理稿", false);
    assert_eq!(
        snapshot.groups[0].hits[0].target,
        SearchTarget::InkSnapshot {
            snapshot_id: "snap-1".into(),
            artifact_id: "art-snap".into(),
            document_id: "note-1".into()
        }
    );
    assert_eq!(
        snapshot.groups[0].hits[0].subtitle.as_deref(),
        Some("一致连续 练习 · 第 7 步")
    );
}

/// Extension point: a semantic-layer provider plugs in without touching the
/// query / response contract or the frontend (generic map-node target).
struct FakeSemanticProvider;

impl SearchProvider for FakeSemanticProvider {
    fn kind(&self) -> SearchKind {
        // A real semantic provider would add its own SearchKind value.
        SearchKind::SourceUnit
    }
    fn candidates(&self, _corpus: &SearchCorpus) -> Vec<SearchCandidate> {
        vec![SearchCandidate {
            id: "semantic-1".into(),
            title: "一致连续性（语义实体）".into(),
            aliases: vec!["uniform continuity".into()],
            subtitle: None,
            target: SearchTarget::MapNode {
                node_id: "semantic-1".into(),
            },
            trashed: false,
            recency: 0,
        }]
    }
}

#[test]
fn registry_accepts_new_providers() {
    let mut registry = SearchRegistry::empty();
    registry.register(Box::new(FakeSemanticProvider));
    let response = search(
        &registry,
        &SearchCorpus::default(),
        &SearchQuery {
            text: "Uniform".into(),
            ..SearchQuery::default()
        },
    );
    assert_eq!(response.total, 1);
    assert_eq!(
        response.groups[0].hits[0].target,
        SearchTarget::MapNode {
            node_id: "semantic-1".into()
        }
    );
    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["groups"][0]["hits"][0]["target"]["kind"], "map_node");
    assert_eq!(json["groups"][0]["kind"], "source_unit");
}
