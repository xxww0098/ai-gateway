use super::*;

// 「一次申请只有一个处置」那条连库测（对应既有连库测 TestRefundPersistedAndSingleDisposition）
// 在 tests/panel/。这里测响应形状与请求绑定。

fn record(status: &str, processed: bool) -> RefundRecord {
    RefundRecord {
        id: 1,
        user_id: 2,
        subscription_id: 3,
        amount: 0.0,
        reason: "太贵了".into(),
        status: status.into(),
        days_used: 0,
        total_days: 0,
        daily_rate: 0.0,
        processed_at: processed.then(Utc::now),
        processed_by: processed.then_some(9),
        created_at: Utc::now(),
    }
}

#[test]
fn refund_record_keeps_processed_keys_as_null_while_pending() {
    // 旧实现用的是 `*time.Time` / `*uint` 且**没有** omitempty：待处理时两个键在、值为
    // null。前端按 `processed_at == null` 判断"还没处理"，键消失会让它读到
    // undefined。
    let json = serde_json::to_value(record(STATUS_PENDING, false)).expect("serialise");
    assert_eq!(json["processed_at"], serde_json::Value::Null);
    assert_eq!(json["processed_by"], serde_json::Value::Null);
}

#[test]
fn refund_record_fills_processed_keys_after_a_disposition() {
    let json = serde_json::to_value(record(STATUS_APPROVED, true)).expect("serialise");
    assert!(!json["processed_at"].is_null());
    assert_eq!(json["processed_by"], serde_json::json!(9));
}

#[test]
fn refund_record_has_the_twelve_keys_the_refunds_page_reads() {
    let json = serde_json::to_value(record(STATUS_PENDING, false)).expect("serialise");
    let mut keys: Vec<_> = json
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "amount",
            "created_at",
            "daily_rate",
            "days_used",
            "id",
            "processed_at",
            "processed_by",
            "reason",
            "status",
            "subscription_id",
            "total_days",
            "user_id",
        ]
    );
}

#[test]
fn a_fresh_application_carries_no_computed_amount() {
    // 申请时 amount / days_used / total_days / daily_rate 全是 0：按比例退款的
    // 计算是人工的。移植时"顺手算一下"会凭空造出一个对外承诺的退款额。
    let fresh = record(STATUS_PENDING, false);
    assert_eq!(fresh.amount, 0.0);
    assert_eq!(fresh.days_used, 0);
    assert_eq!(fresh.total_days, 0);
    assert_eq!(fresh.daily_rate, 0.0);
}

#[test]
fn the_two_terminal_statuses_are_distinct_and_neither_is_pending() {
    // 状态机只有三个值，且终态不能和初态撞名 —— 否则条件 UPDATE 的幂等性失效，
    // 同一张单能被批两次。
    assert_ne!(STATUS_APPROVED, STATUS_REJECTED);
    assert_ne!(STATUS_APPROVED, STATUS_PENDING);
    assert_ne!(STATUS_REJECTED, STATUS_PENDING);
}

#[test]
fn apply_request_defaults_to_a_rejectable_subscription_id() {
    let req: ApplyRequest = serde_json::from_str("{}").expect("parse");
    assert_eq!(req.subscription_id, 0);
    assert!(req.reason.is_empty());
}

#[test]
fn apply_request_binds_the_snake_case_keys_the_frontend_sends() {
    // 与 admin_create 那两处不同，旧实现在这里写了 json tag，所以下划线拼写能绑上。
    let req: ApplyRequest =
        serde_json::from_str(r#"{"subscription_id":5,"reason":" 不用了 "}"#).expect("parse");
    assert_eq!(req.subscription_id, 5);
    assert_eq!(req.reason.trim(), "不用了");
}
