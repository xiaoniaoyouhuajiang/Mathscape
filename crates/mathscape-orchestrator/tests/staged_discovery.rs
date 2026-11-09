use mathscape_domain::*;
use mathscape_llm::*;
use mathscape_orchestrator::*;
use serde_json::{Value, json};

const SOURCE: &str = include_str!("../../mathscape-domain/tests/fixtures/staged-source.md");
const UNITS: &str = include_str!("../../mathscape-domain/tests/fixtures/staged-units.json");
const FACTS: &str = include_str!("../../mathscape-domain/tests/fixtures/staged-facts.json");

fn prepared(source: &str) -> PreparedUnitDiscoveryRequest {
    let doc = anchor_markdown(
        StableId::parse("test:staged-source").unwrap(),
        None,
        source.into(),
    )
    .unwrap();
    let selection = resolve_selection(
        &doc,
        DiscoverySelectionRequest {
            version: DISCOVERY_SELECTION_VERSION.into(),
            snapshot: doc.snapshot.clone(),
            selector: SourceSelector::WholeDocument,
            admission: DiscoveryAdmission::ReviewBeforePublish,
        },
    )
    .unwrap();
    build_unit_discovery_request(&doc, &selection, Default::default()).unwrap()
}
fn proposal(p: &PreparedUnitDiscoveryRequest) -> UnitDiscoveryProposal {
    let mut result: UnitDiscoveryProposal = serde_json::from_str(UNITS).unwrap();
    result.request_ref = p.model_input().request_ref.clone();
    result
}
fn has(report: &StageValidationReport, code: &str) -> bool {
    report.diagnostics.iter().any(|d| d.code == code)
}
fn quote(text: &str) -> Value {
    json!({"quote":text,"occurrence":null,"prefix":null,"suffix":null})
}
fn range(block: &str, start: &str, end: &str) -> DiscoverySelector {
    serde_json::from_value(json!({"kind":"quote_range","first":block,"start":quote(start),"last":block,"end":quote(end)})).unwrap()
}
fn occurrence(block: &str, text: &str, occurrence: Option<u32>) -> TextOccurrence {
    TextOccurrence::TextOccurrence {
        block: block.into(),
        text: text.into(),
        occurrence,
    }
}
fn profile() -> LlmProfile {
    LlmProfile {
        version: LLM_PROFILE_VERSION.into(),
        id: "test".into(),
        display_name: None,
        provider: "openai".into(),
        wire_protocol: WireProtocol::OpenAiChatCompletions,
        model: "scripted".into(),
        endpoint: None,
        credential: None,
        capabilities: Default::default(),
    }
}
fn response(text: &str) -> LlmResponse {
    LlmResponse {
        target: LlmResolvedTarget {
            profile_id: "test".into(),
            provider: "openai".into(),
            wire_protocol: WireProtocol::OpenAiChatCompletions,
            requested_model: "scripted".into(),
            resolved_model: "scripted".into(),
            provider_model: "scripted".into(),
        },
        parts: vec![LlmResponsePart::Text { text: text.into() }],
        reasoning: None,
        finish_reason: Some(LlmFinishReason::Completed { raw: "stop".into() }),
        usage: Default::default(),
        response_id: None,
        elapsed_ms: 1,
        raw_response: None,
    }
}

#[test]
fn input_is_lossless_and_keeps_authority_and_hashes_out_of_model_payload() {
    let p = prepared(SOURCE);
    let model = p.model_input();
    let text = model.leading_gap.clone()
        + &model
            .blocks
            .iter()
            .map(|b| b.text.clone() + &b.following_gap)
            .collect::<String>();
    assert_eq!(text, SOURCE);
    let wire = serde_json::to_string(model).unwrap();
    for forbidden in [
        "sha256",
        "artifact_id",
        "source_range",
        "admission",
        "input_fingerprint",
    ] {
        assert!(!wire.contains(forbidden));
    }
    assert_eq!(
        p.request().task.input_fingerprint.as_ref(),
        Some(p.request_fingerprint())
    );
    assert_ne!(p.request_fingerprint(), &p.input().fingerprint);
    assert_eq!(p.manifest().pipeline_version, DISCOVERY_PIPELINE_VERSION);
    assert_eq!(p.manifest().input_version, UNIT_DISCOVERY_INPUT_VERSION);
    assert_eq!(p.manifest().output_version, UNIT_DISCOVERY_VERSION);
}

