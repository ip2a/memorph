use memorph_lib::config;
use serde_json::json;
use std::process::Command;

struct ArchiveFixture {
    archive_ref: String,
    group_dir: std::path::PathBuf,
}

impl Drop for ArchiveFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.group_dir);
    }
}

fn write_archive_fixture() -> ArchiveFixture {
    let unique = format!(
        "cli-retrieve-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let group_dir = config::memorph_dir()
        .unwrap()
        .join("compression_archives")
        .join(&unique);
    std::fs::create_dir_all(&group_dir).unwrap();
    let archive = json!({
        "version": 1,
        "created_at": "2026-06-07T00:00:00Z",
        "canonical_id": unique,
        "source_provider_id": "claude",
        "target_provider_id": "codex",
        "summary_event_id": "summary-event",
        "source_event_ids": ["needle-event", "other-event"],
        "events": [
            {
                "id": "needle-event",
                "kind": "message",
                "role": "user",
                "timestamp": "2026-06-07T00:00:00Z",
                "links": {},
                "blocks": [
                    {
                        "type": "text",
                        "text": "needle detail from CLI archived original event"
                    }
                ],
                "metadata": {
                    "source": {
                        "provider_id": "claude",
                        "original_role": "user"
                    },
                    "fidelity": "preserved",
                    "provider_ext": {}
                }
            },
            {
                "id": "other-event",
                "kind": "message",
                "role": "assistant",
                "timestamp": "2026-06-07T00:00:00Z",
                "links": {},
                "blocks": [
                    {
                        "type": "text",
                        "text": "unrelated archived original event"
                    }
                ],
                "metadata": {
                    "source": {
                        "provider_id": "claude",
                        "original_role": "assistant"
                    },
                    "fidelity": "preserved",
                    "provider_ext": {}
                }
            }
        ]
    });
    std::fs::write(
        group_dir.join("archive.json"),
        serde_json::to_string_pretty(&archive).unwrap(),
    )
    .unwrap();

    ArchiveFixture {
        archive_ref: format!("memorph-archive://{}/archive.json", unique),
        group_dir,
    }
}

#[test]
fn compression_retrieve_cli_outputs_query_mode_json() {
    let fixture = write_archive_fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_memorph"))
        .args([
            "compression",
            "retrieve",
            &fixture.archive_ref,
            "--query",
            "needle",
            "--max-results",
            "5",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(value["archive_ref"], fixture.archive_ref);
    assert_eq!(value["retrieval_mode"], "query_matches");
    assert!(value["recommended_next_action"]
        .as_str()
        .unwrap()
        .contains("partial retrieval"));
    assert_eq!(value["source_event_count"], 2);
    assert_eq!(value["returned_event_ids"], json!(["needle-event"]));
    assert_eq!(value["returned_event_count"], 1);
    assert_eq!(value["omitted_event_count"], 1);
    assert_eq!(value["events"][0]["id"], "needle-event");
    assert_eq!(value["matches"][0]["event_id"], "needle-event");
}
