use super::*;

// 连库的部分（越权取单得到 404、建单与首条回复同事务）在 tests/panel/。
// 这里覆盖两条纯逻辑：状态流转规则，和拼进 Markdown 的文件名转义。

// ── 状态流转 ─────────────────────────────────────────────────────────────────

#[test]
fn an_admin_reply_answers_an_open_ticket() {
    assert_eq!(status_after_reply(STATUS_OPEN, true), STATUS_ANSWERED);
}

#[test]
fn a_user_reply_reopens_an_answered_ticket() {
    assert_eq!(status_after_reply(STATUS_ANSWERED, false), STATUS_OPEN);
}

#[test]
fn a_reply_never_changes_any_other_state() {
    // 关掉的单被追问不会自己弹回来 —— 那是 PUT /status 的职责。
    // 这两条边最容易在实现时被写成"任何回复都开单"。
    for state in ["closed", "resolved", "pending", "", "AnythingElse"] {
        for is_admin in [true, false] {
            assert_eq!(
                status_after_reply(state, is_admin),
                state,
                "state={state} admin={is_admin}"
            );
        }
    }
}

#[test]
fn a_reply_from_the_same_side_is_idempotent_on_status() {
    // 管理员连回两条：第一条把 open 推到 answered，第二条保持 answered。
    let after_first = status_after_reply(STATUS_OPEN, true);
    assert_eq!(status_after_reply(&after_first, true), after_first);
    // 用户连问两条同理。
    let after_user = status_after_reply(STATUS_ANSWERED, false);
    assert_eq!(status_after_reply(&after_user, false), after_user);
}

#[test]
fn the_two_sides_ping_pong_between_exactly_two_states() {
    // 性质：从 open 出发，管理员/用户交替回复只会在两个状态间来回。
    let mut state = STATUS_OPEN.to_owned();
    for round in 0..4 {
        state = status_after_reply(&state, true);
        assert_eq!(state, STATUS_ANSWERED, "round={round}");
        state = status_after_reply(&state, false);
        assert_eq!(state, STATUS_OPEN, "round={round}");
    }
}

// ── 文件名转义 ───────────────────────────────────────────────────────────────

