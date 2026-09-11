use std::collections::HashSet;

use super::{MODELS, Reasoning, context_window_of};
use crate::kiro::cache::conversation_id;

#[test]
fn fallback_ids_are_unique_and_isolate_conversations() {
    let ids: Vec<&str> = MODELS.iter().map(|row| row.id).collect();
    let unique: HashSet<&str> = ids.iter().copied().collect();
    assert_eq!(unique.len(), ids.len());
    assert!(ids.len() >= 18);
    assert!(ids.contains(&"auto"));
    let pins: HashSet<String> = MODELS
        .iter()
        .map(|row| conversation_id(&serde_json::json!({"model": row.id}), None))
        .collect();
    assert_eq!(pins.len(), MODELS.len());
    for row in MODELS {
        let pin = conversation_id(&serde_json::json!({"model": row.id}), None);
        assert!(pin.contains(row.id), "{pin}");
        assert!(!pin.chars().all(|c| c.is_ascii_digit()), "{pin}");
    }
}

#[test]
fn vision_and_reasoning_split_matches_the_public_table() {
    let gpt = MODELS
        .iter()
        .find(|row| row.id == "gpt-5.6-sol")
        .expect("gpt");
    let opus = MODELS
        .iter()
        .find(|row| row.id == "claude-opus-5")
        .expect("opus");
    let sonnet46 = MODELS
        .iter()
        .find(|row| row.id == "claude-sonnet-4.6")
        .expect("4.6");
    let glm = MODELS.iter().find(|row| row.id == "glm-5").expect("glm");
    assert!(gpt.vision && opus.vision);
    assert!(!glm.vision);
    assert_eq!(gpt.reasoning, Reasoning::Gpt);
    assert_eq!(opus.reasoning, Reasoning::ClaudeXhigh);
    assert_eq!(sonnet46.reasoning, Reasoning::Claude);
    assert_eq!(glm.reasoning, Reasoning::None);
    assert!(context_window_of("claude-opus-5") > context_window_of("claude-haiku-4.5"));
    assert!(context_window_of("deepseek-3.2") < context_window_of("claude-sonnet-5"));
}
