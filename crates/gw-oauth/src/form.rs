//! `application/x-www-form-urlencoded`, sorted by key.

/// Encode `key=value` pairs. Space becomes `+`.
#[must_use]
pub fn encode(params: &[(&str, String)]) -> String {
    let mut sorted: Vec<&(&str, String)> = params.iter().collect();
    sorted.sort_by_key(|(key, _)| *key);
    sorted
        .iter()
        .map(|(key, value)| format!("{}={}", urlencode(key.as_bytes()), urlencode(value.as_bytes())))
        .collect::<Vec<_>>()
        .join("&")
}

fn urlencode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for byte in bytes {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}
