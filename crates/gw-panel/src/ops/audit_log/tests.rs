//! Unit tests for the unified audit feed.
//!
//! The three fetchers need Postgres and live in the integration suite. Pinned
//! here: source selection, the in-memory merge/paginate, and the `omitempty`
//! shape — the last one matters because a key that should have vanished is
//! indistinguishable from a real value to the console.

use super::*;
use crate::paging::page_params;

fn entry(id: &str, source: &str, at: DateTime<Utc>) -> AuditLogEntry {
    AuditLogEntry {
        id: id.to_owned(),
        source: source.to_owned(),
        actor_id: 0,
        actor_email: String::new(),
        action: "a".to_owned(),
        target: String::new(),
        method: String::new(),
        path: String::new(),
        status_code: 0,
        ip_address: String::new(),
        request_id: String::new(),
        metadata: None,
        created_at: at,
    }
}

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).expect("in range")
}

// ---------------------------------------------------------------- selection

#[test]
fn an_unset_or_all_selector_takes_every_source() {
    for selector in ["", "all"] {
        for source in [SOURCE_PANEL, SOURCE_SDK, SOURCE_BALANCE] {
            assert!(source_selected(selector, source));
        }
    }
}

#[test]
fn a_named_selector_takes_only_that_source() {
    assert!(source_selected(SOURCE_SDK, SOURCE_SDK));
    assert!(!source_selected(SOURCE_SDK, SOURCE_PANEL));
    assert!(!source_selected(SOURCE_SDK, SOURCE_BALANCE));
}

#[test]
fn an_unknown_selector_takes_nothing() {
    // A typo must return an empty feed, not silently widen to everything.
    for source in [SOURCE_PANEL, SOURCE_SDK, SOURCE_BALANCE] {
        assert!(!source_selected("panle", source));
    }
}

// ---------------------------------------------------------------- paging

#[test]
fn the_feed_is_ordered_newest_first() {
    let entries = vec![
        entry("a", SOURCE_PANEL, at(100)),
        entry("b", SOURCE_SDK, at(300)),
        entry("c", SOURCE_BALANCE, at(200)),
    ];
    let (items, total) = paginate(entries, 1, 10);
    assert_eq!(total, 3);
    let ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(ids, ["b", "c", "a"]);
}

#[test]
fn sources_interleave_by_time_rather_than_by_table() {
    // The whole point of the merge: an `sdk` row from a second ago outranks a
    // `panel` row from an hour ago.
    let entries = vec![
        entry("panel-old", SOURCE_PANEL, at(1)),
        entry("sdk-new", SOURCE_SDK, at(999)),
    ];
    let (items, _) = paginate(entries, 1, 10);
    assert_eq!(items.first().map(|item| item.id.as_str()), Some("sdk-new"));
}

#[test]
fn a_page_past_the_end_is_empty_rather_than_an_error() {
    let entries = vec![entry("a", SOURCE_PANEL, at(1))];
    let (items, total) = paginate(entries, 99, 30);
    assert!(items.is_empty());
    // `total` still reports the merged candidate count, so the pager can walk
    // back rather than concluding the feed is empty.
    assert_eq!(total, 1);
}

#[test]
fn the_last_page_is_short_rather_than_padded() {
    let entries: Vec<AuditLogEntry> = (0..5)
        .map(|index| entry(&format!("e{index}"), SOURCE_PANEL, at(index)))
        .collect();
    let (items, total) = paginate(entries, 2, 3);
    assert_eq!(total, 5);
    assert_eq!(items.len(), 2);
}

#[test]
fn the_feed_page_size_ceiling_is_wider_than_the_panel_default() {
    // The console pulls wider pages than the rest of the panel; 旧实现的上限是
    // 200，而 `page_params` 的上限是 100。钉住它们确实不同，
    // so a later "unify the paging helpers" refactor cannot quietly narrow it.
    let (_, wide) = audit_page_params(None, Some("200"));
    let (_, narrow) = page_params(None, Some("200"), 30);
    assert!(wide > narrow);
}

#[test]
fn an_absent_page_size_uses_the_feed_default() {
    let (page, size) = audit_page_params(None, None);
    assert_eq!(page, 1);
    let (_, clamped) = audit_page_params(None, Some("0"));
    assert!(clamped >= 1);
    assert!(size >= 1);
}

// ---------------------------------------------------------------- shape

#[test]
fn empty_optional_fields_disappear_from_the_body() {
    // 旧实现在这些字段上打 `omitempty`；发出 `"target": ""` 会让控制台
    // render an empty column instead of hiding it.
    let body = serde_json::to_value(entry("panel-1", SOURCE_PANEL, at(0))).expect("serializes");
    let object = body.as_object().expect("object");
    for absent in [
        "actor_email",
        "target",
        "method",
        "path",
        "status_code",
        "ip_address",
        "request_id",
        "metadata",
    ] {
        assert!(!object.contains_key(absent), "{absent} should be omitted");
    }
    for present in ["id", "source", "actor_id", "action", "created_at"] {
        assert!(
            object.contains_key(present),
            "{present} must always be sent"
        );
    }
}

