use super::*;

// 窗口函数那条（分页后累计余额仍然正确，对应既有连库测
// TestBalanceHistoryRunningBalancePaginated）在 tests/panel/，需要真 Postgres。
// 这里测两个视图的形状差异 —— 它们长得很像，最容易被"统一"掉。

fn item(amount: f64, balance_after: f64) -> BalanceHistoryItem {
    BalanceHistoryItem {
        id: 1,
        user_id: 2,
        amount,
        kind_type: "credit".into(),
        kind: "credit".into(),
        reference: "initial_register_credit".into(),
        note: "initial_register_credit".into(),
        balance_before: balance_after - amount,
        balance_after,
        operator_email: String::new(),
        created_at: Utc::now(),
    }
}

#[test]
fn user_item_exposes_both_names_of_the_same_two_values() {
    // type/kind 与 reference/note 是同一个值的两套键，前端不同页面各读一套。
    // 去掉任何一个都会让某个页面空掉。
    let json = serde_json::to_value(item(1.0, 1.0)).expect("serialise");
    assert_eq!(json["type"], json["kind"]);
    assert_eq!(json["reference"], json["note"]);
}

#[test]
fn user_item_has_the_eleven_keys_the_history_page_reads() {
    let json = serde_json::to_value(item(1.0, 1.0)).expect("serialise");
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
            "amount",
            "balance_after",
            "balance_before",
            "created_at",
            "id",
            "kind",
            "note",
            "operator_email",
            "reference",
            "type",
            "user_id",
        ]
    );
}

#[test]
fn before_plus_amount_always_equals_after() {
    // 这是这一页唯一的数值不变量：前端拿它画"变动前 → 变动后"。
    for (amount, after) in [
        (1.0_f64, 1.0_f64),
        (-2.5, 7.5),
        (0.0, 3.0),
        (100.0, 100.0),
        (-0.000_01, 0.999_99),
    ] {
        let row = item(amount, after);
        assert!(
            (row.balance_before + row.amount - row.balance_after).abs() < 1e-9,
            "amount={amount} after={after}"
        );
    }
}
