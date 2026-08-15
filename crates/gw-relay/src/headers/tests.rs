//! OWNER: worker `relay-core`。
//!
//! 规范 2.11：**不许把源码里的字面量抄进断言**。下面每一条断言的期望值要么来自
//! 测试自己造的输入，要么是「个数」「相等/不等」这类不写死在源码里的性质。

use http::header::{ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, CONTENT_TYPE, SET_COOKIE};
use http::{HeaderMap, HeaderName, HeaderValue};

use super::{copy_preserving_multivalue, upstream_headers};
use crate::contract::{Credential, RelayTransportError};

const CARRIERS: [&str; 3] = ["authorization", "x-api-key", "x-goog-api-key"];

fn map(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (name, value) in pairs {
        out.append(
            HeaderName::from_bytes(name.as_bytes()).expect("测试用的 header 名"),
            HeaderValue::from_str(value).expect("测试用的 header 值"),
        );
    }
    out
}

fn bearer() -> Credential {
    Credential::Bearer("upstream-token".to_owned())
}

/// 缺陷 #7 的守护测试：同名多值不许被折叠成最后一个。
///
/// 把 `append` 改回 `insert` 这条就红 —— 拿到的只剩一条 cookie。
#[test]
fn multi_valued_headers_survive_the_copy() {
    let cookies = ["__cf_bm=abc; Path=/", "_cfuvid=xyz; Path=/"];
    let mut src = HeaderMap::new();
    for cookie in cookies {
        src.append(
            SET_COOKIE,
            HeaderValue::from_str(cookie).expect("测试用的 cookie"),
        );
    }

    let mut dst = HeaderMap::new();
    copy_preserving_multivalue(&mut dst, &src);

    let got: Vec<&HeaderValue> = dst.get_all(SET_COOKIE).iter().collect();
    let want: Vec<&HeaderValue> = src.get_all(SET_COOKIE).iter().collect();
    assert_eq!(got, want, "多值 header 的条数与顺序都必须原样");
}

/// 缺陷 #10 的守护测试：`expect` 与整套逐跳头必须被代理自己消费，
/// 而**普通 header 一个都不许丢**。
#[test]
fn hop_by_hop_headers_are_consumed_and_the_rest_survives() {
    let consumed = [
        ("expect", "100-continue"),
        ("connection", "keep-alive"),
        ("keep-alive", "timeout=5"),
        ("te", "trailers"),
        ("trailer", "x-checksum"),
        ("transfer-encoding", "chunked"),
        ("upgrade", "websocket"),
        ("proxy-authenticate", "Basic realm=\"x\""),
        ("proxy-authorization", "Basic dXNlcjpwdw=="),
        ("host", "client.example.test"),
        ("content-length", "17"),
    ];
    let kept = [
        ("x-custom-trace", "abc123"),
        ("cookie", "session=1"),
        ("content-type", "application/json"),
    ];

    let mut pairs = consumed.to_vec();
    pairs.extend_from_slice(&kept);
    let src = map(&pairs);

    let mut dst = HeaderMap::new();
    copy_preserving_multivalue(&mut dst, &src);

    for (name, _) in consumed {
        assert!(!dst.contains_key(name), "{name} 必须由代理自己消费");
    }
    for (name, _) in kept {
        assert_eq!(dst.get(name), src.get(name), "{name} 必须原样过");
    }
}

/// 观察 #17 的守护测试：本层读过的凭证载体，本层负责剥掉。
///
/// 把 `out.remove(...)` 那三行去掉这条就红 —— 租户的 key 跟着出站了。
#[test]
fn inbound_credential_carriers_never_reach_the_upstream() {
    let inbound = map(&[
        ("authorization", "Bearer tenant-token"),
        ("x-api-key", "tenant-anthropic-key"),
        ("x-goog-api-key", "tenant-google-key"),
    ]);

    let out = upstream_headers(
        &inbound,
        &Credential::XApiKey("upstream-key".to_owned()),
        false,
    )
    .expect("凭证合法");

    for carrier in CARRIERS {
        for value in out.get_all(carrier) {
            assert!(
                inbound.get_all(carrier).iter().all(|old| old != value),
                "{carrier} 上还留着入站的值"
            );
        }
    }
}

