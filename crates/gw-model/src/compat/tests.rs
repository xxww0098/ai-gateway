use super::*;

/// 下面这些十六进制串不是手算的，是 Postgres 16 自己吐出来的：
///
/// ```sql
/// SELECT encode(numeric_send('9.9'::numeric), 'hex');
/// ```
///
/// 也就是说断言的是「PG 的线格式 → 这个值」这一外部事实，而不是把实现抄进断言
/// （规范 2.11）。
const WIRE_VECTORS: &[(&str, f64)] = &[
    ("0000000000000000", 0.0),
    ("00010000000000000001", 1.0),
    ("000200000000000100092328", 9.9),
    ("0002000040000002000309c4", -3.25),
    ("0001ffff0000000302ee", 0.075),
    ("0003000100000004000109291a85", 12345.6789),
    ("0005000300000004000109291a85007b11d7", 1234567890123.4567),
    ("0001fffe000000070ea6", 0.0000375),
    // 尾部数位被省略（ndigits=1, weight=1）——必须按 0 补齐，否则会解成 10。
    ("0001000100000000000a", 100000.0),
    // dscale=2 的 2.50：dscale 对 f64 无意义，值仍是 2.5。
    ("000200000000000200021388", 2.5),
];

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("测试向量不是合法十六进制"))
        .collect()
}

#[test]
fn numeric_wire_format_matches_postgres() {
    for (hex, expected) in WIRE_VECTORS {
        let got = numeric_from_binary(&unhex(hex)).expect("应当能解码");
        assert_eq!(got, *expected, "向量 {hex}");
    }
}

#[test]
fn numeric_nan_decodes_to_nan() {
    // SELECT encode(numeric_send('NaN'::numeric), 'hex');
    let got = numeric_from_binary(&unhex("00000000c0000000")).expect("NaN 应当能解码");
    assert!(got.is_nan());
}

#[test]
fn numeric_rejects_malformed_input() {
    // 头部不足 8 字节
    assert!(numeric_from_binary(&[0, 1, 0, 0]).is_err());
    // 未知符号位
    assert!(numeric_from_binary(&unhex("0000000012340000")).is_err());
    // 声明了 2 个数位却只给了 1 个
    assert!(numeric_from_binary(&unhex("00020000000000010009")).is_err());
}

/// 小数位越多，还原出来的字面量越长，但值必须仍落在同一个区间里 —— 用性质而不是
/// 具体常量来卡住「base-10000 分组拼接」这段逻辑。
#[test]
fn numeric_fraction_groups_keep_magnitude() {
    let small = numeric_from_binary(&unhex("0001fffe000000070ea6")).expect("0.0000375");
    let bigger = numeric_from_binary(&unhex("0001ffff0000000302ee")).expect("0.075");
    assert!(small > 0.0);
    assert!(small < bigger);
    assert!(bigger < 1.0);
}

#[test]
fn zero_time_is_epoch_start() {
    // time.Time{} 零值的 RFC3339 表示 == "0001-01-01T00:00:00Z"
    assert_eq!(
        zero_time().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "0001-01-01T00:00:00Z"
    );
}

#[test]
fn wrappers_unwrap_into_plain_rust_types() {
    // #[sqlx(try_from = "compat::X")] 依赖这些 From 实现存在且是恒等映射。
    assert_eq!(f64::from(Money(1.5)), 1.5);
    assert_eq!(Option::<f64>::from(MoneyOpt(None)), None);
    assert_eq!(String::from(Text("x".into())), "x");
    assert_eq!(i64::from(Int(-7)), -7);
    assert!(bool::from(Bool(true)));
    assert_eq!(DateTime::<Utc>::from(Ts(zero_time())), zero_time());
}
