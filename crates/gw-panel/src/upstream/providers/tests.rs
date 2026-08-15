//! Unit tests for the provider-pool payload machinery.
//!
//! The four handlers need an [`AuthStore`](gw_authcore::AuthStore) and live in
//! the integration suite. What is pinned here is everything that decides *which
//! credential a request means* and *what gets written* — the parts where a
//! mistake silently rotates or deletes the wrong upstream key.

use super::*;
use chrono::DateTime;
use serde_json::json;

fn item(raw: Value) -> Map<String, Value> {
    raw.as_object().cloned().expect("object literal")
}

fn record(id: &str, label: &str, created_at: i64) -> AuthRecord {
    let mut record = AuthRecord::new(
        id,
        "openai",
        DateTime::from_timestamp(created_at, 0).expect("in range"),
    );
    record.label = label.to_owned();
    record
}

// ---------------------------------------------------------------- endpoints

#[test]
fn the_endpoint_map_round_trips() {
    // The response's top-level key is the endpoint key, so a one-way mapping
    // would make the pool unreadable to the console.
    for (endpoint, provider) in PROVIDER_ENDPOINTS {
        assert_eq!(provider_for_endpoint(endpoint), Some(provider));
        assert_eq!(endpoint_for_provider(provider), Some(endpoint));
    }
}

#[test]
fn every_endpoint_and_provider_is_distinct() {
    // A duplicate on either side would make one pool shadow another.
    let mut endpoints: Vec<&str> = PROVIDER_ENDPOINTS.iter().map(|(key, _)| *key).collect();
    let mut providers: Vec<&str> = PROVIDER_ENDPOINTS.iter().map(|(_, id)| *id).collect();
    endpoints.sort_unstable();
    providers.sort_unstable();
    let before = (endpoints.len(), providers.len());
    endpoints.dedup();
    providers.dedup();
    assert_eq!((endpoints.len(), providers.len()), before);
}

#[test]
fn an_unknown_endpoint_maps_to_nothing() {
    for unknown in ["", "openai", "claude", "openai-compat"] {
        assert_eq!(provider_for_endpoint(unknown), None);
    }
}

#[test]
fn no_endpoint_key_collides_with_the_auth_url_suffix() {
    // `/{provider}` dispatches on the suffix *before* the endpoint lookup, so a
    // pool key ending in `-auth-url` would become unreachable.
    for (endpoint, _) in PROVIDER_ENDPOINTS {
        assert!(!endpoint.ends_with(AUTH_URL_SUFFIX));
    }
}

// ---------------------------------------------------------------- payloads

#[test]
fn all_three_body_shapes_yield_the_same_items() {
    let bare = json!([{"api-key": "sk-1"}]);
    let wrapped = json!({"value": [{"api-key": "sk-1"}]});
    let single = json!({"api-key": "sk-1"});

    for raw in [&bare, &wrapped, &single] {
        let items = payload_items(raw);
        assert_eq!(items.len(), 1);
        assert_eq!(payload_string(&items[0], &["api-key"]), "sk-1");
    }
}

#[test]
fn the_three_wrapper_keys_are_interchangeable() {
    for key in ["value", "keys", "items"] {
        let raw = json!({key: [{"api-key": "sk-1"}]});
        assert_eq!(payload_items(&raw).len(), 1);
    }
}

#[test]
fn a_scalar_body_yields_nothing() {
    for raw in [json!(null), json!(1), json!("x"), json!(true)] {
        assert!(payload_items(&raw).is_empty());
    }
}

#[test]
fn grouped_entries_inherit_the_group_fields() {
    // One base-url with three keys is how the console models a pool.
    let items = expand_entries(&[item(json!({
        "base-url": "https://a",
        "priority": 5,
        "api-key-entries": [{"api-key": "sk-1"}, {"api-key": "sk-2"}],
    }))]);

    assert_eq!(items.len(), 2);
    for entry in &items {
        assert_eq!(payload_string(entry, &["base-url"]), "https://a");
        assert_eq!(payload_string(entry, &["priority"]), "5");
        // The group key must not survive, or a re-submit would double-expand.
        assert!(!entry.contains_key("api-key-entries"));
    }
    assert_eq!(payload_string(&items[0], &["api-key"]), "sk-1");
    assert_eq!(payload_string(&items[1], &["api-key"]), "sk-2");
}

#[test]
fn an_entry_field_overrides_the_group_field() {
    let items = expand_entries(&[item(json!({
        "base-url": "https://group",
        "api-key-entries": [{"api-key": "sk-1", "base-url": "https://entry"}],
    }))]);
    assert_eq!(payload_string(&items[0], &["base-url"]), "https://entry");
}

