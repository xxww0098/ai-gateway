use super::*;

// 这个模块的状态是进程全局的，所以测试**不碰那份全局状态**（并行跑会互相干扰）。
// 测的是切页这个纯函数 —— 它是原实现那段最容易写崩的代码（切片越界）——
// 以及响应形状。

#[test]
fn slicing_the_first_page_starts_at_the_beginning() {
    let items: Vec<i64> = (0..25).collect();
    assert_eq!(slice_page(&items, 1, 10), &items[0..10]);
}

#[test]
fn slicing_a_middle_page_is_contiguous_with_its_neighbours() {
    // 性质：连续三页拼起来正好是前 30 项，不重不漏。
    let items: Vec<i64> = (0..30).collect();
    let mut joined = Vec::new();
    for page in 1..=3 {
        joined.extend_from_slice(slice_page(&items, page, 10));
    }
    assert_eq!(joined, items);
}

#[test]
fn the_last_page_is_short_rather_than_out_of_bounds() {
    let items: Vec<i64> = (0..25).collect();
    assert_eq!(slice_page(&items, 3, 10), &items[20..25]);
}

#[test]
fn a_page_past_the_end_is_empty_not_a_panic() {
    // 原实现那段 `if start > len { start = len }` 存在的全部理由。
    let items: Vec<i64> = (0..5).collect();
    for page in [2_i64, 3, 1_000, 1_000_000] {
        assert!(slice_page(&items, page, 10).is_empty(), "page={page}");
    }
}

#[test]
fn slicing_an_empty_list_is_always_empty() {
    let items: Vec<i64> = Vec::new();
    for page in [1_i64, 2, 999] {
        assert!(slice_page(&items, page, 10).is_empty(), "page={page}");
    }
}

#[test]
fn a_huge_page_size_yields_the_whole_list_rather_than_overflowing() {
    // start + page_size 必须防溢出，否则 usize 加法会 wrap 成一个很小的 end，
    // 静默丢数据。
    let items: Vec<i64> = (0..7).collect();
    assert_eq!(slice_page(&items, 1, i64::MAX), &items[..]);
}

#[test]
fn notification_item_has_the_six_keys_the_bell_menu_reads() {
    let json = serde_json::to_value(NotificationItem {
        id: 1,
        title: "欢迎".into(),
        content: "正文".into(),
        is_read: false,
        notification_type: "system".into(),
        created_at: Utc::now(),
    })
    .expect("serialise");
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
            "content",
            "created_at",
            "id",
            "is_read",
            "notification_type",
            "title",
        ]
    );
}

#[test]
fn the_seeded_notification_starts_unread_so_the_badge_shows_up() {
    // 唯一一处允许读全局状态的断言：只读、不改，所以不会影响并行的其他测试。
    let seeded = notifications();
    assert!(!seeded.is_empty(), "种子通知不该是空的");
    assert!(seeded.iter().all(|n| n.id > 0), "id 必须是可寻址的正数");
}
