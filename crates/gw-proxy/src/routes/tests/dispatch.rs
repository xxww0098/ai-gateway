//! Upstream dispatch for the `/v1` dialects: model-to-provider resolution,
//! cross-account failover, the streaming relay, and the middleware order the
//! whole billing pipeline depends on.

use super::*;

// ---------------------------------------------------------------- routing

#[test]
fn a_recognisable_model_outranks_the_endpoint_it_arrived_on() {
    // A caller may reach any dialect endpoint with any model.
    assert_eq!(
        provider_candidates("claude-sonnet-5", ApiFamily::OpenAi),
        vec!["claude", "openai"],
    );
    assert_eq!(
        provider_candidates("gpt-4o", ApiFamily::Claude),
        vec!["openai", "claude"]
    );
}

#[test]
fn a_gemini_model_falls_back_to_vertex_because_both_serve_it() {
    assert_eq!(
        provider_candidates("gemini-2.5-pro", ApiFamily::Gemini),
        vec!["gemini", "vertex"],
    );
}

#[test]
fn an_unrecognised_model_still_reaches_the_endpoint_default() {
    assert_eq!(
        provider_candidates("mystery", ApiFamily::Claude),
        vec!["claude"]
    );
    assert_eq!(provider_candidates("", ApiFamily::OpenAi), vec!["openai"]);
}

#[test]
fn the_gemini_action_suffix_is_split_off_the_model() {
    assert_eq!(
        split_model_action("gemini-2.5-pro:streamGenerateContent"),
        (
            "gemini-2.5-pro".to_owned(),
            "streamGenerateContent".to_owned()
        ),
    );
    assert_eq!(
        split_model_action("gemini-2.5-pro"),
        ("gemini-2.5-pro".to_owned(), String::new()),
    );
}

#[test]
fn query_pairs_keep_their_order_and_duplicates() {
    // Both are significant upstream, which is why this is a Vec and not a map.
    assert_eq!(
        parse_query("alt=sse&key=a&key=b"),
        vec![
            ("alt".to_owned(), "sse".to_owned()),
            ("key".to_owned(), "a".to_owned()),
            ("key".to_owned(), "b".to_owned()),
        ],
    );
    assert_eq!(
        parse_query("flag"),
        vec![("flag".to_owned(), String::new())]
    );
    assert!(parse_query("").is_empty());
}

#[test]
fn only_failures_another_account_could_survive_are_retried() {
    assert!(is_retryable(&ProviderError::Upstream {
        status: 503,
        body: String::new()
    }));
    assert!(is_retryable(&ProviderError::Upstream {
        status: 429,
        body: String::new()
    }));
    assert!(is_retryable(&ProviderError::Credential("expired".into())));
    assert!(
        !is_retryable(&ProviderError::Upstream {
            status: 400,
            body: String::new()
        }),
        "a malformed request fails identically on every account",
    );
}

#[test]
fn hop_by_hop_headers_are_not_relayed() {
    assert!(is_hop_by_hop("connection"));
    assert!(is_hop_by_hop("transfer-encoding"));
    assert!(
        is_hop_by_hop("content-length"),
        "relaying a stale length would truncate the body we actually send",
    );
    assert!(!is_hop_by_hop("content-type"));
}

// ---------------------------------------------------------------- ordering

#[tokio::test]
async fn authentication_runs_before_billing_so_an_anonymous_call_costs_nothing() {
    // Blocker B1: with the layers the other way round every /v1 request aborts
    // with a pre-auth 401 and the billing hot path never executes.
    let harness = Harness::build();
    let (status, _) = send(
        harness.router(),
        anonymous_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        harness.ledger.calls().is_empty(),
        "the hold layer must never see an unauthenticated request",
    );
    assert_eq!(
        harness.provider.call_count(),
        0,
        "and no upstream call may be made either",
    );
}

