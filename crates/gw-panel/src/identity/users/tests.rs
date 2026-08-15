use super::*;

fn row(username: &str, role: &str, status: &str) -> AdminUserRow {
    AdminUserRow {
        id: 1,
        email: "u@example.com".into(),
        username: username.into(),
        role: role.into(),
        balance: 0.0,
        status: status.into(),
        concurrency: 1,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn blank_username_becomes_json_null_not_empty_string() {
    // 旧实现的 nullableString：空/全空白 → null。前端拿 null 显示占位符，拿 ""
    // 会渲染成一个空白单元格。
    for blank in ["", "   ", "\t\n"] {
        assert!(nullable_string(blank).is_none(), "blank={blank:?}");
    }
    assert_eq!(nullable_string("  bob "), Some("bob".to_owned()));
}

#[test]
fn admin_payload_backfills_missing_role_and_status() {
    // 老库里这两列可能是 NULL（旧实现读成 ""）。响应必须给出可用的默认值，
    // 否则前端的角色下拉框会落到一个不存在的选项上。
    let payload = AdminUserPayload::from(row("", "", ""));
    assert_eq!(payload.role, "user");
    assert_eq!(payload.status, USER_STATUS_ACTIVE);
    assert!(payload.username.is_none());
}

#[test]
fn admin_payload_keeps_explicit_role_and_status() {
    let payload = AdminUserPayload::from(row("bob", "admin", "suspended"));
    assert_eq!(payload.role, "admin");
    assert_eq!(payload.status, "suspended");
    assert_eq!(payload.username.as_deref(), Some("bob"));
}

#[test]
fn admin_payload_serialises_the_nine_keys_the_users_table_reads() {
    let json = serde_json::to_value(AdminUserPayload::from(row("bob", "user", "active")))
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
            "balance",
            "concurrency",
            "created_at",
            "email",
            "id",
            "role",
            "status",
            "updated_at",
            "username",
        ]
    );
}

#[test]
fn update_request_distinguishes_absent_balance_from_zero() {
    // 这是旧实现特意把 Balance 做成指针的理由：一次只改角色的编辑不能把余额清零。
    let role_only: UpdateUserRequest = serde_json::from_str(r#"{"role":"admin"}"#).expect("parse");
    assert!(
        role_only.balance.is_none(),
        "缺字段必须是 None，不是 Some(0)"
    );

    let zeroed: UpdateUserRequest = serde_json::from_str(r#"{"balance":0}"#).expect("parse");
    assert_eq!(zeroed.balance, Some(0.0), "显式 0 是一次真实的清零");
}

#[test]
fn update_request_distinguishes_absent_username_from_empty() {
    let absent: UpdateUserRequest = serde_json::from_str("{}").expect("parse");
    assert!(absent.username.is_none());

    let cleared: UpdateUserRequest = serde_json::from_str(r#"{"username":""}"#).expect("parse");
    assert_eq!(cleared.username.as_deref(), Some(""));
}

#[test]
fn create_request_binds_both_capitalisations() {
    // 旧实现那四个字段没有 tag，encoding/json 大小写不敏感 —— 两种拼写都要认。
    let lower: CreateUserRequest =
        serde_json::from_str(r#"{"email":"a@b.c","password":"secret12"}"#).expect("parse");
    assert_eq!(lower.email, "a@b.c");

    let upper: CreateUserRequest =
        serde_json::from_str(r#"{"Email":"a@b.c","Password":"secret12"}"#).expect("parse");
    assert_eq!(upper.email, "a@b.c");
    assert_eq!(upper.password, "secret12");
}

#[test]
fn deposit_request_defaults_to_a_rejectable_amount() {
    // 缺 amount 时必须落到一个会被业务判断拒掉的值（对应 `req.Amount <= 0`）。
    let req: DepositRequest = serde_json::from_str("{}").expect("parse");
    assert!(req.amount <= 0.0);
    assert!(req.note.is_empty());
}

#[test]
fn deposit_rejects_non_positive_amounts_including_nan() {
    // `is_positive_amount` 是"不满足 > 0 就拒"，不是 `amount <= 0.0`：NaN 对两个
    // 比较都是 false，写成后者会让 NaN 通过，然后 balance + NaN 把余额彻底毁掉。
    for amount in [0.0_f64, -1.0, -0.000_001, f64::NAN, f64::NEG_INFINITY] {
        assert!(!is_positive_amount(amount), "amount={amount} 必须被拒");
    }
    for amount in [0.01_f64, 1.0, 1_000.0] {
        assert!(is_positive_amount(amount), "amount={amount} 应当放行");
    }
}