#[tokio::test]
async fn stage_a_runs_once_and_freezes_ids_without_synthetic_v3_facts() {
    let p = prepared(SOURCE);
    let text = serde_json::to_string(&proposal(&p)).unwrap();
    let backend = ScriptedLlmBackend::new(profile(), [Ok(response(&text))]);
    let UnitProductionOutcome::Valid { units, run } =
        UnitDiscoveryProducer::new(&backend).produce(p).await
    else {
        panic!("expected valid A")
    };
    assert_eq!(backend.requests().len(), 1);
    assert_eq!(backend.streaming_request_count(), 1);
    assert_eq!(run.task_kind, "unit-discovery");
    assert_eq!(units.units()[1].proposal().unit_ref, "u2");
    assert_eq!(units.units()[1].proposal().parent.as_deref(), Some("u1"));
    assert!(
        units.units()[0].selectors()[0].range().end
            <= units.units()[1].selectors()[0].range().start
    );
    assert!(!units.is_partial());
    let checkpoint = serde_json::to_value(units.checkpoint()).unwrap();
    assert!(checkpoint["proposal"].get("source_labels").is_none());
    assert!(checkpoint["proposal"].get("relations").is_none());
    assert_eq!(
        checkpoint["unit_set_fingerprint"],
        json!(units.fingerprint())
    );
}

#[test]
fn frozen_identity_is_independent_of_request_token_but_changes_with_structure() {
    let a = prepared(SOURCE);
    let b = prepared(SOURCE);
    assert_ne!(a.model_input().request_ref, b.model_input().request_ref);
    let frozen_a = a.validate(proposal(&a), Default::default()).unwrap();
    let frozen_b = b.validate(proposal(&b), Default::default()).unwrap();
    assert_eq!(frozen_a.fingerprint(), frozen_b.fingerprint());
    let mut changed = proposal(&b);
    changed.units[1].parent = None;
    assert_ne!(
        frozen_a.fingerprint(),
        b.validate(changed, Default::default())
            .unwrap()
            .fingerprint()
    );
    assert!(has(
        &b.validate(proposal(&a), Default::default()).unwrap_err(),
        "request_mismatch"
    ));
    let canonical = frozen_a.proposal().clone();
    assert_eq!(
        a.validate(canonical, Default::default())
            .unwrap()
            .fingerprint(),
        frozen_a.fingerprint()
    );
    let checkpoint = frozen_a.checkpoint();
    assert_eq!(
        b.revalidate_checkpoint(&checkpoint, Default::default())
            .unwrap()
            .fingerprint(),
        frozen_a.fingerprint()
    );
    let mut tampered = checkpoint;
    tampered.unit_set_fingerprint =
        ContentHash::parse(format!("sha256:{}", "9".repeat(64))).unwrap();
    assert!(has(
        &b.revalidate_checkpoint(&tampered, Default::default())
            .unwrap_err(),
        "unit_set_mismatch"
    ));
}

#[test]
fn same_block_statement_and_proof_require_precise_nonoverlapping_boundaries() {
    let p = prepared("命题。证明。\n");
    let mut value = proposal(&p);
    value.units[0].selectors = vec![range("b1", "命题。", "命题。")];
    value.units[1].selectors = vec![range("b1", "证明。", "证明。")];
    value.coverage[0].selector = DiscoverySelector::BlockRange {
        first: "b1".into(),
        last: "b1".into(),
    };
    let frozen = p.validate(value.clone(), Default::default()).unwrap();
    assert_eq!(frozen.units()[0].selectors()[0].text(), "命题。");
    assert_eq!(frozen.units()[1].selectors()[0].text(), "证明。");
    value.units[0].selectors = vec![value.coverage[0].selector.clone()];
    value.units[1].selectors = vec![value.coverage[0].selector.clone()];
    assert!(has(
        &p.validate(value, Default::default()).unwrap_err(),
        "overlapping_units"
    ));
}