/// 每种凭证只落在它自己的载体上，另外两个必须是空的。
#[test]
fn each_credential_kind_occupies_exactly_one_carrier() {
    let token = "the-secret";
    for cred in [
        Credential::Bearer(token.to_owned()),
        Credential::XApiKey(token.to_owned()),
        Credential::GoogleApiKey(token.to_owned()),
    ] {
        let out = upstream_headers(&HeaderMap::new(), &cred, false).expect("凭证合法");
        let present: Vec<&str> = CARRIERS
            .into_iter()
            .filter(|name| out.contains_key(*name))
            .collect();
        assert_eq!(present.len(), 1, "{cred:?} 落在了 {present:?}");
        let value = out.get(present[0]).expect("刚判过存在");
        assert!(
            value.as_bytes().ends_with(token.as_bytes()),
            "{cred:?} 没把 token 带出去"
        );
    }
}

/// 观察 #18 的守护测试：`Accept` 既不覆盖也不发明。`content-type` 同理。
#[test]
fn accept_and_content_type_are_neither_rewritten_nor_invented() {
    let inbound = map(&[
        ("accept", "application/xml, text/plain;q=0.1"),
        ("content-type", "application/x-ndjson"),
    ]);
    let out = upstream_headers(&inbound, &bearer(), false).expect("凭证合法");
    assert_eq!(out.get(ACCEPT), inbound.get(ACCEPT));
    assert_eq!(out.get(CONTENT_TYPE), inbound.get(CONTENT_TYPE));

    let bare = upstream_headers(&HeaderMap::new(), &bearer(), false).expect("凭证合法");
    assert!(!bare.contains_key(ACCEPT), "客户端没给就不该替它编一个");
    assert!(!bare.contains_key(CONTENT_TYPE), "同上");
}

/// 缺陷 #5 的守护测试（默认模式）：客户端要的任何一种压缩都不许到达上游，
/// 否则旁路 usage 读到的是 gzip 魔数，非流式请求全部按估算计费。
#[test]
fn client_content_codings_do_not_reach_the_upstream_by_default() {
    let codings = ["gzip", "deflate", "br", "zstd"];
    let inbound = map(&[("accept-encoding", &codings.join(", "))]);

    let out = upstream_headers(&inbound, &bearer(), false).expect("凭证合法");

    let values: Vec<&HeaderValue> = out.get_all(ACCEPT_ENCODING).iter().collect();
    assert_eq!(values.len(), 1, "只能有一个 accept-encoding");
    let sent = values[0].to_str().expect("ASCII");
    for coding in codings {
        assert!(!sent.contains(coding), "上游仍然被要求 {coding}");
    }
}

/// 缺陷 #5 的另一半：显式选了「保留客户端压缩」就必须**逐字节**原样透传，
/// 且客户端没发时不许替它发明一个。
#[test]
fn client_content_codings_pass_through_when_explicitly_chosen() {
    let inbound = map(&[("accept-encoding", "gzip, deflate, br")]);
    let out = upstream_headers(&inbound, &bearer(), true).expect("凭证合法");
    assert_eq!(out.get(ACCEPT_ENCODING), inbound.get(ACCEPT_ENCODING));

    let bare = upstream_headers(&HeaderMap::new(), &bearer(), true).expect("凭证合法");
    assert!(!bare.contains_key(ACCEPT_ENCODING));
}

/// 凭证里混进 CR/LF 是 header 注入的入口，必须在这一层被挡下。
#[test]
fn a_credential_that_is_not_a_header_value_is_rejected() {
    let err = upstream_headers(
        &HeaderMap::new(),
        &Credential::XApiKey("good\r\nx-injected: evil".to_owned()),
        false,
    )
    .expect_err("含 CRLF 的凭证必须被拒");
    assert!(matches!(err, RelayTransportError::BadCredential));
}

/// 出站凭证必须标记 sensitive，否则会进 hyper 的 debug 日志。
#[test]
fn the_outbound_credential_is_marked_sensitive() {
    let out = upstream_headers(&HeaderMap::new(), &bearer(), false).expect("凭证合法");
    let value = out
        .get(AUTHORIZATION)
        .expect("Bearer 落在 authorization 上");
    assert!(value.is_sensitive());
}
