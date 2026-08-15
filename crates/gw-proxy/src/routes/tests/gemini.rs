//! The Gemini native surface: the `/v1beta` routes, the credential carriers
//! Google's own SDKs use, and the proof that neither is a way around billing.

use super::*;

// ------------------------------------------------ gemini native (`/v1beta`)

/// The prefix the dashboard hands tenants as their Gemini endpoint
/// (`QuickIntegrationPanel.tsx`, frozen). It is a sibling of `/v1`, not a child,
/// because that is how Google versions its Generative Language API.
const GEMINI_GENERATE: &str = "/v1beta/models/gemini-2.5-pro:generateContent";
const GEMINI_STREAM: &str = "/v1beta/models/gemini-2.5-pro:streamGenerateContent";

/// A Gemini `generateContent` payload. Note what it does *not* carry: a `model`
/// field or a `stream` field. Both live in the URL, so a route that reads them
/// off the body alone would dispatch an unnamed model non-streaming.
fn gemini_body() -> serde_json::Value {
    serde_json::json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}]})
}

fn gemini_harness() -> Harness {
    Harness::build_with(vec![auth_record("acct-1", "gemini")])
}

#[tokio::test]
async fn the_gemini_dialect_answers_on_the_prefix_the_dashboard_advertises() {
    let harness = gemini_harness();
    harness.gemini.queue(Ok(ok_response(100, 250)));

    let (status, _) = send(
        harness.router(),
        signed_request(GEMINI_GENERATE, gemini_body()),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "a tenant following the panel's instructions must not get a 404",
    );
    assert_eq!(harness.gemini.call_count(), 1);
    assert_eq!(
        harness.gemini.dispatched(),
        vec![("gemini-2.5-pro".to_owned(), false)],
        "the model must come off the URL, and the plain action must not stream",
    );
}

#[tokio::test]
async fn a_gemini_call_reserves_and_settles_on_the_same_pipeline_as_v1() {
    // This is a paid proxy endpoint: reaching it through a different prefix must
    // not be a way around the hold/settle pipeline.
    let harness = gemini_harness();
    harness.gemini.queue(Ok(ok_response(100, 250)));

    send(
        harness.router(),
        signed_request(GEMINI_GENERATE, gemini_body()),
    )
    .await;

    assert!(
        matches!(
            harness.ledger.calls().first(),
            Some(LedgerCall::Hold { .. })
        ),
        "the reservation must happen before dispatch",
    );
    let logs = harness.usage_store.logs.lock();
    assert_eq!(logs.len(), 1, "exactly one settlement per request");
    assert_eq!(logs[0].input_tokens, 100);
    assert_eq!(logs[0].output_tokens, 250);
    assert!(!logs[0].failed);
}

#[tokio::test]
async fn the_stream_action_streams_although_the_body_never_asked_for_it() {
    let harness = gemini_harness();
    *harness.gemini.stream_chunks.lock() = vec![
        StreamChunk::Payload("data: {\"candidates\":[]}\n\n".into()),
        usage_chunk(11, 22),
    ];

    let (status, content_type, body) =
        collect_sse(&harness, signed_request(GEMINI_STREAM, gemini_body())).await;

    assert_eq!(status, StatusCode::OK);
    assert!(content_type.contains("event-stream"), "got {content_type}");
    assert_eq!(
        body, "data: {\"candidates\":[]}\n\n",
        "the usage chunk is billing metadata and must not reach the client",
    );
    assert_eq!(
        harness.gemini.dispatched(),
        vec![("gemini-2.5-pro".to_owned(), true)],
        "the action suffix is what puts the provider in streaming mode — and \
         only a streaming provider request forces gemini's alt=sse framing",
    );
}

#[tokio::test]
async fn a_gemini_stream_settles_once_from_the_usage_it_carried() {
    let harness = gemini_harness();
    *harness.gemini.stream_chunks.lock() = vec![
        StreamChunk::Payload("data: x\n\n".into()),
        usage_chunk(11, 22),
    ];

    collect_sse(&harness, signed_request(GEMINI_STREAM, gemini_body())).await;

    let logs = harness.usage_store.logs.lock();
    assert_eq!(logs.len(), 1, "a stream must settle exactly once");
    assert_eq!(logs[0].input_tokens, 11);
    assert_eq!(logs[0].output_tokens, 22);
}

#[tokio::test]
async fn a_gemini_stream_without_usage_falls_back_rather_than_billing_zero() {
    let harness = gemini_harness();
    *harness.gemini.stream_chunks.lock() = vec![StreamChunk::Payload("data: x\n\n".into())];

    collect_sse(&harness, signed_request(GEMINI_STREAM, gemini_body())).await;

    let costs = harness.usage_store.settled_costs();
    assert_eq!(costs.len(), 1);
    assert!(costs[0] > 0.0, "upstream output must never be free");
}

