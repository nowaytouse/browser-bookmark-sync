use super::*;
use serde_json::{json, Value};

fn observation(backend: &str, actual_request: &str, cache_hit: bool) -> BackendObservation {
    BackendObservation {
        backend: backend.into(),
        backend_version: "fixture-v1".into(),
        run_id: "current-round".into(),
        check_id: "current-check".into(),
        started_at: "2026-10-25T08:00:00Z".parse().unwrap(),
        finished_at: "2026-10-25T08:00:01Z".parse().unwrap(),
        actual_request: actual_request.into(),
        cache_hit,
        status_code: Some(200),
        ..Default::default()
    }
}

#[test]
fn exported_provenance_preserves_final_observation_identity_and_partitions_counts() {
    let dir = tempfile::tempdir().unwrap();
    let fresh = observation("native-reqwest", "yes", false);
    let mut historical = observation("same_day_resume", "no", true);
    historical.run_id = "original-round".into();
    historical.check_id = "original-check".into();
    historical.started_at = "2026-10-25T06:00:00Z".parse().unwrap();
    historical.finished_at = historical.started_at;
    historical.request_method = "REUSE".into();
    let source = json!({
        "run_id": "dns-original-round", "check_id": "dns-original-check",
        "config_hash": "source-config", "rule_version": "11"
    });
    let mut cached_json = serde_json::to_value(observation("native-dns", "no", true)).unwrap();
    cached_json["started_at"] = json!("2026-10-25T05:00:00Z");
    cached_json["finished_at"] = json!("2026-10-25T05:00:01Z");
    cached_json["source"] = source.clone();
    // JSON injection compiles against the old model, which silently loses source.
    let cached_dns: BackendObservation = serde_json::from_value(cached_json).unwrap();
    let escaped_url = "https://fixture.example.test/?q=\"x\",y&tag=<tag>\nnext";
    let results = vec![
        (
            "fresh".into(),
            escaped_url.into(),
            HealthVerdict::Live,
            vec![fresh.clone()],
        ),
        (
            "historical".into(),
            "https://fixture.example.test/history".into(),
            HealthVerdict::Live,
            vec![historical],
        ),
        (
            "mixed".into(),
            "https://fixture.example.test/mixed".into(),
            HealthVerdict::Live,
            vec![fresh, cached_dns],
        ),
        (
            "empty".into(),
            "https://fixture.example.test/empty".into(),
            HealthVerdict::NeedsReview,
            vec![],
        ),
        (
            "no-request".into(),
            "https://fixture.example.test/no".into(),
            HealthVerdict::NeedsReview,
            vec![observation("security", "no", false)],
        ),
        (
            "unknown".into(),
            "https://fixture.example.test/unknown".into(),
            HealthVerdict::NeedsReview,
            vec![observation("unknown", "unknown", false)],
        ),
    ];
    let original = serde_json::to_value(&results).unwrap();
    let summary = ReportExporter::export_all_for_scope(
        dir.path(),
        "current-scan",
        1.25,
        ProbeScope::Content,
        false,
        &results,
    )
    .unwrap();
    let read_json = |name| -> Value {
        serde_json::from_slice(&std::fs::read(dir.path().join(name)).unwrap()).unwrap()
    };
    let report = read_json("health-provenance.json");
    assert_eq!(report, read_json("content-health-provenance.json"));
    assert_eq!(report["schema_version"], "health-provenance-v1");
    assert_eq!(report["scan_id"], "current-scan");
    assert_eq!(report["probe_scope"], "content");
    let counts = json!({
        "fresh_probe_targets": 2, "reused_only_targets": 1, "unprobed_targets": 3,
        "total_observations": 6, "fresh_request_observations": 2,
        "reused_observations": 2, "no_request_observations": 1,
        "unknown_request_observations": 1
    });
    assert_eq!(report["counts"], counts);
    let summary_json = serde_json::to_value(&summary).unwrap();
    assert_eq!(summary_json["provenance"], counts);
    assert_eq!(summary_json["scan_id"], "current-scan");
    assert_eq!(summary_json["run_id"], "current-scan");
    assert_eq!(read_json("health-summary.json"), summary_json);
    let targets = report["targets"].as_array().unwrap();
    assert_eq!(targets.len(), results.len());
    for (id, url, verdict, observations) in &results {
        let target = targets
            .iter()
            .find(|target| target["bookmark_id"] == *id)
            .unwrap();
        assert_eq!(target["url"], *url);
        assert_eq!(target["verdict"], verdict.as_str());
        let actual = target["observations"].as_array().unwrap();
        assert_eq!(actual.len(), observations.len());
        for (exported, input) in actual.iter().zip(observations) {
            let expected = serde_json::to_value(input).unwrap();
            for field in [
                "backend",
                "backend_version",
                "run_id",
                "check_id",
                "started_at",
                "finished_at",
                "actual_request",
                "cache_hit",
                "request_method",
            ] {
                assert_eq!(exported[field], expected[field], "{id}: {field}");
            }
            assert!(
                exported.get("source").is_some(),
                "unknown source must be explicit null"
            );
            assert_eq!(
                exported["source"],
                if input.backend == "native-dns" {
                    source.clone()
                } else {
                    Value::Null
                }
            );
        }
    }
    assert_eq!(
        serde_json::to_value(&results).unwrap(),
        original,
        "export must not rewrite source observations"
    );
    let manifest = read_json("health-run-manifest.json");
    let artifacts = manifest["artifacts"].as_array().unwrap();
    for name in ["health-provenance.json", "content-health-provenance.json"] {
        assert!(artifacts.contains(&json!(name)));
    }
    let csv = std::fs::read_to_string(dir.path().join("health-observations.csv")).unwrap();
    let mut expected_csv =
        "bookmark_id,url,verdict,backend,status_code,error_kind,response_time_ms\n".to_string();
    for (id, url, verdict, observations) in &results {
        for input in observations {
            let fields = [
                id.as_str(),
                url.as_str(),
                verdict.as_str(),
                input.backend.as_str(),
                "200",
                "",
                "0",
            ];
            expected_csv.push_str(
                &fields
                    .map(|field| format!("\"{}\"", field.replace('"', "\"\"")))
                    .join(","),
            );
            expected_csv.push('\n');
        }
    }
    assert_eq!(
        csv, expected_csv,
        "legacy CSV schema and escaping must remain unchanged"
    );
    assert_eq!(
        csv,
        std::fs::read_to_string(dir.path().join("content-health-observations.csv")).unwrap()
    );
}

#[test]
fn legacy_summary_deserializes_with_unknown_scan_and_provenance() {
    let mut legacy = serde_json::to_value(HealthRunSummaryReport {
        run_id: "legacy-round".into(),
        probe_scope: "content".into(),
        ..Default::default()
    })
    .unwrap();
    legacy.as_object_mut().unwrap().remove("scan_id");
    legacy.as_object_mut().unwrap().remove("provenance");
    let decoded: HealthRunSummaryReport = serde_json::from_value(legacy).unwrap();
    let serialized = serde_json::to_value(decoded).unwrap();
    assert_eq!(serialized["run_id"], "legacy-round");
    assert!(serialized["scan_id"].is_null());
    assert!(serialized["provenance"].is_null());
}
