use super::*;

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
fn unknown_role_strings_are_not_writable() {
    assert!(crate::identity::Role::parse("root").is_err());
    assert!(crate::identity::Role::parse("superadmin").is_err());
}