#[tokio::test]
async fn an_anonymous_gemini_call_is_turned_away_before_it_can_reserve() {
    let harness = gemini_harness();

    for path in [GEMINI_GENERATE, GEMINI_STREAM] {
        let (status, _) = send(harness.router(), anonymous_request(path, gemini_body())).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{path} was served anonymously"
        );
    }

    assert!(
        harness.ledger.calls().is_empty(),
        "the hold layer must never see an unauthenticated request",
    );
    assert_eq!(harness.gemini.call_count(), 0);
}

#[tokio::test]
async fn the_gemini_catalogue_needs_credentials_too() {
    let harness = Harness::build();
    let request = axum::http::Request::builder()
        .uri("/v1beta/models")
        .body(axum::body::Body::empty())
        .expect("request builds");

    let (status, _) = send(harness.router(), request).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(harness.ledger.calls().is_empty());
}

#[tokio::test]
async fn the_gemini_catalogue_answers_in_googles_envelope_not_openais() {
    let harness = Harness::build();

    let (status, body) = send(harness.router(), signed_get("/v1beta/models")).await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        body["data"].is_null() && body["object"].is_null(),
        "the OpenAI envelope must not leak onto the gemini surface: {body}",
    );
    // Google keys a model by its resource name and every client strips the
    // prefix back off — the frontend's own `stripGeminiModelResourceName` does
    // exactly this against upstream Google.
    let served: Vec<String> = body["models"]
        .as_array()
        .expect("a models array")
        .iter()
        .map(|m| {
            let name = m["name"].as_str().expect("every entry is named");
            let id = name.strip_prefix("models/").unwrap_or_else(|| {
                panic!("{name} is not a resource name, so no gemini client will match it")
            });
            id.to_owned()
        })
        .collect();
    let catalogued: Vec<String> = harness
        .catalog
        .models
        .lock()
        .iter()
        .map(|m| m.id.clone())
        .collect();
    assert_eq!(served, catalogued, "the catalogue is served verbatim");
}

