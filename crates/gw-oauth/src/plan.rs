//! Plan-slug → operator-facing label. Family-specific collisions stay here
//! (`glm` `pro` → Pro, Codex `pro` → Pro 20x).

use crate::Family;

/// Pretty-print a vendor plan slug. Unknown values are title-cased, not dropped.
#[must_use]
pub fn format_plan_label(raw: &str, family: Family) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let slug = slug_of(trimmed);
    match family {
        Family::Codex => codex_label(&slug).map(str::to_owned).unwrap_or_else(|| title(trimmed)),
        Family::Grok => grok_label(&slug).map(str::to_owned).unwrap_or_else(|| title(trimmed)),
        Family::Glm => glm_label(&slug).map(str::to_owned).unwrap_or_else(|| title(trimmed)),
        Family::Kiro => kiro_label(&slug).map(str::to_owned).unwrap_or_else(|| title(trimmed)),
        Family::Copilot => copilot_label(&slug).map(str::to_owned).unwrap_or_else(|| title(trimmed)),
        _ => title(trimmed),
    }
}

fn slug_of(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .replace('+', "plus")
        .replace(['-', ' '], "_")
}

fn title(value: &str) -> String {
    value
        .replace(['_', '-'], " ")
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn codex_label(slug: &str) -> Option<&'static str> {
    Some(match slug {
        "plus" | "chatgpt_plus" => "Plus",
        "pro" | "chatgpt_pro" | "pro20x" | "pro_20x" => "Pro 20x",
        "prolite" | "pro_lite" | "pro5x" | "pro_5x" => "Pro 5x",
        "team" | "chatgpt_team" => "Team",
        "free" | "free_plan" => "Free",
        _ => return None,
    })
}

fn grok_label(slug: &str) -> Option<&'static str> {
    Some(match slug {
        "supergrok" | "super_grok" => "SuperGrok",
        "xpremiumplus" | "x_premium_plus" | "x_premiumplus" => "X Premium+",
        "xpremium" | "x_premium" => "X Premium",
        "supergrokheavy" | "super_grok_heavy" | "supergrokpro" => "SuperGrok Heavy",
        "free" => "Free",
        _ => return None,
    })
}

fn glm_label(slug: &str) -> Option<&'static str> {
    Some(match slug {
        "lite" => "Lite",
        "pro" => "Pro",
        "max" => "Max",
        _ => return None,
    })
}

fn kiro_label(slug: &str) -> Option<&'static str> {
    Some(match slug {
        "free" => "Free",
        "pro" => "Pro",
        "proplus" | "pro_plus" => "Pro+",
        _ => return None,
    })
}

fn copilot_label(slug: &str) -> Option<&'static str> {
    Some(match slug {
        "free" => "Free",
        "pro" => "Pro",
        "proplus" | "pro_plus" => "Pro+",
        "business" => "Business",
        "enterprise" => "Enterprise",
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
