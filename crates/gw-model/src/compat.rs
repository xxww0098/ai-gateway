//! 既有 schema 列语义的解码适配器。
//!
//! 这一层存在的原因只有两个，都是「历史建库脚本建的表」逼出来的：
//!
//! 1. **`float64` → `numeric`**。历史建库把 `float64` 建成 Postgres `numeric`
//!    （DDL 里写作 `decimal`），而 sqlx 没有 `f64: Decode<Postgres>` for `NUMERIC`
//!    —— 它只把 `NUMERIC` 交给 `rust_decimal` / `bigdecimal`。本项目的钱一律是
//!    `f64`（`gw-pricing`：「Money never rounds here」，且 JSON 必须与历史的
//!    float64 输出逐位一致），所以这里自己实现 `NUMERIC` 线格式 → `f64`。
//!
//! 2. **几乎所有列都是 nullable**。历史建库只在 tag 里写了 `not null` 的列上加
//!    NOT NULL，其余一律可空；而其读 NULL 时给结构体填零值
//!    （`""` / `0` / `false` / `time.Time{}`），不报错。一个由老版本建库脚本
//!    建出来的库（`ALTER TABLE ADD COLUMN` 不回填）就会有这种 NULL。
//!    sqlx 默认对 `String` / `i64` 解 NULL 直接报 `UnexpectedNull`，那等于历史实现能
//!    读的行 Rust 读不了 —— 破坏「现有 Postgres 数据可直接被 Rust 读写」。
//!
//! 用法（见 `crate::user::User` 等实体）：
//!
//! ```ignore
//! #[derive(sqlx::FromRow)]
//! struct User {
//!     #[sqlx(try_from = "compat::Money")] balance: f64,   // numeric，NULL → 0.0
//!     #[sqlx(try_from = "compat::Text")]  username: String, // NULL → ""
//! }
//! ```
//!
//! 历史上是指针的字段（`*float64` / `*string` / `*time.Time` / `*uint`）不走这里：
//! 它们本来就是 `Option<T>`，sqlx 原生就按 NULL 处理。
//!
//! 写入方向不需要适配器：绑定一个 `f64` 参数到 `numeric` 列时，Postgres 会做
//! float8 → numeric 的 assignment cast；比较 `numeric >= $1(float8)` 也有 numeric
//! → float8 的 implicit cast。只有**标量聚合查询**要注意：
//! `query_scalar::<_, f64>("SELECT SUM(cost) …")` 会因为 `SUM(numeric)` 仍是
//! numeric 而失败，写成 `SUM(cost)::float8` 或 `query_scalar::<_, compat::Money>` 即可。

use std::fmt::Write as _;

use chrono::{DateTime, Utc};
use sqlx::error::BoxDynError;
use sqlx::postgres::{PgTypeInfo, PgValueFormat, PgValueRef};
use sqlx::{Decode, Postgres, Type, TypeInfo, ValueRef};

/// `time.Time{}` 零值（0001-01-01T00:00:00Z）的 Unix 秒。历史实现读到 NULL
/// 时间列时填的就是它，JSON 里序列化成 `"0001-01-01T00:00:00Z"`。
const ZERO_TIME_UNIX: i64 = -62_135_596_800;

/// `numeric` 列 → `f64`，NULL → `0.0`。对应历史实现把 NULL 读进 `float64` 的行为。
///
/// 配 `#[sqlx(try_from = "compat::Money")]` 使用，字段本身保持 `f64`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Money(pub f64);

/// `numeric` 列 → `Option<f64>`，对应历史的 `*float64` 字段（NULL 就是 `None`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoneyOpt(pub Option<f64>);

/// 文本列 → `String`，NULL → `""`。对应历史实现把 NULL 读进 `string` 的行为。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text(pub String);

/// 整数列 → `i64`，NULL → `0`。对应历史实现把 NULL 读进 `int` / `uint` 的行为。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Int(pub i64);

/// 布尔列 → `bool`，NULL → `false`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bool(pub bool);

/// `timestamptz` 列 → `DateTime<Utc>`，NULL → 零时间（0001-01-01T00:00:00Z）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ts(pub DateTime<Utc>);

macro_rules! impl_from {
    ($wrapper:ty => $target:ty) => {
        impl From<$wrapper> for $target {
            fn from(v: $wrapper) -> Self {
                v.0
            }
        }
    };
}

impl_from!(Money => f64);
impl_from!(MoneyOpt => Option<f64>);
impl_from!(Text => String);
impl_from!(Int => i64);
impl_from!(Bool => bool);
impl_from!(Ts => DateTime<Utc>);

// ── numeric ──────────────────────────────────────────────────────────────────

