use super::*;

// 连库的部分（改绑的 403 不清缓存、撤销后不再出现在列表里）在 tests/panel/。
// 这里测的是响应形状 —— 前端直接读这些键，形状错了没有任何编译器会拦。

#[test]
fn masking_keeps_the_prefix_and_never_leaks_the_rest() {
    // 性质：遮蔽值以前缀开头，且比前缀长（后面是固定的遮蔽符）。
    for prefix in ["agw-000000ab", "agw-", ""] {
        let shown = masked(prefix);
        assert!(shown.starts_with(prefix), "prefix={prefix:?}");
        assert!(shown.len() > prefix.len(), "prefix={prefix:?}");
    }
}

#[test]
fn masked_value_of_a_real_key_does_not_contain_the_secret() {
    // 用真实生成器造一把 key，确认列表里那个 `key` 字段不含明文的任何后半段。
    let plaintext = new_api_key().expect("generate");
    let prefix = api_key_prefix(&plaintext);
    let shown = masked(prefix);
    assert!(
        !shown.contains(&plaintext[prefix.len()..]),
        "遮蔽值泄露了明文尾部"
    );
    assert_ne!(shown, plaintext);
}

#[test]
fn list_item_omits_absent_group_and_last_used() {
    // 旧实现的 `*uint` + omitempty：为空时**整个键消失**，不是 null。
    let item = ApiKeyListItem {
        id: 1,
        name: "n".into(),
        key: "agw-abc****".into(),
        key_prefix: "agw-abc".into(),
        status: "active".into(),
        group_id: None,
        last_used_at: None,
        created_at: chrono::Utc::now(),
    };
    let json = serde_json::to_value(&item).expect("serialise");
    let obj = json.as_object().expect("object");
    assert!(!obj.contains_key("group_id"));
    assert!(!obj.contains_key("last_used_at"));
    let mut keys: Vec<_> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["created_at", "id", "key", "key_prefix", "name", "status"]
    );
}

#[test]
fn list_item_emits_group_id_when_bound() {
    let item = ApiKeyListItem {
        id: 1,
        name: "n".into(),
        key: "agw-abc****".into(),
        key_prefix: "agw-abc".into(),
        status: "active".into(),
        group_id: Some(7),
        last_used_at: Some(chrono::Utc::now()),
        created_at: chrono::Utc::now(),
    };
    let json = serde_json::to_value(&item).expect("serialise");
    assert_eq!(json["group_id"], serde_json::json!(7));
    assert!(json.get("last_used_at").is_some());
}

#[test]
fn admin_item_uses_prefix_not_key_prefix() {
    // 管理员视图和用户视图的键名**故意不同**（旧实现里就是两套 gin.H）。
    // 统一它们会悄悄改掉管理员页面读的字段。
    let json = serde_json::to_value(AdminApiKeyItem {
        id: 3,
        name: "n".into(),
        prefix: "agw-abc".into(),
        status: "active".into(),
        quota: 0,
        quota_used: 0,
        created_at: chrono::Utc::now(),
    })
    .expect("serialise");
    let obj = json.as_object().expect("object");
    assert!(obj.contains_key("prefix"));
    assert!(!obj.contains_key("key_prefix"));
    assert!(!obj.contains_key("key"), "管理员视图不该出现任何密钥字段");
}

#[test]
fn rebind_request_distinguishes_unbind_from_absent_field() {
    // `{"group_id": null}` 与 `{}` 在旧实现里都解成 nil 指针 → 都表示解绑。
    // 这条是刻意的：前端的"取消绑定"按钮发的就是显式 null。
    let explicit: RebindRequest = serde_json::from_str(r#"{"group_id":null}"#).expect("parse");
    let absent: RebindRequest = serde_json::from_str("{}").expect("parse");
    assert!(explicit.group_id.is_none());
    assert!(absent.group_id.is_none());

    let bound: RebindRequest = serde_json::from_str(r#"{"group_id":5}"#).expect("parse");
    assert_eq!(bound.group_id, Some(5));
}

#[test]
fn create_request_tolerates_a_missing_name() {
    // gin 的 ShouldBindJSON 对缺字段不报错，校验发生在之后的业务判断里
    // （所以 `{}` 得到的是"名称不能为空"，不是"请求格式无效"）。
    let req: CreateRequest = serde_json::from_str("{}").expect("parse");
    assert!(req.name.is_empty());
}

#[test]
fn create_request_rejects_a_wrong_typed_name() {
    // 类型错了才是"请求格式无效"。
    assert!(serde_json::from_str::<CreateRequest>(r#"{"name":123}"#).is_err());
}