#[test]
fn nested_ancestor_is_allowed_but_crossing_ranges_and_siblings_are_not() {
    let p = prepared(SOURCE);
    let mut value = proposal(&p);
    value.units[0].unit_kind = DiscoveryUnitKind::Section;
    value.units[0].selectors = vec![value.coverage[0].selector.clone()];
    assert!(p.validate(value.clone(), Default::default()).is_ok());
    value.units[1].parent = None;
    assert!(has(
        &p.validate(value.clone(), Default::default()).unwrap_err(),
        "overlapping_units"
    ));
    value.units[1].parent = Some("u1".into());
    value.units[0].selectors = vec![DiscoverySelector::BlockRange {
        first: "b1".into(),
        last: "b2".into(),
    }];
    assert!(has(
        &p.validate(value, Default::default()).unwrap_err(),
        "overlapping_units"
    ));
}

#[test]
fn preflight_is_reported_even_when_location_fails_and_dependent_checks_are_skipped() {
    let p = prepared(SOURCE);
    let mut value = proposal(&p);
    value.units[0].parent = Some("u2".into());
    value.units[1].selectors = vec![range("b2", "不存在的证明", "不存在的证明")];
    let report = p.validate(value, Default::default()).unwrap_err();
    assert!(has(&report, "parent_cycle"));
    assert!(has(&report, "selector_resolution"));
    assert!(!has(&report, "overlapping_units"));
    assert!(!report.not_checked.is_empty());
}

#[test]
fn partial_coverage_completeness_and_limits_are_enforced() {
    let p = prepared(SOURCE);
    let mut value = proposal(&p);
    value.units.truncate(1);
    value.coverage = vec![
        DiscoveryCoverageProposal {
            selector: DiscoverySelector::BlockRange {
                first: "b1".into(),
                last: "b1".into(),
            },
            disposition: DiscoveryCoverageDisposition::Processed {},
        },
        DiscoveryCoverageProposal {
            selector: DiscoverySelector::BlockRange {
                first: "b2".into(),
                last: "b3".into(),
            },
            disposition: DiscoveryCoverageDisposition::Unprocessed {
                reason: "预算不足".into(),
            },
        },
    ];
    assert!(
        p.validate(value.clone(), Default::default())
            .unwrap()
            .is_partial()
    );
    value.coverage.pop();
    assert!(has(
        &p.validate(value, Default::default()).unwrap_err(),
        "coverage_gap"
    ));
    let mut value = proposal(&p);
    value.units[1].completeness = UnitCompleteness::RightTruncated;
    assert!(has(
        &p.validate(value, Default::default()).unwrap_err(),
        "invalid_completeness"
    ));
    assert!(has(
        &p.validate(
            proposal(&p),
            DiscoveryValidationLimits::new(1, 1, 100).unwrap()
        )
        .unwrap_err(),
        "limit_exceeded"
    ));
}

#[test]
fn clipped_input_cannot_authorize_full_physical_block() {
    let doc = anchor_markdown(
        StableId::parse("test:clipped").unwrap(),
        None,
        "之前。目标。之后。\n".into(),
    )
    .unwrap();
    let selection = resolve_selection(
        &doc,
        DiscoverySelectionRequest {
            version: DISCOVERY_SELECTION_VERSION.into(),
            snapshot: doc.snapshot.clone(),
            selector: SourceSelector::QuoteRange {
                first: doc.blocks[0].id.clone(),
                last: doc.blocks[0].id.clone(),
                start_quote: "目标。".into(),
                end_quote: "目标。".into(),
            },
            admission: DiscoveryAdmission::ReviewBeforePublish,
        },
    )
    .unwrap();
    let p = build_unit_discovery_request(&doc, &selection, Default::default()).unwrap();
    let mut value = proposal(&p);
    value.units.truncate(1);
    value.coverage[0].selector = DiscoverySelector::BlockRange {
        first: "b1".into(),
        last: "b1".into(),
    };
    assert!(has(
        &p.validate(value.clone(), Default::default()).unwrap_err(),
        "invalid_selector"
    ));
    value.units[0].selectors = vec![range("b1", "目标。", "目标。")];
    value.coverage[0].selector = range("b1", "目标。", "目标。");
    assert!(p.validate(value, Default::default()).is_ok());
}