#[tokio::test]
async fn an_authenticated_call_reserves_dispatches_and_settles_in_that_order() {
    let harness = Harness::build();
    harness.provider.queue(Ok(ok_response(100, 250)));

    let (status, _) = send(
        harness.router(),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(harness.provider.call_count(), 1);
    assert!(matches!(
        harness.ledger.calls().first(),
        Some(LedgerCall::Hold { .. })
    ));
    assert_eq!(
        harness.usage_store.settled_costs().len(),
        1,
        "exactly one settlement per request",
    );
}

#[tokio::test]
async fn the_reported_usage_is_what_gets_billed() {
    let harness = Harness::build();
    harness.provider.queue(Ok(ok_response(100, 250)));
    send(
        harness.router(),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    let logs = harness.usage_store.logs.lock();
    assert_eq!(logs[0].input_tokens, 100);
    assert_eq!(logs[0].output_tokens, 250);
    assert!(!logs[0].failed);
}

#[tokio::test]
async fn an_upstream_without_a_usage_envelope_falls_back_instead_of_billing_zero() {
    let harness = Harness::build();
    harness.provider.queue(Ok(ok_response_without_usage()));
    send(
        harness.router(),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    let costs = harness.usage_store.settled_costs();
    assert_eq!(costs.len(), 1);
    assert!(costs[0] > 0.0, "upstream output must never be free");
}

// ---------------------------------------------------------------- failover

#[tokio::test]
async fn a_failing_account_is_retried_on_a_different_one_and_billed_once() {
    // This is the invariant the channel selector calls out: cross-account
    // retry settles once, on the final response.
    let harness = Harness::build_with(vec![
        auth_record("acct-1", "openai"),
        auth_record("acct-2", "openai"),
    ]);
    harness.provider.queue(Err(ProviderError::Upstream {
        status: 503,
        body: "overloaded".to_owned(),
    }));
    harness.provider.queue(Ok(ok_response(10, 20)));

    let (status, _) = send(
        harness.router(),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        harness.provider.call_count(),
        2,
        "the retry must have happened"
    );
    let tried = harness.provider.seen_auth_ids.lock().clone();
    assert_ne!(tried[0], tried[1], "the retry must use a different account");
    assert_eq!(
        harness.usage_store.settled_costs().len(),
        1,
        "failover must not double-bill",
    );
}

#[tokio::test]
async fn a_client_error_is_surfaced_immediately_instead_of_burning_the_pool() {
    let harness = Harness::build_with(vec![
        auth_record("acct-1", "openai"),
        auth_record("acct-2", "openai"),
    ]);
    harness.provider.queue(Err(ProviderError::Upstream {
        status: 400,
        body: r#"{"error":"bad request"}"#.to_owned(),
    }));

    let (status, _) = send(
        harness.router(),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(harness.provider.call_count(), 1);
}

#[tokio::test]
async fn a_failed_dispatch_releases_the_reservation() {
    let harness = Harness::build();
    harness.provider.queue(Err(ProviderError::Upstream {
        status: 400,
        body: String::new(),
    }));

    send(
        harness.router(),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert!(
        harness
            .ledger
            .calls()
            .iter()
            .any(|c| matches!(c, LedgerCall::Release { .. })),
        "a request that never reached an upstream must not be charged",
    );
    assert!(harness.usage_store.settled_costs().is_empty());
}

#[tokio::test]
async fn an_empty_credential_pool_reports_unavailability_and_charges_nothing() {
    let harness = Harness::build_with(vec![]);
    let (status, _) = send(
        harness.router(),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(harness.usage_store.settled_costs().is_empty());
    assert!(
        harness
            .ledger
            .calls()
            .iter()
            .any(|c| matches!(c, LedgerCall::Release { .. })),
    );
}

#[tokio::test]
async fn an_account_that_keeps_failing_across_requests_is_benched_from_the_pool() {
    // Health is tracked across requests, not within one: a single client call
    // only ever tries a given account once.
    let harness = Harness::build_with(vec![auth_record("acct-1", "openai")]);
    for _ in 0..crate::channel::DEFAULT_FAILURE_THRESHOLD {
        harness.provider.queue(Err(ProviderError::Upstream {
            status: 503,
            body: String::new(),
        }));
        let (status, _) = send(
            harness.router(),
            signed_request("/v1/chat/completions", chat_body("gpt-4o")),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    assert!(
        harness.health.benched_count() > 0,
        "consecutive failures must take the account out of rotation",
    );
    assert!(
        harness
            .breaker
            .recorded
            .lock()
            .iter()
            .any(|(_, success)| !success),
        "and must be reported to the circuit breaker",
    );
}

// ---------------------------------------------------------------- free endpoints

#[tokio::test]
async fn listing_models_is_billed_the_fallback_estimate() {
    let harness = Harness::build();
    let request = axum::http::Request::builder()
        .uri("/v1/models")
        .header("authorization", format!("Bearer {TEST_API_KEY}"))
        .body(axum::body::Body::empty())
        .expect("request builds");

    let (status, body) = send(harness.router(), request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["object"].as_str(), Some("list"));
    let listed: Vec<&str> = body["data"]
        .as_array()
        .expect("a data array")
        .iter()
        .filter_map(|m| m["id"].as_str())
        .collect();
    let catalogued: Vec<String> = harness
        .catalog
        .models
        .lock()
        .iter()
        .map(|m| m.id.clone())
        .collect();
    assert_eq!(listed, catalogued, "the catalogue is served verbatim");
    // The `/v1/` prefix alone drives the reservation and then, finding no
    // usage envelope, the fallback estimate settles. Reproduced deliberately;
    // see `hold::is_billable`.
    assert!(
        matches!(
            harness.ledger.calls().first(),
            Some(LedgerCall::Hold { .. })
        ),
        "a catalogue read reserves, because the prefix is the only gate",
    );
    let charged = harness.usage_store.settled_costs();
    assert_eq!(charged.len(), 1);
    assert!(
        charged[0] > 0.0,
        "and the fallback settle charges for it — the behaviour flagged for a \
         product decision, not a porting one",
    );
}

#[tokio::test]
async fn counting_tokens_is_billed() {
    let harness = Harness::build_with(vec![auth_record("acct-1", "claude")]);
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/messages/count_tokens")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TEST_API_KEY}"))
        .body(axum::body::Body::from(
            chat_body("claude-sonnet-5").to_string(),
        ))
        .expect("request builds");

    let (_, _) = send(harness.router(), request).await;

    // Anthropic bills nothing for token counting, and its reply carries no
    // `usage` wrapper, so the usage parser reports absent and the fallback
    // settle charges the streaming estimate at the model's real rate.
    assert!(matches!(
        harness.ledger.calls().first(),
        Some(LedgerCall::Hold { .. })
    ),);
    assert_eq!(harness.usage_store.settled_costs().len(), 1);
}

#[tokio::test]
async fn this_router_can_be_merged_with_one_that_owns_the_metrics_endpoint() {
    // `/metrics/prometheus` belongs to the composition root. Registering it
    // here too makes `Router::merge` panic on the duplicate and
    // the process never finishes booting — so the guard is that the merge is
    // simply possible.
    let harness = Harness::build();
    let host: axum::Router = axum::Router::new().route(
        "/metrics/prometheus",
        axum::routing::get(|| async { "cpa_v1_requests_total 0" }),
    );

    let merged = host.merge(harness.router());

    let request = axum::http::Request::builder()
        .uri("/metrics/prometheus")
        .body(axum::body::Body::empty())
        .expect("request builds");
    use tower::ServiceExt;
    let response = merged.oneshot(request).await.expect("responds");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the host's metrics route must survive the merge",
    );
}

#[tokio::test]
async fn the_gauges_this_crate_observes_are_pushed_to_the_host_not_exported_here() {
    // The benched count is read at scrape time; the scrape lives in another
    // crate now, so the value travels through the sink instead.
    let harness = Harness::build();
    harness.health.record_result("acct-1", false, None);
    harness.health.record_result("acct-1", false, None);
    harness.health.record_result("acct-1", false, None);

    harness.state.publish_gauges();

    assert_eq!(
        harness.metrics.benched(),
        harness.health.benched_count(),
        "the gauge must reflect what the pool actually benched",
    );
}

// ---------------------------------------------------------------- streaming

fn stream_body(model: &str) -> serde_json::Value {
    let mut body = chat_body(model);
    body["stream"] = serde_json::json!(true);
    body
}

/// Sends a streaming chat request and returns the relayed SSE text.
async fn collect_stream(
    harness: &Harness,
    body: serde_json::Value,
) -> (StatusCode, String, String) {
    collect_sse(harness, signed_request("/v1/chat/completions", body)).await
}

#[tokio::test]
async fn a_streamed_response_relays_its_payload_chunks_verbatim() {
    let harness = Harness::build();
    *harness.provider.stream_chunks.lock() = vec![
        StreamChunk::Payload("data: one\n\n".into()),
        usage_chunk(10, 20),
        StreamChunk::Payload("data: [DONE]\n\n".into()),
    ];

    let (status, content_type, body) = collect_stream(&harness, stream_body("gpt-4o")).await;

    assert_eq!(status, StatusCode::OK);
    assert!(content_type.contains("event-stream"), "got {content_type}");
    assert_eq!(
        body, "data: one\n\ndata: [DONE]\n\n",
        "the usage chunk is billing metadata and must not reach the client",
    );
}

#[tokio::test]
async fn a_stream_settles_once_it_has_finished_and_uses_the_usage_it_carried() {
    let harness = Harness::build();
    *harness.provider.stream_chunks.lock() = vec![
        StreamChunk::Payload("data: one\n\n".into()),
        usage_chunk(11, 22),
    ];

    collect_stream(&harness, stream_body("gpt-4o")).await;

    let logs = harness.usage_store.logs.lock();
    assert_eq!(logs.len(), 1, "a stream must settle exactly once");
    assert_eq!(logs[0].input_tokens, 11);
    assert_eq!(logs[0].output_tokens, 22);
    assert!(logs[0].stream, "the usage row must record that it streamed");
}

#[tokio::test]
async fn a_stream_that_reports_an_error_releases_instead_of_charging() {
    let harness = Harness::build();
    *harness.provider.stream_chunks.lock() = vec![
        StreamChunk::Payload("data: partial\n\n".into()),
        StreamChunk::Error {
            status: Some(500),
            message: "upstream exploded".to_owned(),
        },
    ];

    collect_stream(&harness, stream_body("gpt-4o")).await;

    assert!(
        harness
            .ledger
            .calls()
            .iter()
            .any(|c| matches!(c, LedgerCall::Release { .. })),
    );
    assert!(harness.usage_store.settled_costs().is_empty());
}

#[tokio::test]
async fn a_stream_without_a_usage_chunk_falls_back_rather_than_billing_zero() {
    let harness = Harness::build();
    *harness.provider.stream_chunks.lock() = vec![StreamChunk::Payload("data: one\n\n".into())];

    collect_stream(&harness, stream_body("gpt-4o")).await;

    let costs = harness.usage_store.settled_costs();
    assert_eq!(costs.len(), 1);
    assert!(costs[0] > 0.0);
}

#[tokio::test]
async fn abandoning_a_stream_mid_flight_settles_through_the_shutdown_tracker() {
    // A client that hangs up drops the body without the stream ever ending, so
    // the settlement is detached. It MUST go to the composition root's tracker:
    // a bare `tokio::spawn` is aborted when the runtime is dropped, which turns
    // a disconnect during shutdown into a lost charge and a leaked hold.
    //
    // The deferred finalizer must run on the server even when the client
    // disconnects, so shutdown drains it.
    use tower::ServiceExt;

    let harness = Harness::build();
    *harness.provider.stream_chunks.lock() = vec![
        StreamChunk::Payload("data: one\n\n".into()),
        usage_chunk(5, 5),
        StreamChunk::Payload("data: two\n\n".into()),
    ];

    let response = harness
        .router()
        .oneshot(signed_request(
            "/v1/chat/completions",
            stream_body("gpt-4o"),
        ))
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        harness.usage_store.logs.lock().is_empty(),
        "nothing should have settled while the stream was still live",
    );

    drop(response); // the client goes away without reading a byte

    assert_eq!(
        harness.drain.len(),
        1,
        "the settlement must be registered on the tracker the composition root \
         drains, not spawned loose onto the runtime",
    );

    // Exactly what `gw_server::drain_settlements` does after graceful shutdown.
    harness.drain.close();
    harness.drain.wait().await;

    assert_eq!(
        harness.usage_store.logs.lock().len(),
        1,
        "draining the tracker must have run the settlement to completion",
    );
    assert_eq!(harness.drain.len(), 0);
}

#[tokio::test]
async fn a_disconnect_that_lands_during_the_drain_is_still_waited_on() {
    // `TaskTracker::close` does not block later spawns, which is why
    // `StreamSettler::drop` needs no "already closing?" check: a body dropped
    // after the drain began is still tracked, and `wait()` still covers it.
    use tower::ServiceExt;

    let harness = Harness::build();
    *harness.provider.stream_chunks.lock() = vec![
        StreamChunk::Payload("data: one\n\n".into()),
        usage_chunk(5, 5),
    ];

    let response = harness
        .router()
        .oneshot(signed_request(
            "/v1/chat/completions",
            stream_body("gpt-4o"),
        ))
        .await
        .expect("router responds");

    harness.drain.close(); // shutdown starts while the client is still attached
    drop(response); // ...and only then does the client go away
    harness.drain.wait().await;

    assert_eq!(
        harness.usage_store.logs.lock().len(),
        1,
        "a settlement spawned after close() must still be drained",
    );
}

#[tokio::test]
async fn a_stream_that_runs_to_completion_settles_inline_and_leaves_the_tracker_empty() {
    // The tracker is the disconnect path only. A stream the client actually
    // read has already settled by the time the body ends, so nothing detaches.
    let harness = Harness::build();
    *harness.provider.stream_chunks.lock() = vec![
        StreamChunk::Payload("data: one\n\n".into()),
        usage_chunk(5, 5),
    ];

    collect_stream(&harness, stream_body("gpt-4o")).await;

    assert_eq!(harness.usage_store.logs.lock().len(), 1);
    assert_eq!(
        harness.drain.len(),
        0,
        "an inline settlement must not also queue a detached one",
    );
}

#[tokio::test]
async fn an_error_status_relayed_in_band_fails_over_like_a_raised_one() {
    // Some providers surface upstream 5xx as a normal response; the client
    // should not get a 503 that a different credential would have served.
    let harness = Harness::build_with(vec![
        auth_record("acct-1", "openai"),
        auth_record("acct-2", "openai"),
    ]);
    harness.provider.queue(Ok(ProviderResponse {
        status: 503,
        headers: http::HeaderMap::new(),
        body: b"overloaded".to_vec(),
        usage: None,
    }));
    harness.provider.queue(Ok(ok_response(10, 20)));

    let (status, _) = send(
        harness.router(),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(harness.provider.call_count(), 2);
    assert_eq!(harness.usage_store.settled_costs().len(), 1);
}

#[test]
fn only_account_level_statuses_are_worth_another_credential() {
    assert!(is_retryable_status(500));
    assert!(is_retryable_status(429));
    assert!(!is_retryable_status(400));
    assert!(!is_retryable_status(200));
}
