use super::*;

// 连库的部分（权益过滤、MAX(group_id)+1）在 tests/panel/。这里盯两件事：
// 响应形状，以及那条刻意保留的旧实现绑定缺陷 —— 后者没有测试就会在下一次
// "顺手修一下" 里被静悄悄改掉，连同它牵动的配额语义。

#[test]
fn available_group_always_emits_the_three_limit_keys() {
    // 旧实现的 availableGroupItem 上三个 *_limit_usd 没有 omitempty：
    // 它们恒为 null，但键必须在，否则前端读到 undefined 而不是 null。
    let json = serde_json::to_value(AvailableGroup {
        id: 1,
        name: "default".into(),
        description: String::new(),
        subscription_type: AVAILABLE_SUBSCRIPTION_TYPE.into(),
        rate_multiplier: 1.0,
        daily_limit_usd: None,
        weekly_limit_usd: None,
        monthly_limit_usd: None,
        default_validity_days: AVAILABLE_DEFAULT_VALIDITY_DAYS,
    })
    .expect("serialise");
    for key in ["daily_limit_usd", "weekly_limit_usd", "monthly_limit_usd"] {
        assert_eq!(
            json[key],
            serde_json::Value::Null,
            "{key} 必须是 null 而不是缺席"
        );
    }
}

#[test]
fn available_group_and_package_use_different_subscription_type_labels() {
    // 用户侧是 standard、管理员套餐侧是 subscription。合并成一个会让前端的
    // 分支走错。
    assert_ne!(AVAILABLE_SUBSCRIPTION_TYPE, PACKAGE_SUBSCRIPTION_TYPE);
}

#[test]
fn package_payload_carries_the_nine_keys_the_admin_page_reads() {
    let json = serde_json::to_value(PackagePayload {
        id: 2,
        name: "Pro".into(),
        subscription_type: PACKAGE_SUBSCRIPTION_TYPE.into(),
        rate_multiplier: 0.95,
        daily_limit_usd: Some(1.0),
        weekly_limit_usd: None,
        monthly_limit_usd: Some(100.0),
        default_validity_days: 30,
        subscription_price_usd: 29.9,
    })
    .expect("serialise");
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
            "daily_limit_usd",
            "default_validity_days",
            "id",
            "monthly_limit_usd",
            "name",
            "rate_multiplier",
            "subscription_price_usd",
            "subscription_type",
            "weekly_limit_usd",
        ]
    );
}

#[test]
fn save_request_does_not_bind_snake_case_limits() {
    // 前端发的就是这个 body。旧实现侧那三个字段没有 json tag，encoding/json 只做
    // 大小写不敏感匹配、不拆下划线，所以三个额度绑不上、一直是 nil。
    // 这条测试把这个**已知缺陷**钉住：哪天有人"顺手修好"，配额行为会跟着变，
    // 应该是一次显式决定，而不是一次静默改动。
    let body = r#"{
        "name":"Pro",
        "subscription_type":"subscription",
        "rate_multiplier":0.95,
        "default_validity_days":30,
        "daily_limit_usd":5,
        "weekly_limit_usd":20,
        "monthly_limit_usd":100,
        "subscription_price_usd":29.9
    }"#;
    let req: SavePackageRequest = serde_json::from_str(body).expect("parse");
    assert_eq!(req.name, "Pro");
    assert_eq!(req.subscription_price_usd, 29.9);
    assert!(req.daily_limit_usd.is_none(), "与既有实现一致：绑不上");
    assert!(req.weekly_limit_usd.is_none());
    assert!(req.monthly_limit_usd.is_none());
}

#[test]
fn save_request_binds_the_field_names_case_insensitively() {
    // 旧实现真正能接受的拼写。
    for key in ["DailyLimitUSD", "dailylimitusd"] {
        let body = format!(r#"{{"name":"x","{key}":7.5}}"#);
        let req: SavePackageRequest = serde_json::from_str(&body).expect("parse");
        assert_eq!(req.daily_limit_usd, Some(7.5), "key={key}");
    }
}

#[test]
fn save_request_defaults_are_all_zero_so_the_handler_can_apply_fallbacks() {
    // 缺字段不是解析错误（gin 的 ShouldBindJSON 语义），零值留给 handler 兜底成
    // rate=1 / 30 天。
    let req: SavePackageRequest = serde_json::from_str("{}").expect("parse");
    assert!(req.name.is_empty());
    assert!(req.rate_multiplier <= 0.0);
    assert!(req.default_validity_days <= 0);
}