#[test]
fn occurrence_matching_normalizes_layout_and_rejects_semantic_changes() {
    let p = prepared("公式 $x \\le 1$，标签 (20) \\quad (21)。\n");
    let resolver =
        OccurrenceResolver::new(p.input(), vec![p.input().requested.source_range.clone()]).unwrap();
    let located = resolver
        .resolve(&occurrence("b1", "$(20)   (21)$", None))
        .unwrap();
    assert!(located.normalized());
    assert_eq!(located.selector().text(), "(20) \\quad (21)");
    assert_eq!(
        resolver
            .resolve(&occurrence("b1", "$x < 1$", None))
            .unwrap_err()
            .code,
        OccurrenceErrorCode::QuoteNotFound
    );
    assert_eq!(
        resolver
            .resolve(&occurrence("b99", "(20)", None))
            .unwrap_err()
            .code,
        OccurrenceErrorCode::UnknownBlock
    );
    assert_eq!(
        resolver
            .resolve(&occurrence("b1", "(20)", Some(0)))
            .unwrap_err()
            .code,
        OccurrenceErrorCode::InvalidOccurrence
    );
}

#[test]
fn pagination_marker_cannot_be_used_as_a_fact_anchor() {
    let p = prepared("{21}------------------------------------------------\n");
    let resolver =
        OccurrenceResolver::new(p.input(), vec![p.input().requested.source_range.clone()]).unwrap();
    assert_eq!(
        resolver
            .resolve(&occurrence(
                "b1",
                "{21}------------------------------------------------",
                None
            ))
            .unwrap_err()
            .code,
        OccurrenceErrorCode::NonSemanticBlock
    );
}

#[test]
fn candidate_selection_binds_request_scope_revision_and_field() {
    let p = prepared("参考定义 2，再次参考定义 2。\n");
    let full = p.input().requested.source_range.clone();
    let resolver = OccurrenceResolver::new(p.input(), vec![full.clone()]).unwrap();
    let other = OccurrenceResolver::new(p.input(), vec![full]).unwrap();
    let anchor = occurrence("b1", "定义 2", None);
    assert_eq!(
        resolver.resolve(&anchor).unwrap_err().code,
        OccurrenceErrorCode::AmbiguousQuote
    );
    let base = ContentHash::parse(format!("sha256:{}", "1".repeat(64))).unwrap();
    let set = resolver
        .candidates(&anchor, &base, "references[0]")
        .unwrap();
    assert_eq!(set.candidates().len(), 2);
    let selected = resolver
        .choose_candidate(&set, &base, "references[0]", "c2")
        .unwrap();
    assert_eq!(selected.selector().text(), "定义 2");
    assert!(selected.selector().range().start > 10);
    assert_eq!(
        other
            .choose_candidate(&set, &base, "references[0]", "c2")
            .unwrap_err()
            .code,
        OccurrenceErrorCode::StaleCandidate
    );
    assert!(
        resolver
            .choose_candidate(&set, &base, "references[1]", "c2")
            .is_err()
    );
    let stale = ContentHash::parse(format!("sha256:{}", "2".repeat(64))).unwrap();
    assert!(
        resolver
            .choose_candidate(&set, &stale, "references[0]", "c2")
            .is_err()
    );
    assert!(
        resolver
            .choose_candidate(&set, &base, "references[0]", "c3")
            .is_err()
    );
}