impl Type<Postgres> for Money {
    fn type_info() -> PgTypeInfo {
        PgTypeInfo::with_name("NUMERIC")
    }

    fn compatible(ty: &PgTypeInfo) -> bool {
        is_numeric_like(ty)
    }
}

impl Type<Postgres> for MoneyOpt {
    fn type_info() -> PgTypeInfo {
        PgTypeInfo::with_name("NUMERIC")
    }

    fn compatible(ty: &PgTypeInfo) -> bool {
        is_numeric_like(ty)
    }
}

impl<'r> Decode<'r, Postgres> for Money {
    fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
        Ok(Self(decode_f64(value)?.unwrap_or(0.0)))
    }
}

impl<'r> Decode<'r, Postgres> for MoneyOpt {
    fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
        Ok(Self(decode_f64(value)?))
    }
}

/// 同时认 `numeric` 和几种数值列，这样即便某天有人把某列改成 `double precision`
/// （或读到的是 `SUM(int)`），实体也不用跟着改。
fn is_numeric_like(ty: &PgTypeInfo) -> bool {
    matches!(
        ty.name(),
        "NUMERIC" | "FLOAT4" | "FLOAT8" | "INT2" | "INT4" | "INT8"
    )
}

fn decode_f64(value: PgValueRef<'_>) -> Result<Option<f64>, BoxDynError> {
    if value.is_null() {
        return Ok(None);
    }
    let name = value.type_info().name().to_owned();
    match value.format() {
        // 文本协议下所有数值列都是十进制字面量（含 "NaN"），直接交给 Rust 的
        // 正确舍入解析器。
        PgValueFormat::Text => Ok(Some(value.as_str()?.parse::<f64>()?)),
        PgValueFormat::Binary => {
            let buf = value.as_bytes()?;
            let v = match name.as_str() {
                "NUMERIC" => numeric_from_binary(buf)?,
                "FLOAT8" => f64::from_bits(u64::from_be_bytes(fixed::<8>(buf, "FLOAT8")?)),
                "FLOAT4" => f64::from(f32::from_bits(u32::from_be_bytes(fixed::<4>(
                    buf, "FLOAT4",
                )?))),
                "INT8" => i64::from_be_bytes(fixed::<8>(buf, "INT8")?) as f64,
                "INT4" => f64::from(i32::from_be_bytes(fixed::<4>(buf, "INT4")?)),
                "INT2" => f64::from(i16::from_be_bytes(fixed::<2>(buf, "INT2")?)),
                other => return Err(format!("compat::Money: 不支持的列类型 {other}").into()),
            };
            Ok(Some(v))
        }
    }
}

fn fixed<const N: usize>(buf: &[u8], what: &str) -> Result<[u8; N], BoxDynError> {
    buf.get(..N)
        .and_then(|s| <[u8; N]>::try_from(s).ok())
        .ok_or_else(|| format!("{what}: 期望 {N} 字节，实际 {}", buf.len()).into())
}

/// Postgres `numeric` 二进制线格式 → `f64`。
///
/// 头 8 字节是 `ndigits / weight / sign / dscale`（都是 big-endian i16），后面跟
/// `ndigits` 个 base-10000 的 i16 数位，最高位在前，值为
/// `Σ digits[i] * 10000^(weight - i)`。
///
/// 实现方式是先还原成十进制字面量再 `parse::<f64>()`：Rust 的浮点解析是正确舍入的，
/// 比自己做 `powi` 累加少一层误差。`dscale`（显示精度）对 `f64` 没有意义，忽略。
fn numeric_from_binary(buf: &[u8]) -> Result<f64, BoxDynError> {
    if buf.len() < 8 {
        return Err(format!("numeric: 头部需要 8 字节，实际 {}", buf.len()).into());
    }
    let ndigits = i16::from_be_bytes([buf[0], buf[1]]);
    let weight = i16::from_be_bytes([buf[2], buf[3]]);
    let sign = u16::from_be_bytes([buf[4], buf[5]]);

    match sign {
        // 0x0000 正 / 0x4000 负，其余是 PG 的特殊值。
        0x0000 | 0x4000 => {}
        0xC000 => return Ok(f64::NAN),
        0xD000 => return Ok(f64::INFINITY),
        0xF000 => return Ok(f64::NEG_INFINITY),
        other => return Err(format!("numeric: 未知符号位 0x{other:04X}").into()),
    }

    if ndigits < 0 {
        return Err(format!("numeric: ndigits 为负数 {ndigits}").into());
    }
    let ndigits = usize::from(ndigits.unsigned_abs());
    if buf.len() < 8 + ndigits * 2 {
        return Err(format!(
            "numeric: {ndigits} 个数位需要 {} 字节，实际 {}",
            8 + ndigits * 2,
            buf.len()
        )
        .into());
    }
    let digit = |i: usize| -> i16 { i16::from_be_bytes([buf[8 + i * 2], buf[9 + i * 2]]) };

    let mut s = String::with_capacity(16 + ndigits * 4);
    if sign == 0x4000 {
        s.push('-');
    }

    // 整数部分：weight 是最高位数位的 10000 次幂。weight < 0 表示整数部分是 0。
    if weight < 0 {
        s.push('0');
    } else {
        for i in 0..=usize::from(weight.unsigned_abs()) {
            let d = if i < ndigits { digit(i) } else { 0 };
            if !(0..10_000).contains(&d) {
                return Err(format!("numeric: 非法数位 {d}").into());
            }
            if i == 0 {
                let _ = write!(s, "{d}");
            } else {
                let _ = write!(s, "{d:04}");
            }
        }
    }

    // 小数部分：weight + 1 到 ndigits - 1；下标为负表示小数点后先补 4 个 0。
    let first_frac = i32::from(weight) + 1;
    if first_frac < ndigits as i32 {
        s.push('.');
        for i in first_frac..ndigits as i32 {
            if i < 0 {
                s.push_str("0000");
            } else {
                let d = digit(i as usize);
                if !(0..10_000).contains(&d) {
                    return Err(format!("numeric: 非法数位 {d}").into());
                }
                let _ = write!(s, "{d:04}");
            }
        }
    }

    Ok(s.parse::<f64>()?)
}

