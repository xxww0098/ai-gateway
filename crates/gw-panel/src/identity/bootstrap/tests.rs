use super::*;

// 连库的那几条（"没有管理员时提升"/"有管理员后永久失效"/"幂等"）在
// tests/panel/ 里，需要真 Postgres。这里只测那个不碰库、却最容易写错的谓词。

#[test]
fn unconfigured_deployment_never_has_a_bootstrap_target() {
    // 最重要的一条：没配置引导邮箱时，任何注册者都不是候选。写反了就等于
    // 「第一个注册的人自动成为管理员」。
    for email in ["", "someone@example.com", "  "] {
        assert!(!is_bootstrap_target("", email), "email={email:?}");
    }
}

#[test]
fn bootstrap_target_ignores_case_and_surrounding_space() {
    let configured = "founder@example.com";
    for typed in [
        "founder@example.com",
        "Founder@Example.com",
        "  FOUNDER@EXAMPLE.COM  ",
        "\tfounder@example.com\n",
    ] {
        assert!(is_bootstrap_target(configured, typed), "typed={typed:?}");
    }
}

#[test]
fn bootstrap_target_rejects_anything_but_that_exact_address() {
    let configured = "founder@example.com";
    for other in [
        "founder@example.com.evil.test",
        "xfounder@example.com",
        "founder@example.co",
        "founder+admin@example.com",
        "",
    ] {
        assert!(!is_bootstrap_target(configured, other), "other={other:?}");
    }
}