#[test]
fn authorization_does_not_renumber_occurrences_and_range_cannot_cross_excluded_text() {
    let p = prepared("定义 2；定义 2\n");
    let text = &p.input().requested.segments[0].text();
    let start = text.rfind("定义 2").unwrap() as u64;
    let resolver = OccurrenceResolver::new(
        p.input(),
        vec![TextByteRange {
            start,
            end: p.input().requested.source_range.end,
        }],
    )
    .unwrap();
    let Err(empty_scope) = OccurrenceResolver::new(p.input(), vec![]) else {
        panic!("an empty scope must be rejected")
    };
    assert_eq!(empty_scope.code, OccurrenceErrorCode::UnauthorizedRange);
    assert_eq!(
        resolver
            .resolve(&occurrence("b1", "定义 2", Some(1)))
            .unwrap_err()
            .code,
        OccurrenceErrorCode::UnauthorizedRange
    );
    assert_eq!(
        resolver
            .resolve(&occurrence("b1", "定义 2", Some(2)))
            .unwrap()
            .selector()
            .range()
            .start,
        start
    );

    let p = prepared("前端。\n\n排除内容。\n\n末端。\n");
    let ranges: Vec<_> = p
        .input()
        .requested
        .segments
        .iter()
        .filter_map(|s| match s {
            DiscoveryInputSegment::Block { source_range, .. } => Some(source_range.clone()),
            _ => None,
        })
        .collect();
    let anchor=TextOccurrence::RangeOccurrence{selector:serde_json::from_value(json!({"kind":"quote_range","first":"b1","start":quote("前端。"),"last":"b3","end":quote("末端。")})).unwrap()};
    let scoped =
        OccurrenceResolver::new(p.input(), vec![ranges[0].clone(), ranges[2].clone()]).unwrap();
    assert_eq!(
        scoped.resolve(&anchor).unwrap_err().code,
        OccurrenceErrorCode::UnauthorizedRange
    );
    let all =
        OccurrenceResolver::new(p.input(), vec![p.input().requested.source_range.clone()]).unwrap();
    assert!(all.resolve(&anchor).unwrap().precision_review());

    let p = prepared("标题开始，正文，标题结束。\n");
    let anchor = TextOccurrence::RangeOccurrence {
        selector: serde_json::from_value(json!({
            "kind":"quote_range",
            "first":"b1",
            "start":quote("标题开始"),
            "last":"b1",
            "end":quote("标题结束。")
        }))
        .unwrap(),
    };
    let resolver =
        OccurrenceResolver::new(p.input(), vec![p.input().requested.source_range.clone()]).unwrap();
    let located = resolver.resolve(&anchor).unwrap();
    assert!(!located.precision_review());
    assert_eq!(
        located.original(),
        &occurrence("b1", "标题开始，正文，标题结束。", None)
    );
}

#[test]
fn fact_preflight_catches_self_reference_without_resolving_the_bad_quote() {
    let p = prepared(SOURCE);
    let units = proposal(&p);
    let input = SourceFactInput {
        version: SourceFactInputVersion::V2,
        leading_gap: String::new(),
        boundary: p.model_input().boundary.clone(),
        request_ref: "f1".into(),
        blocks: p.model_input().blocks.clone(),
        units: units
            .units
            .iter()
            .map(|u| FrozenUnitEntry {
                unit_ref: u.unit_ref.clone(),
                unit_kind: u.unit_kind,
                display_title: u.display_title.clone(),
                parent: u.parent.clone(),
                selectors: u.selectors.clone(),
            })
            .collect(),
        tasks: vec![
            FactTask {
                task_ref: "t1".into(),
                owner_unit_ref: "u1".into(),
                evidence_block_refs: vec!["b1".into()],
                context_block_refs: vec![],
            },
            FactTask {
                task_ref: "t2".into(),
                owner_unit_ref: "u2".into(),
                evidence_block_refs: vec!["b2".into(), "b3".into()],
                context_block_refs: vec![],
            },
        ],
        source_warnings: vec![],
    };
    let mut facts: SourceFactProposal = serde_json::from_str(FACTS).unwrap();
    assert!(preflight_source_facts(&input, &facts).is_valid());
    facts.tasks[1].references[0].anchor = occurrence("b99", "不存在", None);
    facts.tasks[1].references[0].target_hint = Some("u2".into());
    let report = preflight_source_facts(&input, &facts);
    assert!(has(&report, "invalid_reference"));
    assert!(!report.not_checked.is_empty());
}

