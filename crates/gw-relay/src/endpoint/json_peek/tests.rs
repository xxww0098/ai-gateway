//! 顶层窥视器的性质：与 serde 同成败、嵌套字段不泄漏。

use super::{parse_top_fields, top_level_field};

/// 嵌套对象 / 数组里的同名键不能冒充顶层。
///
/// 守护的 bug：用 `payload.windows().find(b"\"usage\"")` 这种字节搜索
/// 去定位字段。模型回复里写一句 `{"usage":...}` 就会把计费数字换成
/// 模型自己编的。
#[test]
fn nested_keys_do_not_count_as_top_level() {
    let body = br#"{
        "messages": [{"model": "nested", "stream": true, "usage": {"prompt_tokens": 99}}],
        "model": "outer",
        "stream": false
    }"#;
    let fields = parse_top_fields(body).expect("outer object is valid");
    assert_eq!(fields.model.as_deref(), Some("outer"));
    assert_eq!(fields.stream, Some(false));
    assert!(top_level_field(body, "usage").is_none());
}

/// 重复键取最后一个，与 serde_json 一致。
#[test]
fn a_duplicate_top_level_key_keeps_the_last_value() {
    let body = br#"{"model":"first","model":"second"}"#;
    let fields = parse_top_fields(body).expect("duplicates are legal JSON");
    assert_eq!(fields.model.as_deref(), Some("second"));
    assert_eq!(
        top_level_field(body, "model"),
        Some(br#""second""#.as_slice())
    );
}

/// `stream_options` 只看键在不在，值是 null 也算写过。
#[test]
fn stream_options_null_still_counts_as_present() {
    let fields = parse_top_fields(br#"{"stream_options":null}"#).unwrap();
    assert!(fields.stream_options_present);
    assert_eq!(
        top_level_field(br#"{"stream_options":null}"#, "stream_options"),
        Some(br#"null"#.as_slice())
    );
}

/// 认识的字段类型不对 → 整份 peek 失败，不能 silently 丢掉那一个键。
///
/// 守护的 bug：`stream: "true"` 被当成「没写」而 `model` 还在。
/// 那样计费按非流式预扣，上游却按字符串（非法）回 400，两边对不上。
#[test]
fn a_wrong_type_on_a_known_field_fails_the_whole_peek() {
    assert!(parse_top_fields(br#"{"stream":"true"}"#).is_none());
    assert!(parse_top_fields(br#"{"model":1}"#).is_none());
    assert!(parse_top_fields(br#"{"max_tokens":"8"}"#).is_none());
}
