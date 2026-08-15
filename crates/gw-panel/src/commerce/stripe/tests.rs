use super::*;

const SECRET: &str = "whsec_test_key";

/// 按 Stripe 的方案造一个合法签名头，好让下面的用例有"正确答案"可比。
/// 注意它**不是**从被测代码抄来的：它独立地实现了 `HMAC(secret, "t.payload")`。
fn sign(payload: &[u8], timestamp: i64, secret: &str) -> String {
    let mut mac = <Hmac<Sha256>>::new_from_slice(secret.as_bytes()).expect("key");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(payload);
    format!(
        "t={timestamp},v1={}",
        hex::encode(mac.finalize().into_bytes())
    )
}

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).expect("timestamp")
}

#[test]
fn a_correctly_signed_payload_verifies() {
    let payload = br#"{"type":"payment_intent.succeeded"}"#;
    let now = 1_700_000_000;
    let header = sign(payload, now, SECRET);
    assert_eq!(
        verify_signature(
            payload,
            &header,
            SECRET,
            SIGNATURE_TOLERANCE_SECONDS,
            at(now)
        ),
        Ok(())
    );
}

#[test]
fn a_payload_tampered_after_signing_is_rejected() {
    // 这是整个端点存在的理由：签名覆盖 body，改一个字节就失效。
    let now = 1_700_000_000;
    let header = sign(br#"{"amount":100}"#, now, SECRET);
    let tampered = br#"{"amount":900}"#;
    assert_eq!(
        verify_signature(
            tampered,
            &header,
            SECRET,
            SIGNATURE_TOLERANCE_SECONDS,
            at(now)
        ),
        Err(SignatureError::NoMatch)
    );
}

#[test]
fn a_signature_from_another_secret_is_rejected() {
    let payload = b"{}";
    let now = 1_700_000_000;
    let header = sign(payload, now, "whsec_someone_elses_key");
    assert_eq!(
        verify_signature(
            payload,
            &header,
            SECRET,
            SIGNATURE_TOLERANCE_SECONDS,
            at(now)
        ),
        Err(SignatureError::NoMatch)
    );
}

#[test]
fn the_timestamp_is_part_of_the_signed_material() {
    // 只改 t= 而不重新签名 → 必须失败。否则重放时把时间戳改到当下就能绕过容差。
    let payload = b"{}";
    let now = 1_700_000_000;
    let header = sign(payload, now, SECRET);
    let moved = header.replacen(&format!("t={now}"), &format!("t={}", now + 1), 1);
    assert_eq!(
        verify_signature(
            payload,
            &moved,
            SECRET,
            SIGNATURE_TOLERANCE_SECONDS,
            at(now + 1)
        ),
        Err(SignatureError::NoMatch)
    );
}

#[test]
fn stale_and_future_timestamps_are_rejected_symmetrically() {
    // 容差是双向的：过旧是重放，过新是时钟被拨动过。
    let payload = b"{}";
    let signed_at = 1_700_000_000;
    let header = sign(payload, signed_at, SECRET);
    let tolerance = SIGNATURE_TOLERANCE_SECONDS;

    for skew in [tolerance + 1, -(tolerance + 1)] {
        assert_eq!(
            verify_signature(payload, &header, SECRET, tolerance, at(signed_at + skew)),
            Err(SignatureError::OutsideTolerance),
            "skew={skew}"
        );
    }
    // 恰好落在容差边界上仍然放行（用的是严格大于）。
    for skew in [tolerance, -tolerance, 0] {
        assert_eq!(
            verify_signature(payload, &header, SECRET, tolerance, at(signed_at + skew)),
            Ok(()),
            "skew={skew}"
        );
    }
}

#[test]
fn tolerance_can_be_disabled_for_replay_agnostic_callers() {
    let payload = b"{}";
    let signed_at = 1_700_000_000;
    let header = sign(payload, signed_at, SECRET);
    assert_eq!(
        verify_signature(payload, &header, SECRET, 0, at(signed_at + 86_400)),
        Ok(())
    );
}

#[test]
fn any_one_of_several_v1_candidates_is_enough() {
    // 密钥轮换期间 Stripe 会同时发多个 v1。只要有一个对上就通过。
    let payload = b"{}";
    let now = 1_700_000_000;
    let good = sign(payload, now, SECRET);
    let good_hex = good.split("v1=").nth(1).expect("v1").to_owned();
    let header = format!("t={now},v1=deadbeef,v1={good_hex}");
    assert_eq!(
        verify_signature(
            payload,
            &header,
            SECRET,
            SIGNATURE_TOLERANCE_SECONDS,
            at(now)
        ),
        Ok(())
    );
}

#[test]
fn malformed_headers_are_reported_as_malformed_not_as_a_mismatch() {
    let payload = b"{}";
    let now = at(1_700_000_000);
    for header in ["", "garbage", "v1=abcd", "t=", "t=123", "t=123,v2=abcd"] {
        let verdict = verify_signature(payload, header, SECRET, 0, now);
        assert!(
            matches!(
                verdict,
                Err(SignatureError::Malformed | SignatureError::BadTimestamp)
            ),
            "header={header:?} -> {verdict:?}"
        );
    }
}

#[test]
fn a_non_numeric_timestamp_is_a_bad_timestamp() {
    assert_eq!(
        verify_signature(b"{}", "t=not-a-number,v1=abcd", SECRET, 0, at(0)),
        Err(SignatureError::BadTimestamp)
    );
}

#[test]
fn non_hex_candidates_are_skipped_rather_than_aborting_the_scan() {
    // 一个格式不对的 v1 不该让后面那个正确的失去机会。
    let payload = b"{}";
    let now = 1_700_000_000;
    let good_hex = sign(payload, now, SECRET)
        .split("v1=")
        .nth(1)
        .expect("v1")
        .to_owned();
    let header = format!("t={now},v1=zzzz,v1={good_hex}");
    assert_eq!(
        verify_signature(payload, &header, SECRET, 0, at(now)),
        Ok(())
    );
}

// ── 事件解析 ─────────────────────────────────────────────────────────────────

#[test]
fn event_parsing_ignores_the_rest_of_stripes_envelope() {
    // Stripe 的事件体又大又会演进；我们只认三个字段，多出来的一律无视。
    let body = r#"{
        "id":"evt_1","object":"event","api_version":"2020-08-27",
        "type":"payment_intent.succeeded",
        "data":{"object":{"id":"pi_1","amount":1000,"metadata":{"order_id":"42"}}},
        "livemode":false
    }"#;
    let event: StripeEvent = serde_json::from_str(body).expect("parse");
    assert_eq!(event.kind, "payment_intent.succeeded");
    assert_eq!(
        event
            .data
            .object
            .metadata
            .get("order_id")
            .map(String::as_str),
        Some("42")
    );
}

#[test]
fn event_parsing_survives_a_missing_data_object() {
    // 结算无关的事件常常没有我们要的那层结构；解析不能因此失败，否则会 400 掉
    // 一个本该被 ack 的通知，Stripe 会一直重投。
    let event: StripeEvent = serde_json::from_str(r#"{"type":"charge.refunded"}"#).expect("parse");
    assert_eq!(event.kind, "charge.refunded");
    assert!(event.data.object.metadata.is_empty());
}

#[test]
fn only_terminal_success_events_trigger_settlement() {
    for kind in SETTLING_EVENTS {
        assert!(SETTLING_EVENTS.contains(&kind));
    }
    for kind in [
        "payment_intent.created",
        "payment_intent.payment_failed",
        "charge.refunded",
        "",
    ] {
        assert!(!SETTLING_EVENTS.contains(&kind), "kind={kind}");
    }
}