#[test]
fn fact_ids_are_assigned_by_the_validator_and_legacy_refs_have_no_authority() {
    let p = prepared(SOURCE);
    let units = p
        .validate(proposal(&p), Default::default())
        .expect("fixture units validate");
    let batch = plan_source_facts(&p, &units, 16)
        .expect("fact plan")
        .into_iter()
        .next()
        .expect("one batch");
    let mut facts: SourceFactProposal = serde_json::from_str(FACTS).unwrap();
    facts.request_ref = batch.model_input().request_ref.clone();
    // Replay compatibility accepts old producer refs, including values which
    // were formerly rejected. They are never used as validated identities.
    facts.tasks[0].source_labels[0].fact_ref = "(legacy label)".into();
    facts.tasks[1].references[0].fact_ref = "duplicate".into();
    facts.tasks[1].references[1].fact_ref = "duplicate".into();
    facts.tasks[1].relations[0].fact_ref = "duplicate".into();
    let validation = batch.validate(&facts);
    assert!(validation.report.is_valid(), "{:?}", validation.report);
    assert_eq!(validation.tasks[0].labels()[0].fact_ref, "l1");
    assert_eq!(validation.tasks[1].references()[0].fact.fact_ref, "r1");
    assert_eq!(validation.tasks[1].references()[1].fact.fact_ref, "r2");
    assert_eq!(validation.tasks[1].relations()[0].fact_ref, "rel1");
}

#[tokio::test]
async fn producer_rejects_incomplete_transport_json_and_budget_without_retries() {
    for variant in [
        "max_tokens",
        "bad_json",
        "noncanonical",
        "wrong_request",
        "oversize",
    ] {
        let p = prepared(SOURCE);
        let mut value = serde_json::to_value(proposal(&p)).unwrap();
        if variant == "noncanonical" {
            value["units"][0]
                .as_object_mut()
                .unwrap()
                .remove("display_title");
        }
        if variant == "wrong_request" {
            value["request_ref"] = json!("another-request");
        }
        let mut r = response(&value.to_string());
        if variant == "max_tokens" {
            r.finish_reason = Some(LlmFinishReason::MaxTokens {
                raw: "length".into(),
            });
        }
        if variant == "bad_json" {
            r.parts = vec![LlmResponsePart::Text { text: "{".into() }];
        }
        let backend = ScriptedLlmBackend::new(profile(), [Ok(r)]);
        let limits = DiscoveryProducerLimits::new(
            if variant == "oversize" {
                10
            } else {
                2 * 1024 * 1024
            },
            Default::default(),
        )
        .unwrap();
        let result = UnitDiscoveryProducer::with_limits(&backend, limits)
            .produce(p)
            .await;
        assert!(
            matches!(result, UnitProductionOutcome::InvalidProposal { .. }),
            "{variant}: {result:?}"
        );
        assert_eq!(backend.requests().len(), 1);
    }
    let p = prepared(SOURCE);
    let backend = ScriptedLlmBackend::new(profile(), [Err(LlmError::Exhausted)]);
    assert!(matches!(
        UnitDiscoveryProducer::new(&backend).produce(p).await,
        UnitProductionOutcome::Failed { .. }
    ));

    let p = prepared(SOURCE);
    let mut no_reason = response(&serde_json::to_string(&proposal(&p)).unwrap());
    no_reason.finish_reason = None;
    let backend = ScriptedLlmBackend::new(profile(), [Ok(no_reason)]);
    assert!(matches!(
        UnitDiscoveryProducer::new(&backend).produce(p).await,
        UnitProductionOutcome::Valid { .. }
    ));
}

