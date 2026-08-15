use super::*;

// 规则 2.11：不断言源码里的字面量，断言那些**不写死在源码里的性质**
// —— 夹紧的边界关系、ceil 的定义、解析失败的回退方向。

#[test]
fn query_int_falls_back_when_unparsable() {
    // 缺参 / 空串 / 非数字 / 尾部有杂质，四种都必须落到 def 而不是 0。
    for raw in [None, Some(""), Some("abc"), Some("12x"), Some("1.5")] {
        assert_eq!(query_int(raw, 7, 1, 100), 7, "raw={raw:?}");
    }
}

#[test]
fn query_int_result_always_within_bounds() {
    // 性质：无论输入什么，结果都落在 [min, max] 内 —— 连 def 越界也要被夹回。
    let (min, max) = (3_i64, 9_i64);
    for raw in ["-100", "0", "1", "5", "9", "10", "999999999999999999999"] {
        let v = query_int(Some(raw), 100, min, max);
        assert!((min..=max).contains(&v), "raw={raw} -> {v}");
    }
    assert_eq!(query_int(None, 100, min, max), max);
    assert_eq!(query_int(None, -100, min, max), min);
}

#[test]
fn query_int_is_monotonic_in_its_input() {
    let mut prev = i64::MIN;
    for raw in ["1", "2", "3", "4", "5"] {
        let v = query_int(Some(raw), 1, 2, 4);
        assert!(v >= prev, "clamp must not invert order at {raw}");
        prev = v;
    }
}

#[test]
fn parse_id_rejects_zero_and_non_positive() {
    // 原实现的 parseUintParam 把 0 与解析失败一视同仁，因为自增主键从 1 起。
    for raw in ["0", "-1", "", " ", "abc", "1.0", "0x1"] {
        assert!(parse_id(raw).is_none(), "raw={raw:?} must not resolve");
    }
}

#[test]
fn parse_id_round_trips_positive_ids() {
    for id in [1_i64, 42, i64::from(u32::MAX), i64::MAX] {
        assert_eq!(parse_id(&id.to_string()), Some(id));
    }
}

#[test]
fn parse_id_rejects_values_beyond_i64() {
    // u64 能装下但 i64 装不下 —— 主键列是 bigint，越界就不是一个可能的 id。
    let beyond = u64::MAX.to_string();
    assert!(parse_id(&beyond).is_none());
}

#[test]
fn total_pages_satisfies_ceiling_definition() {
    // 性质：pages 是能装下 total 的最小页数。
    for page_size in [1_i64, 7, 20, 100] {
        for total in [0_i64, 1, 6, 7, 8, 99, 100, 101, 12_345] {
            let pages = total_pages(total, page_size);
            assert!(pages * page_size >= total, "{total}/{page_size} 装不下");
            assert!(
                pages == 0 || (pages - 1) * page_size < total,
                "{total}/{page_size} 多出了一整页"
            );
        }
    }
}

#[test]
fn total_pages_of_empty_set_is_zero() {
    assert_eq!(total_pages(0, 20), 0);
}

#[test]
fn offset_of_first_page_is_zero_and_grows_by_page_size() {
    let size = 15;
    assert_eq!(offset(1, size), 0);
    for page in 1..5 {
        assert_eq!(offset(page + 1, size) - offset(page, size), size);
    }
}

#[test]
fn page_envelope_reports_the_page_it_was_built_for() {
    let page = Page::new(vec![1, 2, 3], 2, 3, 10);
    assert_eq!(page.page, 2);
    assert_eq!(page.page_size, 3);
    assert_eq!(page.total, 10);
    assert_eq!(page.total_pages, total_pages(10, 3));
}

#[test]
fn page_envelope_serialises_all_five_keys() {
    // 前端读的就是这五个键；少一个都是破坏性变更。
    let json = serde_json::to_value(Page::new(vec![1], 1, 20, 1)).expect("serialise");
    let obj = json.as_object().expect("object");
    let mut keys: Vec<_> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["items", "page", "page_size", "total", "total_pages"]);
}

#[test]
fn list_page_envelope_omits_total_pages() {
    let json = serde_json::to_value(ListPage::new(vec![1], 1, 1, 20)).expect("serialise");
    let obj = json.as_object().expect("object");
    assert!(
        !obj.contains_key("total_pages"),
        "ListPage 多了 total_pages 就不再是原实现那个信封了"
    );
    let mut keys: Vec<_> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["items", "page", "page_size", "total"]);
}
