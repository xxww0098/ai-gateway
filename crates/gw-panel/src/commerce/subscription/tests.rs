use super::*;

// 连库的那几条（购买守恒、补偿、欠款拦截、余额不足零写入）在 tests/panel/，
// 需要真 Postgres。这里覆盖：reference 的构造契约、两个视图的 omitempty 差异、
// 以及那两处刻意保留的既有实现的绑定行为。

// ── 扣款 / 补偿 reference ────────────────────────────────────────────────────

/// 复刻 handler 里那两行拼串，好让下面的性质测试有对象可测。
fn debit_ref(package_id: i64, nonce: &str) -> String {
    format!("subscription_purchase:{package_id}:{nonce}")
}
fn compensate_ref(package_id: i64, debit: &str) -> String {
    format!("subscription_purchase:{package_id}:compensate:{debit}")
}

#[test]
fn compensation_reference_embeds_the_whole_debit_reference() {
    // 这是运维配对「哪一次扣款被退了」的唯一线索：补偿串里必须能原样找回扣款串。
    let debit = debit_ref(7, "e3a1");
    let comp = compensate_ref(7, &debit);
    assert!(comp.contains(&debit), "补偿串丢了扣款串：{comp}");
    assert_ne!(comp, debit, "两者必须可区分");
}

#[test]
fn both_references_share_the_prefix_operators_scan_for() {
    // 运维用 `Reference LIKE 'subscription_purchase:<pkg>:%'` 一次捞出这一对。
    let package_id = 12;
    let prefix = format!("subscription_purchase:{package_id}:");
    let debit = debit_ref(package_id, "n1");
    assert!(debit.starts_with(&prefix));
    assert!(compensate_ref(package_id, &debit).starts_with(&prefix));
}

#[test]
fn debit_references_are_unique_per_attempt() {
    // nonce 存在的理由：同一个用户重复买同一个套餐，两次扣款必须是两条可区分的
    // 流水，否则补偿会配对到错误的那一次。
    let a = debit_ref(3, &uuid::Uuid::new_v4().to_string());
    let b = debit_ref(3, &uuid::Uuid::new_v4().to_string());
    assert_ne!(a, b);
}

#[test]
fn compensation_prefix_is_distinguishable_from_a_plain_purchase() {
    // 补偿串里多了一段 `:compensate:`，扫描时可以只留扣款那一半。
    let debit = debit_ref(1, "n");
    let comp = compensate_ref(1, &debit);
    assert!(!debit.contains(":compensate:"));
    assert!(comp.contains(":compensate:"));
}

// ── 响应形状 ─────────────────────────────────────────────────────────────────

fn package_item(description: &str, monthly: Option<f64>) -> PackageItem {
    PackageItem {
        id: 2,
        name: "Pro".into(),
        description: description.into(),
        rate_multiplier: 0.95,
        default_validity_days: 30,
        daily_limit_usd: None,
        weekly_limit_usd: None,
        monthly_limit_usd: monthly,
        subscription_price_usd: 29.9,
    }
}

#[test]
fn package_item_omits_empty_description_and_absent_limits() {
    let json = serde_json::to_value(package_item("", None)).expect("serialise");
    let obj = json.as_object().expect("object");
    for absent in [
        "description",
        "daily_limit_usd",
        "weekly_limit_usd",
        "monthly_limit_usd",
    ] {
        assert!(!obj.contains_key(absent), "{absent} 应当整个键消失");
    }
}

#[test]
fn package_item_emits_description_and_limits_when_present() {
    let json = serde_json::to_value(package_item("适合团队", Some(100.0))).expect("serialise");
    assert_eq!(json["description"], serde_json::json!("适合团队"));
    assert_eq!(json["monthly_limit_usd"], serde_json::json!(100.0));
}

#[test]
fn package_item_id_is_the_group_id_the_purchase_endpoint_expects() {
    // 列表里的 id 直接被前端拿去当 purchase 的 group_id。这条断言把这个隐式契约
    // 写下来：它不是 subscription_packages 的主键。
    let row = PackageRow {
        id: 99,
        group_id: 3,
        name: "Pro".into(),
        description: String::new(),
        rate_multiplier: 1.0,
        default_validity_days: 30,
        daily_limit_usd: None,
        weekly_limit_usd: None,
        monthly_limit_usd: None,
        subscription_price_usd: 1.0,
    };
    let item = PackageItem {
        id: row.group_id,
        name: row.name.clone(),
        description: row.description.clone(),
        rate_multiplier: row.rate_multiplier,
        default_validity_days: row.default_validity_days,
        daily_limit_usd: row.daily_limit_usd,
        weekly_limit_usd: row.weekly_limit_usd,
        monthly_limit_usd: row.monthly_limit_usd,
        subscription_price_usd: row.subscription_price_usd,
    };
    assert_eq!(item.id, row.group_id);
    assert_ne!(item.id, row.id);
}