#[test]
fn schemas_match_snapshots_and_enforce_stage_separation() {
    for (schema, snapshot, sample) in [
        (
            unit_output_schema(),
            include_str!("../../../prompts/unit-discovery/v1/output.schema.json"),
            UNITS,
        ),
        (
            fact_output_schema(),
            include_str!("../../../prompts/source-facts/v1/output.schema.json"),
            FACTS,
        ),
    ] {
        assert_eq!(schema, serde_json::from_str::<Value>(snapshot).unwrap());
        let validator = jsonschema::validator_for(&schema).unwrap();
        let mut value: Value = serde_json::from_str(sample).unwrap();
        assert!(validator.is_valid(&value));
        value["trust"] = json!("accepted");
        assert!(!validator.is_valid(&value));
    }
    let mut units: Value = serde_json::from_str(UNITS).unwrap();
    units["units"][0]
        .as_object_mut()
        .unwrap()
        .remove("display_title");
    assert!(
        !jsonschema::validator_for(&unit_output_schema())
            .unwrap()
            .is_valid(&units)
    );
    let facts = fact_prompt_manifest();
    assert_eq!(facts.task_kind, "source-facts");
    assert_eq!(facts.pipeline_version, DISCOVERY_PIPELINE_VERSION);
    assert_eq!(facts.input_version, SOURCE_FACT_INPUT_VERSION);
    assert_eq!(facts.output_version, SOURCE_FACT_VERSION);
}

fn validated_fixture() -> (
    PreparedUnitDiscoveryRequest,
    ValidatedUnitSet,
    Vec<PreparedFactRequest>,
    Vec<ValidatedFactTask>,
) {
    let prepared = prepared(SOURCE);
    let units = prepared
        .validate(proposal(&prepared), Default::default())
        .unwrap();
    let batches = plan_source_facts(&prepared, &units, 16).unwrap();
    let mut facts: SourceFactProposal = serde_json::from_str(FACTS).unwrap();
    facts.request_ref = batches[0].model_input().request_ref.clone();
    let validation = batches[0].validate(&facts);
    assert!(validation.report.is_valid(), "{:?}", validation.report);
    (prepared, units, batches, validation.tasks)
}

#[test]
fn persisted_wire_checkpoints_restore_authority_only_by_revalidation() {
    let prepared = prepared(SOURCE);
    let units = prepared
        .validate(proposal(&prepared), Default::default())
        .unwrap();
    let facts: SourceFactProposal = serde_json::from_str(FACTS).unwrap();
    let restored =
        revalidate_discovery_checkpoint(&prepared, &units.checkpoint(), facts.tasks.clone(), 16)
            .unwrap();
    assert_eq!(restored.units().fingerprint(), units.fingerprint());
    assert!(restored.result().complete());
    let differently_batched =
        revalidate_discovery_checkpoint(&prepared, &units.checkpoint(), facts.tasks.clone(), 1)
            .unwrap();
    assert_eq!(
        discovery_result_fingerprint(restored.result()),
        discovery_result_fingerprint(differently_batched.result())
    );

    let mut invalid = facts.tasks;
    invalid[0].task_ref = "unknown-task".into();
    assert!(matches!(
        revalidate_discovery_checkpoint(&prepared, &units.checkpoint(), invalid, 16),
        Err(DiscoveryCheckpointRevalidationError::UnknownFactTask(task))
            if task == "unknown-task"
    ));
}

#[test]
fn discovery_assembly_is_stable_under_fact_arrival_order() {
    let (_prepared, units, batches, tasks) = validated_fixture();
    let forward = assemble_discovery(&units, &batches, tasks.clone()).unwrap();
    let mut reversed_tasks = tasks;
    reversed_tasks.reverse();
    let reversed = assemble_discovery(&units, &batches, reversed_tasks).unwrap();

    assert_eq!(
        forward
            .tasks()
            .iter()
            .map(|task| task.task_ref())
            .collect::<Vec<_>>(),
        reversed
            .tasks()
            .iter()
            .map(|task| task.task_ref())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        serde_json::to_value(&forward).unwrap(),
        serde_json::to_value(&reversed).unwrap()
    );
    assert_eq!(
        discovery_result_fingerprint(&forward),
        discovery_result_fingerprint(&reversed)
    );
}