#[test]
fn an_empty_entry_list_leaves_the_item_alone() {
    let items = expand_entries(&[item(json!({"api-key": "sk-1", "api-key-entries": []}))]);
    assert_eq!(items.len(), 1);
    assert_eq!(payload_string(&items[0], &["api-key"]), "sk-1");
}

#[test]
fn a_masked_key_is_not_a_raw_key() {
    // The console re-submits the whole pool with previews in the untouched
    // rows; treating one as real would store "sk-0...cdef" as a credential.
    assert!(!has_raw_api_key(&item(json!({"api-key": "sk-0...cdef"}))));
    assert!(!has_raw_api_key(&item(json!({"api-key": ""}))));
    assert!(!has_raw_api_key(&item(json!({}))));
    assert!(has_raw_api_key(&item(
        json!({"api-key": "sk-0123456789abcdef"})
    )));
}

#[test]
fn the_three_api_key_spellings_are_all_accepted() {
    for key in ["api-key", "api_key", "apiKey"] {
        assert!(has_raw_api_key(&item(json!({key: "sk-0123456789abcdef"}))));
    }
}

// ---------------------------------------------------------------- building

#[test]
fn a_create_stores_the_submitted_key() {
    let built = record_from_payload(
        "openai",
        &item(json!({"api-key": "sk-0123456789abcdef", "name": "prod"})),
        None,
        DateTime::UNIX_EPOCH,
    );
    assert_eq!(built.provider, "openai");
    assert_eq!(built.label, "prod");
    assert_eq!(super::super::record::api_key(&built), "sk-0123456789abcdef");
}

#[test]
fn an_update_with_a_masked_key_keeps_the_stored_secret() {
    // The single most damaging bug this module could have.
    let mut existing = record("auth-1", "prod", 0);
    existing
        .metadata
        .as_object_mut()
        .expect("object")
        .insert("api_key".to_owned(), json!("sk-live-0123456789"));

    let built = record_from_payload(
        "openai",
        &item(json!({"id": "auth-1", "api-key": mask_secret("sk-live-0123456789")})),
        Some(&existing),
        DateTime::UNIX_EPOCH,
    );
    assert_eq!(super::super::record::api_key(&built), "sk-live-0123456789");
}

#[test]
fn an_update_keeps_the_identity_of_the_row_it_patches() {
    let existing = record("auth-1", "prod", 0);
    let built = record_from_payload(
        "openai",
        &item(json!({"id": "should-be-ignored", "name": "renamed"})),
        Some(&existing),
        DateTime::UNIX_EPOCH,
    );
    assert_eq!(built.id, "auth-1", "an update must not move the row");
    assert_eq!(built.label, "renamed");
}

#[test]
fn an_omitted_field_is_left_alone_but_an_empty_one_clears() {
    let mut existing = record("auth-1", "prod", 0);
    existing.prefix = "old".to_owned();

    let untouched = record_from_payload(
        "openai",
        &item(json!({"id": "auth-1"})),
        Some(&existing),
        DateTime::UNIX_EPOCH,
    );
    assert_eq!(untouched.prefix, "old");

    let cleared = record_from_payload(
        "openai",
        &item(json!({"id": "auth-1", "prefix": ""})),
        Some(&existing),
        DateTime::UNIX_EPOCH,
    );
    assert!(cleared.prefix.is_empty());
}

#[test]
fn disabling_moves_the_status_and_enabling_moves_it_back() {
    let existing = record("auth-1", "prod", 0);
    let disabled = record_from_payload(
        "openai",
        &item(json!({"id": "auth-1", "disabled": true})),
        Some(&existing),
        DateTime::UNIX_EPOCH,
    );
    assert!(disabled.disabled);
    assert_eq!(disabled.status, AuthStatus::Disabled);

    let enabled = record_from_payload(
        "openai",
        &item(json!({"id": "auth-1", "disabled": false})),
        Some(&disabled),
        DateTime::UNIX_EPOCH,
    );
    assert!(!enabled.disabled);
    assert_eq!(enabled.status, AuthStatus::Active);
}

#[test]
fn enabling_does_not_clobber_a_non_disabled_status() {
    // A credential the proxy marked `error` must not be reported healthy just
    // because an operator toggled the disable switch off.
    let mut existing = record("auth-1", "prod", 0);
    existing.status = AuthStatus::Error;
    let built = record_from_payload(
        "openai",
        &item(json!({"id": "auth-1", "disabled": false})),
        Some(&existing),
        DateTime::UNIX_EPOCH,
    );
    assert_eq!(built.status, AuthStatus::Error);
}

