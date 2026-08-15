use super::*;

// 连库的那几条（停用用户被拒、logout 之后旧 token 失效、API Key 缓存命中路径）
// 在 tests/panel/。这里覆盖不碰库的部分：头部解析、口令校验、限流桶、时间格式。

// ── 口令/邮箱的最低门槛 ───────────────────────────────────────────────────────

#[test]
fn auth_input_requires_an_at_sign_and_eight_bytes_of_password() {
    assert!(valid_auth_input("a@b.c", "12345678"));
    assert!(!valid_auth_input("", "12345678"), "空邮箱");
    assert!(!valid_auth_input("nope", "12345678"), "没有 @");
    assert!(!valid_auth_input("a@b.c", "1234567"), "口令差一个字节");
}

#[test]
fn password_length_is_counted_in_bytes_not_characters() {
    // 旧实现用的是 len(password)（字节）。三个汉字 = 9 字节，够门槛；
    // 换成 chars().count() 就只有 3，会把一批现有口令挡在门外。
    let three_han = "密码码";
    assert_eq!(three_han.chars().count(), 3);
    assert!(three_han.len() >= 8);
    assert!(valid_auth_input("a@b.c", three_han));
}

// ── 限流 ─────────────────────────────────────────────────────────────────────

fn at_minute(minute: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(minute * 60, 0).expect("valid timestamp")
}

#[test]
fn limiter_admits_exactly_capacity_requests_per_window() {
    // 性质：窗口内前 N 次放行、第 N+1 次拒 —— N 是传进去的容量，不是抄来的常量。
    let identity = "test:limiter_admits_exactly_capacity";
    let capacity = 4;
    let now = at_minute(10_000_000);
    for i in 0..capacity {
        assert!(
            allow_request_at(identity, capacity, now),
            "第 {i} 次应当放行"
        );
    }
    assert!(!allow_request_at(identity, capacity, now), "超额必须被拒");
}

#[test]
fn limiter_resets_on_the_next_minute() {
    let identity = "test:limiter_resets";
    let capacity = 2;
    let minute = 10_000_100;
    assert!(allow_request_at(identity, capacity, at_minute(minute)));
    assert!(allow_request_at(identity, capacity, at_minute(minute)));
    assert!(!allow_request_at(identity, capacity, at_minute(minute)));
    // 跨进下一分钟，配额回满。
    assert!(allow_request_at(identity, capacity, at_minute(minute + 1)));
}

#[test]
fn limiter_buckets_are_independent_per_identity() {
    // 一个 IP 撞满不该影响另一个 IP —— 否则一次爆破就能把所有人挡在门外。
    let now = at_minute(10_000_200);
    assert!(allow_request_at("test:limiter_a", 1, now));
    assert!(!allow_request_at("test:limiter_a", 1, now));
    assert!(allow_request_at("test:limiter_b", 1, now));
}

#[test]
fn limiter_refuses_everything_at_non_positive_capacity() {
    let now = at_minute(10_000_300);
    for capacity in [0_i64, -1] {
        assert!(!allow_request_at("test:limiter_zero", capacity, now));
    }
}

#[test]
fn auth_limit_is_far_below_a_credential_stuffing_rate() {
    // 不抄常量值，只钉住它的量级：这是防撞库的闸门，几百/分钟就等于没有。
    const { assert!(AUTH_RATE_LIMIT_PER_MIN > 0) };
    const { assert!(AUTH_RATE_LIMIT_PER_MIN < 60, "每秒一次以上就挡不住撞库了") };
}

// ── 响应形状 ─────────────────────────────────────────────────────────────────

#[test]
fn auth_user_created_at_is_a_second_precision_rfc3339_string() {
    // 旧实现手工 Format 成秒精度，而不是直接输出 time.Time（纳秒）。前端把
    // AuthUser.created_at 声明成 string，靠的就是这个。
    let ts = DateTime::from_timestamp(1_700_000_000, 987_654_321).expect("timestamp");
    let rendered = legacy_rfc3339(ts);
    assert!(rendered.ends_with('Z'), "UTC 必须渲染成 Z：{rendered}");
    assert!(!rendered.contains('.'), "不能带亚秒：{rendered}");
    // 能被原样解析回同一秒。
    let parsed = DateTime::parse_from_rfc3339(&rendered).expect("round trip");
    assert_eq!(parsed.timestamp(), ts.timestamp());
}

#[test]
fn auth_user_payload_has_exactly_the_six_keys_the_frontend_declares() {
    let json = serde_json::to_value(AuthUserPayload {
        id: 1,
        email: "u@example.com".into(),
        role: "user".into(),
        balance: 1.0,
        status: "active".into(),
        created_at: legacy_rfc3339(Utc::now()),
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
        ["balance", "created_at", "email", "id", "role", "status"]
    );
    assert!(json["created_at"].is_string());
}

#[test]
fn auth_request_treats_missing_fields_as_empty_not_as_a_parse_error() {
    // gin 的 ShouldBindJSON 语义：`{}` 能绑上，校验发生在之后。
    let req: AuthRequest = serde_json::from_str("{}").expect("parse");
    assert!(req.email.is_empty() && req.password.is_empty());
    // 类型错了才是解析错误。
    assert!(serde_json::from_str::<AuthRequest>(r#"{"email":1}"#).is_err());
}