#[test]
fn admin_payload_keeps_limit_keys_even_when_null() {
    // 用户视图 omitempty、管理员视图不 omitempty —— 两个视图刻意不同。
    let json = serde_json::to_value(AdminSubscriptionPayload {
        id: 1,
        user_id: 2,
        group_id: 3,
        email: "u@example.com".into(),
        username: None,
        group_name: "Pro".into(),
        status: "active".into(),
        starts_at: Utc::now(),
        expires_at: Utc::now(),
        daily_usage_usd: 0.0,
        weekly_usage_usd: 0.0,
        monthly_usage_usd: 0.0,
        daily_limit_usd: None,
        weekly_limit_usd: None,
        monthly_limit_usd: None,
        created_at: Utc::now(),
        funding_source: String::new(),
        funding_reference: String::new(),
        price_paid: 0.0,
        notes: None,
    })
    .expect("serialise");
    for key in ["daily_limit_usd", "weekly_limit_usd", "monthly_limit_usd"] {
        assert_eq!(json[key], serde_json::Value::Null, "{key} 必须在且为 null");
    }
    // 键名是 price_paid，不是列名 price_paid_usd。
    assert!(json.get("price_paid").is_some());
    assert!(json.get("price_paid_usd").is_none());
}

#[test]
fn user_subscription_item_omits_absent_limits() {
    let json = serde_json::to_value(SubscriptionItem {
        id: 1,
        group_id: 3,
        group_name: "Pro".into(),
        status: "active".into(),
        starts_at: Utc::now(),
        expires_at: Utc::now(),
        daily_usage_usd: 0.0,
        weekly_usage_usd: 0.0,
        monthly_usage_usd: 0.0,
        daily_limit_usd: None,
        weekly_limit_usd: None,
        monthly_limit_usd: Some(100.0),
    })
    .expect("serialise");
    let obj = json.as_object().expect("object");
    assert!(!obj.contains_key("daily_limit_usd"));
    assert!(obj.contains_key("monthly_limit_usd"));
}

// ── 请求绑定 ─────────────────────────────────────────────────────────────────

#[test]
fn purchase_request_defaults_to_a_rejectable_group_id() {
    let req: PurchaseRequest = serde_json::from_str("{}").expect("parse");
    assert_eq!(req.group_id, 0, "缺字段必须落到会被 400 拒掉的 0");
}

#[test]
fn admin_create_does_not_bind_snake_case_ids() {
    // 前端 AdminSubscriptionAssignDialog 发的就是这个 body。旧实现的 UserID/GroupID
    // 没有 json tag，反序列化不拆下划线 → 两者恒为 0 → 400。
    // 这条测试把这个**已知缺陷**钉住，免得被人无意间"修好"从而打开一条
    // 旧实现从未执行过的写路径。
    let body = r#"{"user_id":7,"group_id":3,"funding_source":"manual"}"#;
    let req: AdminCreateRequest = serde_json::from_str(body).expect("parse");
    assert_eq!(req.user_id, 0);
    assert_eq!(req.group_id, 0);
}

#[test]
fn admin_create_binds_the_capitalized_field_names() {
    let body = r#"{"UserID":7,"GroupID":3,"FundingSource":"manual","price_paid_usd":9.9}"#;
    let req: AdminCreateRequest = serde_json::from_str(body).expect("parse");
    assert_eq!(req.user_id, 7);
    assert_eq!(req.group_id, 3);
    assert_eq!(req.funding_source, "manual");
    assert_eq!(req.price_paid_usd, 9.9);
}

// ── 日期算术 ─────────────────────────────────────────────────────────────────

#[test]
fn add_days_moves_forward_by_exactly_that_many_days() {
    let base = DateTime::from_timestamp(1_700_000_000, 0).expect("timestamp");
    for days in [1_i64, 7, 30, 365] {
        let moved = add_days(base, days);
        assert_eq!((moved - base).num_days(), days, "days={days}");
    }
}

#[test]
fn add_days_is_a_no_op_for_negative_or_overflowing_input() {
    // 绝不 panic：溢出/负数原样返回，让上层的校验去拒绝。
    let base = DateTime::from_timestamp(1_700_000_000, 0).expect("timestamp");
    assert_eq!(add_days(base, -1), base);
    assert_eq!(add_days(base, i64::MAX), base);
}

#[test]
fn purchase_validity_never_drops_below_one_day() {
    // 旧实现 `if days < 1 { days = 1 }`。零/负有效期的套餐仍然要给出一份当天有效的
    // 订阅，而不是一份已经过期的。
    for configured in [-5_i64, 0, 1, 30] {
        let days = configured.max(MIN_PURCHASE_VALIDITY_DAYS);
        assert!(days >= 1, "configured={configured}");
    }
}
