use super::*;
use std::collections::HashSet;

// 「同一张码只能兑一次」那条连库测（对应既有连库测 TestRedeemCodePersistedAndSingleUse）
// 在 tests/panel/。这里覆盖码本身的性质 —— 这是安全回归测试
// TestRedeemCodesAreRandomAndSequentialGuessFails 的 Rust 侧对应物。

#[test]
fn generated_codes_are_uppercase_and_stable_under_the_lookup_normalisation() {
    // 兑换时 handler 会先 to_uppercase()。码本身必须已经是那个形态，
    // 否则用户拿到的码永远查不到。
    let code = generate_redeem_code().expect("entropy");
    assert_eq!(code, code.to_uppercase());
}

#[test]
fn generated_codes_carry_the_human_visible_prefix() {
    let code = generate_redeem_code().expect("entropy");
    assert!(code.starts_with(CODE_PREFIX), "code={code}");
    assert!(code.len() > CODE_PREFIX.len());
}

#[test]
fn generated_codes_do_not_repeat_and_are_not_sequential() {
    // 这条对应旧实现的安全回归测试：顺序生成的码等于把余额送人。
    // 性质有两层 —— 不重复，且相邻两次之间没有可利用的关系（这里用「不共享
    // 长公共前缀」近似）。
    let mut seen = HashSet::new();
    let mut previous: Option<String> = None;
    for _ in 0..256 {
        let code = generate_redeem_code().expect("entropy");
        assert!(seen.insert(code.clone()), "生成了重复的码：{code}");
        if let Some(prev) = &previous {
            let shared = prev
                .chars()
                .zip(code.chars())
                .take_while(|(a, b)| a == b)
                .count();
            // 前缀 "RDM-" 是四个字符，随机部分不该再共享多少。
            assert!(
                shared <= CODE_PREFIX.len() + 4,
                "两次生成共享了 {shared} 个字符：{prev} / {code}"
            );
        }
        previous = Some(code);
    }
}

#[test]
fn generated_codes_use_only_the_base32_alphabet() {
    // 字母表外的字符（尤其是小写）会破坏大小写归一化的单射性。
    let allowed: HashSet<char> = BASE32_ALPHABET.iter().map(|b| char::from(*b)).collect();
    let code = generate_redeem_code().expect("entropy");
    for ch in code.trim_start_matches(CODE_PREFIX).chars() {
        assert!(allowed.contains(&ch), "字母表外的字符 {ch:?} in {code}");
    }
}

#[test]
fn generated_codes_have_a_fixed_length() {
    // 定长意味着长度本身不泄露任何信息，也让前端能按固定宽度渲染。
    let lengths: HashSet<usize> = (0..32)
        .map(|_| generate_redeem_code().expect("entropy").len())
        .collect();
    assert_eq!(lengths.len(), 1, "长度不稳定：{lengths:?}");
}

// ── base32 编码本身 ──────────────────────────────────────────────────────────

#[test]
fn base32_matches_rfc4648_test_vectors() {
    // RFC 4648 §10 的官方向量（去掉填充）。不是从被测实现抄来的，
    // 而是来自规范 —— 这正是规则 2.11 要的那种「与另一份权威数据一致」。
    for (input, expected) in [
        ("", ""),
        ("f", "MY"),
        ("fo", "MZXQ"),
        ("foo", "MZXW6"),
        ("foob", "MZXW6YQ"),
        ("fooba", "MZXW6YTB"),
        ("foobar", "MZXW6YTBOI"),
    ] {
        assert_eq!(base32_no_pad(input.as_bytes()), expected, "input={input:?}");
    }
}

#[test]
fn base32_is_injective_on_the_entropy_width_we_use() {
    // 单射性是唯一索引不冲突的前提。抽样验证：不同输入 → 不同输出。
    let mut seen = HashSet::new();
    for i in 0_u32..512 {
        let mut raw = [0_u8; CODE_ENTROPY_BYTES];
        raw[..4].copy_from_slice(&i.to_be_bytes());
        assert!(seen.insert(base32_no_pad(&raw)), "i={i} 撞了");
    }
}

#[test]
fn base32_length_is_the_ceiling_of_bits_over_five() {
    // 性质：无填充编码的长度恰好是 ceil(8n/5)。
    for n in 0_usize..=16 {
        let raw = vec![0_u8; n];
        assert_eq!(base32_no_pad(&raw).len(), (n * 8).div_ceil(5), "n={n}");
    }
}

// ── 响应形状与请求绑定 ───────────────────────────────────────────────────────

#[test]
fn unused_code_omits_used_at_and_used_by() {
    let json = serde_json::to_value(RedeemCodeRecord {
        id: 1,
        code: "RDM-XXXX".into(),
        amount: 10.0,
        status: STATUS_UNUSED.into(),
        created_at: Utc::now(),
        used_at: None,
        used_by: None,
    })
    .expect("serialise");
    let obj = json.as_object().expect("object");
    assert!(!obj.contains_key("used_at"));
    assert!(!obj.contains_key("used_by"));
}

#[test]
fn used_code_reports_who_and_when() {
    let json = serde_json::to_value(RedeemCodeRecord {
        id: 1,
        code: "RDM-XXXX".into(),
        amount: 10.0,
        status: STATUS_USED.into(),
        created_at: Utc::now(),
        used_at: Some(Utc::now()),
        used_by: Some("u@example.com".into()),
    })
    .expect("serialise");
    assert!(json.get("used_at").is_some());
    assert_eq!(json["used_by"], serde_json::json!("u@example.com"));
}

#[test]
fn batch_creation_bounds_reject_the_dangerous_inputs() {
    // count 与 amount 的边界：0 / 负 / 超上限 / 非正金额都必须落在拒绝一侧。
    let rejected = |count: i64, amount: f64| {
        count <= 0 || count > MAX_BATCH || !crate::identity::is_positive_amount(amount)
    };
    assert!(rejected(0, 10.0));
    assert!(rejected(-1, 10.0));
    assert!(rejected(MAX_BATCH + 1, 10.0));
    assert!(rejected(1, 0.0));
    assert!(rejected(1, -5.0));
    assert!(rejected(1, f64::NAN));
    assert!(!rejected(1, 0.01));
    assert!(!rejected(MAX_BATCH, 10.0));
}

#[test]
fn redeem_request_defaults_to_an_empty_code() {
    let req: RedeemRequest = serde_json::from_str("{}").expect("parse");
    assert!(req.code.is_empty());
}
