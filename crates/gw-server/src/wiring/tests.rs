//! Unit tests for the parts of wiring that need no Postgres/Redis: the
//! provider gating and the seeded `sdk_config` document.
//!
//! `wire` itself is exercised end-to-end against the real stack, not mocked
//! here — every one of its steps is an I/O call, so a test double would only
//! assert that the calls are in the order the source already shows.

use gw_config::{Config, SdkConfig, SdkProviderConfig};

use super::{build_providers, sdk_seed_config};

fn complete(base_url: &str) -> SdkProviderConfig {
    SdkProviderConfig {
        base_url: base_url.to_owned(),
        api_key: "sk-test-credential".to_owned(),
        enabled: true,
    }
}

fn names(sdk: &SdkConfig) -> Vec<&'static str> {
    build_providers(sdk)
        .expect("providers build")
        .iter()
        .map(|p| p.name())
        .collect()
}

/// Claude / Gemini / Codex / Vertex must exist even with an entirely blank
/// config: a persisted `auth_records` row can supply the credential at request
/// time, and without the executor that row has nothing to dispatch through.
#[test]
fn blank_config_still_yields_the_db_backed_upstreams() {
    let built = names(&SdkConfig::default());

    for expected in ["claude", "gemini", "codex", "vertex", "xai", "kiro"] {
        assert!(
            built.contains(&expected),
            "{expected} missing from {built:?}",
        );
    }
}

/// The OpenAI-compatible upstream is the one exception: it has no default host,
/// so an incomplete config leaves nothing to build. Completing it adds exactly
/// one provider and disturbs none of the others.
#[test]
fn openai_appears_only_once_its_config_is_complete() {
    let without = names(&SdkConfig::default());

    let mut sdk = SdkConfig {
        openai: complete("https://api.example.test/v1"),
        ..SdkConfig::default()
    };
    let with = names(&sdk);

    assert_eq!(with.len(), without.len() + 1);
    for name in &without {
        assert!(with.contains(name), "{name} lost when openai was enabled");
    }

    // Enabled but credential-less is still incomplete, so it drops back out.
    sdk.openai.api_key.clear();
    assert_eq!(names(&sdk).len(), without.len());
}

/// An unusable base URL fails at wiring time rather than on the first request —
/// the reason the constructors fail closed, and the reason `wire` returns a Result.
#[test]
fn unparseable_base_url_fails_the_build() {
    let sdk = SdkConfig {
        claude: complete("not-a-url"),
        ..SdkConfig::default()
    };

    assert!(build_providers(&sdk).is_err());
}

/// The seeded `provider_configs.sdk_config` row is world-readable through the
/// admin panel, so it must carry base URLs and flags only. A credential
/// reaching it would be a secret at rest in a place nothing encrypts.
#[test]
fn the_seeded_sdk_config_carries_no_credential() {
    const SECRET: &str = "sk-must-not-be-persisted";

    let provider = SdkProviderConfig {
        base_url: "https://api.example.test".to_owned(),
        api_key: SECRET.to_owned(),
        enabled: true,
    };
    let config = Config {
        sdk: SdkConfig {
            base_url: "https://legacy.example.test".to_owned(),
            api_key: SECRET.to_owned(),
            openai: provider.clone(),
            openai_compatible: provider.clone(),
            claude: provider.clone(),
            gemini: provider.clone(),
            codex: provider.clone(),
            vertex: provider,
            ..SdkConfig::default()
        },
        ..Config::default()
    };

    let document =
        serde_json::to_string(&sdk_seed_config(&config)).expect("the seed document serializes");

    assert!(!document.contains(SECRET), "credential leaked: {document}");
    assert!(document.contains("https://api.example.test"));
}

/// Every provider the config knows about gets an entry, so an operator editing
/// the seeded row in the panel sees the same six slots the config has.
#[test]
fn the_seeded_sdk_config_covers_every_configured_provider() {
    let seeded = sdk_seed_config(&Config::default());

    let mut expected: Vec<String> = serde_json::to_value(SdkConfig::default())
        .expect("the sdk config serializes")
        .as_object()
        .expect("an object")
        .keys()
        .filter(|key| !matches!(key.as_str(), "base_url" | "api_key" | "timeout_seconds"))
        .cloned()
        .collect();
    expected.sort_unstable();

    // `providers` is a BTreeMap, so its keys already come out sorted.
    let got: Vec<String> = seeded.providers.keys().cloned().collect();

    assert_eq!(got, expected);
}
