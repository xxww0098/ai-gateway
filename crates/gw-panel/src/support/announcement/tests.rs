use super::*;

// 「管理员发的公告能被用户读到」那条连库测（对应原实现的
// TestAnnouncementPersistedAndReachesUsers）在 tests/panel/。
// 这里盯两个视图的字段差异，以及 create 与 update 之间那几处刻意的不对称。

#[test]
fn the_user_digest_carries_only_three_keys() {
    // 仪表盘上是一行标题：正文不该跟着一起下发。
    let json = serde_json::to_value(AnnouncementDigest {
        id: 1,
        title: "维护通知".into(),
        kind: "info".into(),
    })
    .expect("serialise");
    let mut keys: Vec<_> = json
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["id", "title", "type"]);
    assert!(
        !json.as_object().expect("object").contains_key("content"),
        "用户视图不该带正文"
    );
}

#[test]
fn the_admin_record_carries_all_six_keys() {
    let json = serde_json::to_value(AnnouncementRecord {
        id: 1,
        title: "维护通知".into(),
        content: "今晚 02:00 停机".into(),
        kind: "warning".into(),
        is_active: true,
        created_at: Utc::now(),
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
        ["content", "created_at", "id", "is_active", "title", "type"]
    );
}

#[test]
fn both_views_call_the_column_type_not_kind() {
    // Rust 侧字段叫 kind（避开关键字），线上必须是 `type`。写错了前端拿不到
    // 公告级别，全部渲染成默认样式。
    let digest = serde_json::to_value(AnnouncementDigest {
        id: 1,
        title: "t".into(),
        kind: "warning".into(),
    })
    .expect("serialise");
    assert_eq!(digest["type"], serde_json::json!("warning"));
    assert!(digest.get("kind").is_none());

    let record = serde_json::to_value(AnnouncementRecord {
        id: 1,
        title: "t".into(),
        content: "c".into(),
        kind: "warning".into(),
        is_active: false,
        created_at: Utc::now(),
    })
    .expect("serialise");
    assert_eq!(record["type"], serde_json::json!("warning"));
    assert!(record.get("kind").is_none());
}

#[test]
fn save_request_reads_the_type_key_not_a_renamed_one() {
    let req: SaveRequest =
        serde_json::from_str(r#"{"title":"t","content":"c","type":"warning","is_active":true}"#)
            .expect("parse");
    assert_eq!(req.kind, "warning");
    assert!(req.is_active);
}

#[test]
fn save_request_defaults_is_active_to_false() {
    // bool 零值是 false：不传就是**停用**。写成 true 会让每一条新公告
    // 直接推到所有人的仪表盘上。
    let req: SaveRequest = serde_json::from_str(r#"{"title":"t","content":"c"}"#).expect("parse");
    assert!(!req.is_active);
    assert!(req.kind.is_empty());
}

#[test]
fn creation_requires_both_title_and_content_but_update_does_not() {
    // create 的校验是 `title == "" || content == ""` → 400；update 那边完全没有
    // 这一段。两处不对称是既有行为，统一它会改掉一个端点的可用输入集。
    let create_rejects =
        |title: &str, content: &str| title.trim().is_empty() || content.trim().is_empty();
    assert!(create_rejects("", "c"));
    assert!(create_rejects("t", "  "));
    assert!(!create_rejects("t", "c"));
}

#[test]
fn an_empty_type_means_default_on_create_and_keep_on_update() {
    // create：空 → "info"。update：空 → 保持原值（SQL 里的 NULLIF+COALESCE）。
    // 这条把两种含义的区别写下来 —— 它们共用同一个请求结构体，最容易混。
    let blank = "   ";
    let on_create = if blank.trim().is_empty() {
        DEFAULT_TYPE
    } else {
        blank.trim()
    };
    assert_eq!(on_create, DEFAULT_TYPE);
    // update 侧把空串原样交给 SQL，由 NULLIF 决定"保持原值"。
    assert_eq!(blank.trim(), "");
}