fn acceptance(
    result: &DiscoveryResult,
    selection: AcceptanceSelection,
) -> DiscoveryAcceptanceDecision {
    DiscoveryAcceptanceDecision {
        decision_id: StableId::parse("mathscape:acceptance/staged-fixture").unwrap(),
        discovery_run_id: StableId::parse("mathscape:discovery-run/staged-fixture").unwrap(),
        actor: ActorRef::parse("fixture-reviewer").unwrap(),
        decided_at: Timestamp::parse("2026-09-12T12:00:00+08:00").unwrap(),
        authority: AcceptanceAuthority::HumanReview,
        expected_result_fingerprint: discovery_result_fingerprint(result),
        selection,
    }
}

#[test]
fn registration_plan_accepts_a_validated_partial_result_without_granting_trust() {
    let (prepared, units, batches, mut tasks) = validated_fixture();
    tasks.truncate(1);
    let result = assemble_discovery(&units, &batches, tasks).unwrap();
    assert!(!result.complete());
    assert_eq!(result.pending_tasks(), &["t2"]);
    let plan = plan_source_registration(
        prepared.input(),
        &units,
        &result,
        acceptance(&result, AcceptanceSelection::AllValidated),
    )
    .unwrap();
    assert!(plan.partial);
    assert_eq!(plan.units.len(), 2);
    assert_eq!(plan.pending_tasks, ["t2"]);
    assert!(plan.units.iter().any(|unit| unit.facts_pending));
    assert!(plan.relations.is_empty());
    let wire = serde_json::to_value(&plan).unwrap();
    assert!(wire.get("trust").is_none());
    assert!(wire["units"].as_array().unwrap().iter().all(|unit| {
        unit["stable_id"]
            .as_str()
            .unwrap()
            .starts_with("mathscape:source-unit/")
    }));
}

#[test]
fn registration_plan_requires_parent_and_relation_closed_explicit_selection() {
    let (prepared, units, batches, tasks) = validated_fixture();
    let result = assemble_discovery(&units, &batches, tasks).unwrap();
    assert!(result.complete());
    let error = plan_source_registration(
        prepared.input(),
        &units,
        &result,
        acceptance(
            &result,
            AcceptanceSelection::ExplicitUnits {
                unit_refs: vec!["u2".into()],
            },
        ),
    )
    .unwrap_err();
    assert_eq!(
        error,
        SourceRegistrationPlanError::MissingParent {
            unit: "u2".into(),
            parent: "u1".into(),
        }
    );

    let plan = plan_source_registration(
        prepared.input(),
        &units,
        &result,
        acceptance(&result, AcceptanceSelection::AllValidated),
    )
    .unwrap();
    assert!(!plan.partial);
    assert_eq!(plan.relations.len(), 1);
    assert_eq!(plan.relations[0].relation, DiscoveryRelationKind::Proves);
    assert!(!plan.spans.is_empty());
    // Hash/identity contract shared with `propose_changes`: the stored
    // `normalized_text_hash` is `source_text_hash` of the exact range text,
    // and the span id is `markdown_span_id(artifact, range)`.
    for span in &plan.spans {
        let text = &SOURCE[span.range.start as usize..span.range.end as usize];
        assert_eq!(span.exact_text_hash, source_text_hash(text));
        assert_eq!(
            span.stable_id,
            markdown_span_id(&span.markdown_artifact_id, &span.range)
        );
    }
}

#[test]
fn registration_plan_rejects_a_stale_acceptance_decision() {
    let (prepared, units, batches, tasks) = validated_fixture();
    let result = assemble_discovery(&units, &batches, tasks).unwrap();
    let mut decision = acceptance(&result, AcceptanceSelection::AllValidated);
    decision.expected_result_fingerprint =
        ContentHash::parse(format!("sha256:{}", "9".repeat(64))).unwrap();
    assert_eq!(
        plan_source_registration(prepared.input(), &units, &result, decision).unwrap_err(),
        SourceRegistrationPlanError::StaleDecision
    );
}