#[test]
fn an_empty_metadata_object_is_omitted_too() {
    let mut row = entry("panel-1", SOURCE_PANEL, at(0));
    row.metadata = Some(Map::new());
    let body = serde_json::to_value(&row).expect("serializes");
    assert!(!body.as_object().expect("object").contains_key("metadata"));

    row.metadata = Some(Map::from_iter([("k".to_owned(), json!(1))]));
    let body = serde_json::to_value(&row).expect("serializes");
    assert!(body.as_object().expect("object").contains_key("metadata"));
}

#[test]
fn a_non_object_json_column_reads_as_no_metadata() {
    // 旧实现的 `decodeJSONMap` 对任何不是 JSON 对象的值返回 nil，
    // and the tag then drops the key.
    assert!(as_object(None).is_none());
    assert!(as_object(Some(json!(null))).is_none());
    assert!(as_object(Some(json!([1, 2]))).is_none());
    assert!(as_object(Some(json!("text"))).is_none());
    assert!(as_object(Some(json!({"k": 1}))).is_some());
}

// ---------------------------------------------------------------- sdk rows

fn usage_row() -> UsageEntryRow {
    UsageEntryRow {
        id: 9,
        user_id: 3,
        api_key_id: 4,
        request_id: Some("req-1".to_owned()),
        idempotency_key: None,
        model: Some("gpt-4o".to_owned()),
        provider: Some("openai".to_owned()),
        tokens_in: Some(10),
        tokens_out: Some(20),
        input_cost: Some(0.1),
        output_cost: Some(0.2),
        total_cost: Some(0.3),
        actual_cost: Some(0.3),
        stream: Some(true),
        duration_ms: Some(42),
        ip_address: Some("10.0.0.1".to_owned()),
        failed: Some(false),
        created_at: at(5),
    }
}

#[test]
fn a_usage_row_is_actioned_by_provider() {
    let mapped = usage_row().to_entry();
    assert!(mapped.action.starts_with(SOURCE_SDK));
    assert!(mapped.action.ends_with("openai"));
    assert_eq!(mapped.target, "gpt-4o");
    assert_eq!(mapped.actor_id, 3);
    assert!(mapped.id.starts_with(SOURCE_SDK));
}

#[test]
fn a_providerless_row_still_reaches_the_feed() {
    // A request that died before an upstream was chosen has no provider and no
    // model; dropping it would hide exactly the failures an operator is looking
    // for.
    let mut row = usage_row();
    row.provider = None;
    row.model = None;
    let mapped = row.to_entry();
    assert!(!mapped.action.is_empty());
    assert!(mapped.target.contains("req-1"));
}

#[test]
fn a_failed_call_is_reported_as_an_upstream_failure() {
    let mut row = usage_row();
    row.failed = Some(true);
    let failed = row.to_entry();
    row.failed = Some(false);
    let succeeded = row.to_entry();

    // The exact codes 沿用旧实现；需要保证的是它们不同、且失败为 5xx
    // failure is a 5xx — the console colours the row on that.
    assert_ne!(failed.status_code, succeeded.status_code);
    assert!(failed.status_code >= 500);
    assert!((200..300).contains(&succeeded.status_code));
}

#[test]
fn usage_metadata_carries_the_billing_columns() {
    let mapped = usage_row().to_entry();
    let meta = mapped.metadata.expect("usage rows always carry metadata");
    for key in [
        "model",
        "provider",
        "tokens_in",
        "tokens_out",
        "input_cost",
        "output_cost",
        "total_cost",
        "actual_cost",
        "stream",
        "duration_ms",
        "failed",
        "api_key_id",
        "idempotency",
    ] {
        assert!(meta.contains_key(key), "{key} missing from usage metadata");
    }
}

// ---------------------------------------------------------------- crypto

#[test]
fn constant_time_eq_matches_plain_equality() {
    for (left, right) in [
        (&b""[..], &b""[..]),
        (&b"abc"[..], &b"abc"[..]),
        (&b"abc"[..], &b"abd"[..]),
        (&b"abc"[..], &b"ab"[..]),
        (&b"ab"[..], &b"abc"[..]),
    ] {
        assert_eq!(constant_time_eq(left, right), left == right);
    }
}

#[test]
fn verify_decodes_status_code_as_bigint() {
    // 列是 bigint。这个类型签一旦退回 i32，连库 verify 会整表读失败。
    fn as_i64(code: Option<i64>) -> i64 {
        code.unwrap_or_default()
    }
    let row = AuditRow {
        id: 1,
        source: None,
        actor_id: None,
        actor_email: None,
        actor_role: None,
        action: None,
        target: None,
        method: None,
        path: None,
        status_code: Some(200),
        ip_address: None,
        request_id: None,
        metadata: None,
        created_at: at(0),
        entry_hash: None,
    };
    assert_eq!(as_i64(row.status_code), 200);
    assert_eq!(row.to_entry().status_code, 200);
}