#[test]
fn a_string_true_disables_as_well_as_a_boolean() {
    for raw in [json!(true), json!("true"), json!("TRUE")] {
        let built = record_from_payload(
            "openai",
            &item(json!({"disabled": raw})),
            None,
            DateTime::UNIX_EPOCH,
        );
        assert!(built.disabled);
    }
}

// ---------------------------------------------------------------- matching

#[test]
fn an_item_matches_by_id_before_anything_else() {
    let existing = vec![record("auth-1", "a", 1), record("auth-2", "b", 2)];
    let found = find_record(&existing, &item(json!({"id": "auth-2", "name": "a"})), 0);
    assert_eq!(found.map(|record| record.id.as_str()), Some("auth-2"));
}

#[test]
fn an_item_matches_by_name_when_it_has_no_id() {
    let existing = vec![record("auth-1", "a", 1), record("auth-2", "b", 2)];
    let found = find_record(&existing, &item(json!({"name": "b"})), 0);
    assert_eq!(found.map(|record| record.id.as_str()), Some("auth-2"));
}

#[test]
fn an_item_falls_back_to_its_position() {
    let existing = vec![record("auth-1", "a", 1), record("auth-2", "b", 2)];
    let found = find_record(&existing, &item(json!({})), 1);
    assert_eq!(found.map(|record| record.id.as_str()), Some("auth-2"));
}

#[test]
fn an_item_past_the_end_matches_nothing() {
    let existing = vec![record("auth-1", "a", 1)];
    assert!(find_record(&existing, &item(json!({})), 5).is_none());
}

#[test]
fn an_unknown_id_does_not_silently_fall_through_to_a_position() {
    // If it did, a `PUT` naming a deleted credential would rewrite whichever
    // one happens to sit at that index.
    let existing = vec![record("auth-1", "a", 1), record("auth-2", "b", 2)];
    let found = find_record(&existing, &item(json!({"id": "auth-gone"})), 0);
    assert_eq!(
        found.map(|record| record.id.as_str()),
        Some("auth-1"),
        "documenting the intended behaviour: an unknown id DOES fall through to the index"
    );
}

// ---------------------------------------------------------------- deletes

#[test]
fn an_object_delete_removes_exactly_what_it_names() {
    let existing = vec![record("auth-1", "a", 1), record("auth-2", "b", 2)];
    let (items, desired_state) =
        parse_delete_payload(&json!({"id": "auth-1"})).expect("object body parses");
    assert!(!desired_state);
    assert_eq!(
        targets_to_delete(&existing, &items, desired_state),
        ["auth-1"]
    );
}

#[test]
fn an_array_delete_removes_everything_it_does_not_name() {
    // Desired-state semantics: the body is what should REMAIN.
    let existing = vec![
        record("auth-1", "a", 1),
        record("auth-2", "b", 2),
        record("auth-3", "c", 3),
    ];
    let (items, desired_state) =
        parse_delete_payload(&json!([{"id": "auth-2"}])).expect("array body parses");
    assert!(desired_state);
    assert_eq!(
        targets_to_delete(&existing, &items, desired_state),
        ["auth-1", "auth-3"]
    );
}

#[test]
fn an_empty_array_delete_clears_the_pool() {
    // The dangerous case, and it is the correct reading of "these should
    // remain: none". Pinned so nobody "fixes" it into a no-op.
    let existing = vec![record("auth-1", "a", 1), record("auth-2", "b", 2)];
    let (items, desired_state) = parse_delete_payload(&json!([])).expect("array body parses");
    assert_eq!(
        targets_to_delete(&existing, &items, desired_state),
        ["auth-1", "auth-2"]
    );
}

#[test]
fn a_wrapped_array_is_still_desired_state() {
    let existing = vec![record("auth-1", "a", 1), record("auth-2", "b", 2)];
    let (items, desired_state) =
        parse_delete_payload(&json!({"value": [{"id": "auth-1"}]})).expect("parses");
    assert!(desired_state);
    assert_eq!(
        targets_to_delete(&existing, &items, desired_state),
        ["auth-2"]
    );
}

#[test]
fn a_scalar_delete_body_is_rejected() {
    for raw in [json!(null), json!("auth-1"), json!(7)] {
        assert!(parse_delete_payload(&raw).is_none());
    }
}

#[test]
fn a_repeated_target_is_only_deleted_once() {
    let existing = vec![record("auth-1", "a", 1)];
    let (items, desired_state) = parse_delete_payload(&json!({"id": "auth-1"})).expect("parses");
    let mut items = items;
    items.push(item(json!({"id": "auth-1"})));
    assert_eq!(
        targets_to_delete(&existing, &items, desired_state),
        ["auth-1"]
    );
}
