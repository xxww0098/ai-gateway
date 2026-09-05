//! Cache-id sanitizer shared as a *charset*, not as a session.
//!
//! Each family still owns the fallback constant and whether a model suffix is
//! appended. This function only answers "can this string go on the wire as an
//! id". Empty / whitespace-only input is absent, never a generated timestamp.

/// Keep ASCII alphanumerics plus `. _ : -`; anything else becomes `-`.
/// Caps at 64. Empty after trim/clean is `None`.
#[must_use]
pub fn sanitize_cache_id(key: &str) -> Option<String> {
    let cleaned: String = key
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.chars().take(64).collect())
    }
}

/// First present, sanitizable candidate. `None` if every candidate is empty.
#[must_use]
pub fn first_cache_id<'a, I>(candidates: I) -> Option<String>
where
    I: IntoIterator<Item = Option<&'a str>>,
{
    candidates.into_iter().flatten().find_map(sanitize_cache_id)
}

#[cfg(test)]
mod tests;
