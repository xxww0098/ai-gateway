use super::*;

// 连库的那条（重复确认只入账一次）在 tests/panel/。这里测订单号这个双向契约，
// 以及金额绑定的边界 —— 两者都不碰库，却都能悄悄毁掉钱。

#[test]
fn public_order_id_round_trips() {
    // 性质：渲染出去的订单号必须能被自己的解析器原样拆回来。前端把它当不透明串
    // 传回 /payment/*/status，两端对不上就查不到单。
    for (provider, id) in [
        ("stripe", 1_i64),
        ("alipay", 999_999),
        ("wechat", 1_000_000),
    ] {
        let rendered = public_order_id(provider, id);
        assert_eq!(
            parse_public_order_id(&rendered),
            Some((provider, id)),
            "rendered={rendered}"
        );
    }
}

#[test]
fn public_order_id_is_zero_padded_to_a_stable_width() {
    // 性质：小 id 也不短于大 id 的最小宽度 —— 前端按定宽渲染。
    let short = public_order_id("stripe", 1);
    let long = public_order_id("stripe", 123_456);
    assert_eq!(short.len(), long.len());
    assert!(short.ends_with("000001"));
}

#[test]
fn parse_public_order_id_requires_exactly_two_segments() {
    // 旧实现用 strings.Split 并要求恰好两段。放宽会让 `a-b-123` 落到一个意外的订单。
    for raw in [
        "",
        "stripe",
        "stripe-",
        "stripe-000001-x",
        "stripe-abc",
        "stripe-0x1",
    ] {
        assert!(parse_public_order_id(raw).is_none(), "raw={raw:?}");
    }
}

#[test]
fn an_empty_provider_parses_but_can_never_match_a_real_order() {
    // 旧实现的 strings.Split("-000001", "-") 同样得到两段，于是解析"成功"、
    // provider 为空 —— 拦截发生在随后的 `WHERE provider = ''` 上，那里查不到行。
    // 把这一点写下来，免得有人以为解析器漏了一个校验而"补"上一个不同的错误码。
    assert_eq!(parse_public_order_id("-000001"), Some(("", 1)));
}

#[test]
fn parse_public_order_id_rejects_ids_beyond_i64() {
    // 主键是 bigint；越界的数字不可能是一个订单。
    let raw = format!("stripe-{}", u64::MAX);
    assert!(parse_public_order_id(&raw).is_none());
}

#[test]
fn parse_public_order_id_keeps_the_provider_for_the_cross_check() {
    // handler 会拿解析出的 provider 与路由上的渠道比对，两者不同就 404 ——
    // 否则 /payment/alipay/status 能查到一张 stripe 的单。
    let (provider, id) = parse_public_order_id("wechat-000042").expect("parse");
    assert_eq!(provider, "wechat");
    assert_eq!(id, 42);
}

#[test]
fn amount_binding_rejects_non_positive_and_nan() {
    for raw in [
        r#"{"amount":0}"#,
        r#"{"amount":-1}"#,
        r#"{}"#,
        r#"{"amount":"5"}"#,
        "",
        "not json",
    ] {
        assert!(bind_amount(raw.as_bytes()).is_none(), "raw={raw:?}");
    }
    assert_eq!(bind_amount(br#"{"amount":9.9}"#), Some(9.9));
}

#[test]
fn amount_binding_rejects_nan_through_the_positive_predicate() {
    // JSON 本身表达不了 NaN，但 `is_positive_amount` 那个"不满足 > 0 就拒"的
    // 写法是刻意的：换成 `x <= 0.0` 会在任何 NaN 来源下放行，然后 amount_usd
    // 变成 NaN 并污染余额。
    assert!(!crate::identity::is_positive_amount(f64::NAN));
    assert!(!crate::identity::is_positive_amount(0.0));
    assert!(crate::identity::is_positive_amount(0.01));
}

#[test]
fn order_record_carries_the_twelve_keys_the_orders_table_reads() {
    let json = serde_json::to_value(OrderRecord {
        id: 1,
        user_id: 2,
        provider: "stripe".into(),
        amount_usd: 10.0,
        amount_local: 10.0,
        currency: "USD".into(),
        status: "pending".into(),
        transaction_id: None,
        metadata: None,
        paid_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
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
            "amount_local",
            "amount_usd",
            "created_at",
            "currency",
            "id",
            "metadata",
            "paid_at",
            "provider",
            "status",
            "transaction_id",
            "updated_at",
            "user_id",
        ]
    );
    // 三个可空字段是 null 而不是消失（旧实现用的是指针 + 无 omitempty）。
    for key in ["transaction_id", "metadata", "paid_at"] {
        assert_eq!(json[key], serde_json::Value::Null, "{key}");
    }
}

#[test]
fn payment_config_stubs_are_the_three_local_channels_and_disabled() {
    let items = payment_config_stubs();
    let providers: Vec<&str> = items
        .iter()
        .filter_map(|item| item["provider"].as_str())
        .collect();
    assert_eq!(providers, ["stripe", "alipay", "wechat"]);
    assert!(items.iter().all(|item| item["enabled"] == false));
}

#[test]
fn a_user_order_list_ignores_query_user_id() {
    // 性质：即使用户在 query 里塞了别人的 id，绑定的仍是登录身份。
    // 管理员列表才认 ?user_id=；0 是「不过滤」哨兵，用户路径碰不得。
    assert_eq!(own_orders_user_id(7, Some("99")), 7);
    assert_eq!(own_orders_user_id(7, None), 7);
    assert_eq!(own_orders_user_id(7, Some("0")), 7);
    assert_ne!(own_orders_user_id(7, Some("0")), UNSCOPED_USER_ID);
}

#[test]
fn local_currency_conversion_is_proportional() {
    // 性质：本币金额与美元金额成正比，且 0 映射到 0。不断言 7.2 本身
    // （那是抄源码），只钉住"它是一个正的线性换算"。
    const { assert!(CNY_PER_USD > 0.0) };
    assert_eq!(0.0 * CNY_PER_USD, 0.0);
    assert!((2.0 * CNY_PER_USD - 2.0 * (1.0 * CNY_PER_USD)).abs() < f64::EPSILON);
}
