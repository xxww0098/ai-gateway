use super::format_plan_label;
use crate::Family;

#[test]
fn pro_slug_is_family_specific() {
    assert_eq!(format_plan_label("pro", Family::Codex), "Pro 20x");
    assert_eq!(format_plan_label("pro", Family::Glm), "Pro");
    assert_eq!(format_plan_label("prolite", Family::Codex), "Pro 5x");
}

#[test]
fn unknown_slug_is_not_dropped() {
    let label = format_plan_label("custom_tier", Family::Ollama);
    assert!(!label.is_empty());
    assert_ne!(label, "Pro 20x");
}