#[test]
fn escaping_neutralises_markup_in_a_filename() {
    // 文件名是用户输入，会被拼进 `![alt](data:…)` 并在工单页渲染。
    let escaped = escape_html(r#"<img src=x onerror="alert(1)">.png"#);
    assert!(!escaped.contains('<'), "残留了 <：{escaped}");
    assert!(!escaped.contains('>'), "残留了 >：{escaped}");
    assert!(!escaped.contains('"'), "残留了 \"：{escaped}");
}

#[test]
fn escaping_covers_all_five_characters_go_escapes() {
    for raw in ["&", "'", "<", ">", "\""] {
        let escaped = escape_html(raw);
        assert_ne!(escaped, raw, "{raw:?} 没有被转义");
        assert!(
            escaped.starts_with('&') && escaped.ends_with(';'),
            "{escaped}"
        );
    }
}

#[test]
fn escaping_does_not_double_escape_its_own_output() {
    // `&` 必须最先被转 —— 顺序写错会把 `&lt;` 再转成 `&amp;lt;`。
    // 性质：一次转义之后，输出里不含 `&amp;amp;`。
    let once = escape_html("a & b < c");
    assert!(!once.contains("&amp;amp;"), "重复转义了：{once}");
    assert!(once.contains("&amp;"));
    assert!(once.contains("&lt;"));
}

#[test]
fn escaping_leaves_ordinary_filenames_alone() {
    for raw in ["photo.png", "截图 2026-08-15.png", "a_b-c(1).jpeg", ""] {
        assert_eq!(escape_html(raw), raw, "raw={raw:?}");
    }
}

// ── 响应形状 ─────────────────────────────────────────────────────────────────

fn ticket_row(status: &str) -> TicketRow {
    TicketRow {
        id: 1,
        user_id: 2,
        title: "登录不上".into(),
        category: "other".into(),
        priority: "medium".into(),
        status: status.into(),
        assignee_id: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn ticket_payload_has_the_ten_keys_the_ticket_pages_read() {
    let json = serde_json::to_value(ticket_row(STATUS_OPEN).into_payload("")).expect("serialise");
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
            "assignee_id",
            "category",
            "content",
            "created_at",
            "id",
            "priority",
            "status",
            "title",
            "updated_at",
            "user_id",
        ]
    );
    // 未分派时 assignee_id 是 null 而不是缺席（原实现用的是 *uint，无 omitempty）。
    assert_eq!(json["assignee_id"], serde_json::Value::Null);
}

#[test]
fn list_rows_carry_an_empty_content_but_the_create_response_carries_the_body() {
    // 正文其实住在第一条回复里；只有刚建单那次响应会把它填上。
    assert_eq!(ticket_row(STATUS_OPEN).into_payload("").content, "");
    assert_eq!(ticket_row(STATUS_OPEN).into_payload("正文").content, "正文");
}

#[test]
fn ticket_detail_flattens_the_ticket_and_appends_replies() {
    // 详情不是 `{"ticket": …, "replies": …}`，而是工单字段**平铺**再加一个
    // replies —— 原实现往 gin.H 上直接 `out["replies"] = …`。
    let json = serde_json::to_value(TicketDetail {
        ticket: ticket_row(STATUS_OPEN).into_payload(""),
        replies: vec![ReplyPayload {
            id: 5,
            ticket_id: 1,
            user_id: 2,
            is_admin: false,
            content: "正文".into(),
            created_at: Utc::now(),
        }],
    })
    .expect("serialise");
    assert!(json.get("ticket").is_none(), "不该出现嵌套的 ticket 键");
    assert_eq!(json["id"], serde_json::json!(1));
    assert_eq!(json["replies"].as_array().map(Vec::len), Some(1));
    assert_eq!(json["replies"][0]["is_admin"], serde_json::json!(false));
}

#[test]
fn create_request_falls_back_to_defaults_for_the_three_optional_fields() {
    let req: CreateTicketRequest = serde_json::from_str(r#"{"content":"救命"}"#).expect("parse");
    assert_eq!(first_non_empty(&[&req.title, DEFAULT_TITLE]), DEFAULT_TITLE);
    assert_eq!(
        first_non_empty(&[&req.category, DEFAULT_CATEGORY]),
        DEFAULT_CATEGORY
    );
    assert_eq!(
        first_non_empty(&[&req.priority, DEFAULT_PRIORITY]),
        DEFAULT_PRIORITY
    );
}

#[test]
fn assign_request_treats_an_empty_body_as_assign_to_me() {
    // 原实现忽略 ShouldBindJSON 的错误：空 body / `{}` / 显式 null 都表示"派给自己"。
    for body in ["{}", r#"{"assignee_id":null}"#] {
        let req: AssignRequest = serde_json::from_str(body).expect("parse");
        assert!(req.assignee_id.is_none(), "body={body}");
    }
    let explicit: AssignRequest = serde_json::from_str(r#"{"assignee_id":9}"#).expect("parse");
    assert_eq!(explicit.assignee_id, Some(9));
}

#[test]
fn the_image_ceiling_is_stated_in_bytes_and_is_a_whole_number_of_mebibytes() {
    // 上限同时是数据库行大小的上限（图片会 base64 进正文），所以它必须有界且
    // 不为零。base64 会再放大约三分之一 —— 这条断言把那份预算写下来。
    const { assert!(MAX_IMAGE_BYTES > 0) };
    const { assert!(MAX_IMAGE_BYTES.is_multiple_of(1 << 20)) };
    let encoded_ceiling = MAX_IMAGE_BYTES.div_ceil(3) * 4;
    assert!(encoded_ceiling > MAX_IMAGE_BYTES);
}