// ── text / int / bool / timestamptz ──────────────────────────────────────────

impl Type<Postgres> for Text {
    fn type_info() -> PgTypeInfo {
        <String as Type<Postgres>>::type_info()
    }

    fn compatible(ty: &PgTypeInfo) -> bool {
        <String as Type<Postgres>>::compatible(ty)
    }
}

impl<'r> Decode<'r, Postgres> for Text {
    fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
        if value.is_null() {
            return Ok(Self(String::new()));
        }
        Ok(Self(value.as_str()?.to_owned()))
    }
}

impl Type<Postgres> for Int {
    fn type_info() -> PgTypeInfo {
        <i64 as Type<Postgres>>::type_info()
    }

    fn compatible(ty: &PgTypeInfo) -> bool {
        matches!(ty.name(), "INT2" | "INT4" | "INT8")
    }
}

impl<'r> Decode<'r, Postgres> for Int {
    fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
        if value.is_null() {
            return Ok(Self(0));
        }
        let name = value.type_info().name().to_owned();
        match value.format() {
            PgValueFormat::Text => Ok(Self(value.as_str()?.parse::<i64>()?)),
            PgValueFormat::Binary => {
                let buf = value.as_bytes()?;
                let v = match name.as_str() {
                    "INT8" => i64::from_be_bytes(fixed::<8>(buf, "INT8")?),
                    "INT4" => i64::from(i32::from_be_bytes(fixed::<4>(buf, "INT4")?)),
                    "INT2" => i64::from(i16::from_be_bytes(fixed::<2>(buf, "INT2")?)),
                    other => return Err(format!("compat::Int: 不支持的列类型 {other}").into()),
                };
                Ok(Self(v))
            }
        }
    }
}

impl Type<Postgres> for Bool {
    fn type_info() -> PgTypeInfo {
        <bool as Type<Postgres>>::type_info()
    }

    fn compatible(ty: &PgTypeInfo) -> bool {
        <bool as Type<Postgres>>::compatible(ty)
    }
}

impl<'r> Decode<'r, Postgres> for Bool {
    fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
        if value.is_null() {
            return Ok(Self(false));
        }
        Ok(Self(<bool as Decode<Postgres>>::decode(value)?))
    }
}

impl Type<Postgres> for Ts {
    fn type_info() -> PgTypeInfo {
        <DateTime<Utc> as Type<Postgres>>::type_info()
    }

    fn compatible(ty: &PgTypeInfo) -> bool {
        <DateTime<Utc> as Type<Postgres>>::compatible(ty)
    }
}

impl<'r> Decode<'r, Postgres> for Ts {
    fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
        if value.is_null() {
            return Ok(Self(zero_time()));
        }
        Ok(Self(<DateTime<Utc> as Decode<Postgres>>::decode(value)?))
    }
}

/// `time.Time{}` 零值的等价物。历史实现把 NULL 时间列读成它，Rust 侧保持一致。
pub fn zero_time() -> DateTime<Utc> {
    DateTime::from_timestamp(ZERO_TIME_UNIX, 0).unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
}

#[cfg(test)]
mod tests;