#[tokio::test]
async fn one_gemini_catalogue_entry_is_addressable_by_its_bare_id() {
    let harness = Harness::build();
    harness
        .catalog
        .models
        .lock()
        .push(crate::ports::ModelEntry {
            id: "gemini-2.5-pro".to_owned(),
            created: 0,
            owned_by: "google".to_owned(),
        });

    let (status, body) = send(
        harness.router(),
        signed_get("/v1beta/models/gemini-2.5-pro"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"].as_str(), Some("models/gemini-2.5-pro"));
    assert_eq!(body["baseModelId"].as_str(), Some("gemini-2.5-pro"));

    let (missing, _) = send(harness.router(), signed_get("/v1beta/models/not-a-model")).await;
    assert_eq!(missing, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_gemini_catalogue_is_billed_exactly_as_the_openai_one_is() {
    // The prefix alone drives the reservation and then, finding no usage
    // envelope, the fallback estimate settles. `/v1beta` inherits that
    // verbatim; see `hold::is_billable`.
    let harness = Harness::build();

    send(harness.router(), signed_get("/v1beta/models")).await;

    assert!(matches!(
        harness.ledger.calls().first(),
        Some(LedgerCall::Hold { .. })
    ));
    let charged = harness.usage_store.settled_costs();
    assert_eq!(charged.len(), 1);
    assert!(charged[0] > 0.0);
}

// --------------------------------- gemini credential carriers (`/v1beta`)

/// Builds a `POST` that authenticates the way a Gemini client actually does:
/// with `header_name`, and never with `Authorization`.
fn gemini_request_with_header(
    path: &str,
    header_name: &str,
) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header(header_name, TEST_API_KEY)
        .body(axum::body::Body::from(gemini_body().to_string()))
        .expect("request builds")
}

#[tokio::test]
async fn a_gemini_client_authenticates_the_way_google_sdks_actually_do() {
    // No stock Gemini client sets `Authorization`, so accepting only that turns
    // the 404 this surface used to return into a 401 — no better for the tenant
    // following the dashboard's instructions.
    for header in ["x-goog-api-key", "x-api-key"] {
        let harness = gemini_harness();
        let (status, _) = send(
            harness.router(),
            gemini_request_with_header(GEMINI_GENERATE, header),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{header} was not accepted");
        assert_eq!(
            harness.usage_store.settled_costs().len(),
            1,
            "{header} must reach the same billing pipeline, not bypass it",
        );
    }
}

#[tokio::test]
async fn the_key_query_parameter_authenticates_and_is_billed_like_any_other() {
    let harness = gemini_harness();
    let request = axum::http::Request::builder()
        .method("POST")
        .uri(format!("{GEMINI_GENERATE}?key={TEST_API_KEY}"))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(gemini_body().to_string()))
        .expect("request builds");

    let (status, _) = send(harness.router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert!(matches!(
        harness.ledger.calls().first(),
        Some(LedgerCall::Hold { .. })
    ));
    assert_eq!(harness.usage_store.settled_costs().len(), 1);
}

#[tokio::test]
async fn a_bad_credential_is_rejected_whichever_carrier_it_arrives_on() {
    let harness = gemini_harness();

    let mut requests = vec![
        axum::http::Request::builder()
            .method("POST")
            .uri(format!("{GEMINI_GENERATE}?key=cpa-wrong"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(gemini_body().to_string()))
            .expect("request builds"),
    ];
    for header in ["authorization", "x-goog-api-key", "x-api-key"] {
        let value = if header == "authorization" {
            "Bearer cpa-wrong"
        } else {
            "cpa-wrong"
        };
        requests.push(
            axum::http::Request::builder()
                .method("POST")
                .uri(GEMINI_GENERATE)
                .header("content-type", "application/json")
                .header(header, value)
                .body(axum::body::Body::from(gemini_body().to_string()))
                .expect("request builds"),
        );
    }

    for request in requests {
        let carrier = format!("{:?}", request.headers());
        let (status, _) = send(harness.router(), request).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "accepted a bad key via {carrier}"
        );
    }
    assert!(
        harness.ledger.calls().is_empty(),
        "a rejected key must never reserve"
    );
    assert_eq!(harness.gemini.call_count(), 0);
}

#[tokio::test]
async fn the_authorization_header_outranks_the_carriers_google_uses() {
    // A request carrying several must have one defined outcome, so the order is
    // fixed rather than incidental: Authorization first.
    let harness = gemini_harness();
    let request = axum::http::Request::builder()
        .method("POST")
        .uri(format!("{GEMINI_GENERATE}?key=cpa-wrong"))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TEST_API_KEY}"))
        .header("x-goog-api-key", "cpa-wrong")
        .body(axum::body::Body::from(gemini_body().to_string()))
        .expect("request builds");

    let (status, _) = send(harness.router(), request).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "the highest-priority carrier must win"
    );
}

#[tokio::test]
async fn a_consumed_credential_is_never_relayed_to_google() {
    // The upstream credential comes from the account pool. Forwarding the
    // carriers would hand a CPA tenant key to Google — `copy_outbound_headers`
    // drops only `Authorization`, and the gemini executor overwrites
    // `x-goog-api-key` only when an upstream api key is configured.
    let harness = gemini_harness();
    let request = axum::http::Request::builder()
        .method("POST")
        .uri(format!("{GEMINI_GENERATE}?alt=sse&key={TEST_API_KEY}"))
        .header("content-type", "application/json")
        .header("x-goog-api-key", TEST_API_KEY)
        .header("x-api-key", TEST_API_KEY)
        .body(axum::body::Body::from(gemini_body().to_string()))
        .expect("request builds");

    let (status, _) = send(harness.router(), request).await;
    assert_eq!(status, StatusCode::OK);

    let forwarded = harness.gemini.only_request();
    for header in ["x-goog-api-key", "x-api-key", "authorization"] {
        assert!(
            forwarded.headers.get(header).is_none(),
            "{header} reached the upstream request",
        );
    }
    assert!(
        !forwarded.query.iter().any(|(name, _)| name == "key"),
        "the tenant key survived in the upstream query: {:?}",
        forwarded.query,
    );
    assert!(
        forwarded
            .query
            .iter()
            .any(|(name, value)| name == "alt" && value == "sse"),
        "stripping a credential must not disturb the caller's own parameters",
    );
}

#[tokio::test]
async fn the_v1_surface_keeps_reading_authorization_and_nothing_else() {
    // `/v1` is implemented by this repo's own code and is the real parity
    // territory: widening its credential sources is exactly what was not asked
    // for. An `x-goog-api-key` there must still be a 401.
    let harness = Harness::build();
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .header("x-goog-api-key", TEST_API_KEY)
        .body(axum::body::Body::from(chat_body("gpt-4o").to_string()))
        .expect("request builds");

    let (status, _) = send(harness.router(), request).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(harness.ledger.calls().is_empty());
}

#[tokio::test]
async fn an_anthropic_key_header_still_reaches_the_claude_upstream_on_v1() {
    // The strip is scoped to the gemini surface for a reason: on `/v1`,
    // `x-api-key` is Anthropic's own credential header and its executor needs
    // to see what the caller sent.
    let harness = Harness::build_with(vec![auth_record("acct-1", "claude")]);
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TEST_API_KEY}"))
        .header("x-api-key", "caller-supplied")
        .body(axum::body::Body::from(
            chat_body("claude-sonnet-5").to_string(),
        ))
        .expect("request builds");

    let (status, _) = send(harness.router(), request).await;
    assert_eq!(status, StatusCode::OK);

    let forwarded = harness.claude.only_request();
    assert_eq!(
        forwarded
            .headers
            .get("x-api-key")
            .map(|v| v.to_str().expect("ascii")),
        Some("caller-supplied"),
    );
}
