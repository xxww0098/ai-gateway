use super::{
    CLAUDE_DEFAULT_BASE_URL, CODEX_DEFAULT_BASE_URL, CODEX_METADATA_ACCESS_TOKEN,
    GEMINI_DEFAULT_BASE_URL, PROVIDER_CLAUDE, PROVIDER_CODEX, PROVIDER_OPENAI, PROVIDER_VERTEX,
    RUNTIME_SOURCE, RuntimeAuthStore, VERTEX_METADATA_SERVICE_ACCOUNT, build_runtime_upstreams,
    build_runtime_upstreams_at,
};
use crate::{
    AuthError,
    record::{AuthRecord, AuthStatus, AuthStore, BASE_URL_ATTRIBUTE, SOURCE_ATTRIBUTE},
};
use chrono::Utc;
use gw_config::{SdkConfig, SdkProviderConfig};
use std::sync::{Arc, Mutex};

fn provider(base_url: &str, api_key: &str) -> SdkProviderConfig {
    SdkProviderConfig {
        base_url: base_url.to_owned(),
        api_key: api_key.to_owned(),
        enabled: true,
    }
}

fn find<'a>(auths: &'a [AuthRecord], provider: &str) -> Option<&'a AuthRecord> {
    auths.iter().find(|auth| auth.provider == provider)
}

// ---------------------------------------------------------------------------
// BuildRuntimeUpstreams
// ---------------------------------------------------------------------------

#[test]
fn an_empty_config_seeds_nothing() {
    let auths = build_runtime_upstreams(&SdkConfig::default()).expect("no provider to validate");

    assert!(
        auths.is_empty(),
        "an unconfigured gateway must not advertise upstreams it cannot reach"
    );
}

#[test]
fn every_seeded_credential_is_runtime_only_active_and_attributed() {
    let cfg = SdkConfig {
        claude: provider("https://api.anthropic.com", "sk-claude"),
        gemini: provider("https://generativelanguage.googleapis.com", "sk-gemini"),
        codex: provider("", "codex-access-token"),
        vertex: provider("", "{\"client_email\":\"x\"}"),
        openai: provider("https://api.example.com", "sk-openai"),
        ..SdkConfig::default()
    };

    let auths = build_runtime_upstreams(&cfg).expect("all base URLs are valid");

    assert_eq!(
        auths.len(),
        5,
        "five providers configured, five credentials"
    );
    for auth in &auths {
        assert!(
            auth.is_runtime_only(),
            "{} would otherwise be persisted and outlive its config entry",
            auth.id
        );
        assert_eq!(auth.attribute(SOURCE_ATTRIBUTE), Some(RUNTIME_SOURCE));
        assert_eq!(auth.status, AuthStatus::Active);
        assert!(auth.is_usable());
        assert!(auth.id.starts_with("cpa-gateway-"));
        assert!(!auth.label.is_empty());
    }
}

#[test]
fn the_legacy_top_level_credential_still_seeds_the_openai_upstream() {
    // Deployments that predate the nested `sdk.openai:` block only set
    // `sdk.base_url` / `sdk.api_key`.
    let cfg = SdkConfig {
        base_url: "https://legacy.example.com".to_owned(),
        api_key: "sk-legacy".to_owned(),
        ..SdkConfig::default()
    };

    let auths = build_runtime_upstreams(&cfg).expect("the legacy base URL is valid");
    let openai = find(&auths, PROVIDER_OPENAI).expect("the legacy upstream is seeded");

    assert_eq!(
        openai.attribute(BASE_URL_ATTRIBUTE),
        Some("https://legacy.example.com")
    );
}

#[test]
fn a_provider_without_a_credential_is_left_to_its_persisted_rows() {
    let cfg = SdkConfig {
        claude: SdkProviderConfig {
            base_url: "https://api.anthropic.com".to_owned(),
            api_key: String::new(),
            enabled: true,
        },
        ..SdkConfig::default()
    };

    let auths = build_runtime_upstreams(&cfg).expect("the base URL is valid");

    assert!(
        find(&auths, PROVIDER_CLAUDE).is_none(),
        "a base URL alone is not a credential"
    );
}

#[test]
fn a_token_provider_without_a_base_url_falls_back_to_the_provider_default() {
    // Codex and Vertex are the only providers that can be seeded without a
    // base_url — Complete() demands one from the others — so they are the only
    // ones whose attribute can show the built-in default.
    let cfg = SdkConfig {
        codex: provider("", "codex-token"),
        ..SdkConfig::default()
    };

    let auths = build_runtime_upstreams(&cfg).expect("defaults are valid");

    assert_eq!(
        find(&auths, PROVIDER_CODEX).and_then(|a| a.attribute(BASE_URL_ATTRIBUTE)),
        Some(CODEX_DEFAULT_BASE_URL)
    );
}

#[test]
fn a_key_without_a_base_url_does_not_seed_a_url_bound_provider() {
    let cfg = SdkConfig {
        claude: provider("", "sk-claude"),
        gemini: provider("", "sk-gemini"),
        ..SdkConfig::default()
    };

    let auths = build_runtime_upstreams(&cfg).expect("nothing to validate");

    assert!(
        auths.is_empty(),
        "Complete() wants enabled + base_url + key; a half-filled block is a config error, \
         not a silent fallback to {CLAUDE_DEFAULT_BASE_URL} / {GEMINI_DEFAULT_BASE_URL}"
    );
}

#[test]
fn a_configured_base_url_is_trimmed() {
    let cfg = SdkConfig {
        claude: provider("  https://proxy.example.com///  ", "sk-claude"),
        ..SdkConfig::default()
    };

    let auths = build_runtime_upstreams(&cfg).expect("the base URL is valid");

    assert_eq!(
        find(&auths, PROVIDER_CLAUDE).and_then(|a| a.attribute(BASE_URL_ATTRIBUTE)),
        Some("https://proxy.example.com"),
        "trailing slashes are stripped before the executor appends its path"
    );
}

#[test]
fn token_bearing_providers_carry_their_secret_in_metadata() {
    let cfg = SdkConfig {
        codex: provider("", "codex-access-token"),
        vertex: provider("", "{\"client_email\":\"svc@example.com\"}"),
        ..SdkConfig::default()
    };

    let auths = build_runtime_upstreams(&cfg).expect("defaults are valid");

    let codex = find(&auths, PROVIDER_CODEX).expect("codex is seeded");
    assert_eq!(
        codex.metadata[CODEX_METADATA_ACCESS_TOKEN],
        "codex-access-token"
    );

    let vertex = find(&auths, PROVIDER_VERTEX).expect("vertex is seeded");
    assert_eq!(
        vertex.metadata[VERTEX_METADATA_SERVICE_ACCOUNT],
        "{\"client_email\":\"svc@example.com\"}"
    );
    assert_eq!(
        vertex.attribute(BASE_URL_ATTRIBUTE),
        Some(""),
        "vertex derives its host from the request region"
    );
}

#[test]
fn a_disabled_provider_carrying_a_token_is_still_seeded() {
    // Codex/Vertex gate on Configured(), not Complete(): their credential is an
    // access token, and they are seeded even with `enabled: false`.
    let cfg = SdkConfig {
        codex: SdkProviderConfig {
            base_url: String::new(),
            api_key: "codex-access-token".to_owned(),
            enabled: false,
        },
        claude: SdkProviderConfig {
            base_url: String::new(),
            api_key: "sk-claude".to_owned(),
            enabled: false,
        },
        ..SdkConfig::default()
    };

    let auths = build_runtime_upstreams(&cfg).expect("defaults are valid");

    assert!(find(&auths, PROVIDER_CODEX).is_some());
    assert!(
        find(&auths, PROVIDER_CLAUDE).is_none(),
        "claude needs enabled + base_url + key"
    );
}

#[test]
fn a_base_url_that_is_not_absolute_fails_startup() {
    for bad in ["api.anthropic.com", "https://", "/v1", "://host"] {
        let cfg = SdkConfig {
            claude: provider(bad, "sk-claude"),
            ..SdkConfig::default()
        };

        assert!(
            matches!(
                build_runtime_upstreams(&cfg),
                Err(AuthError::InvalidBaseUrl { .. })
            ),
            "{bad:?} must not be accepted as an upstream"
        );
    }
}

#[test]
fn a_broken_base_url_is_rejected_even_without_a_credential() {
    // The claude/gemini/codex executors are always constructed, so a typo in a
    // base URL fails startup whether or not that provider has a key.
    let cfg = SdkConfig {
        gemini: SdkProviderConfig {
            base_url: "not a url".to_owned(),
            api_key: String::new(),
            enabled: false,
        },
        ..SdkConfig::default()
    };

    assert!(matches!(
        build_runtime_upstreams(&cfg),
        Err(AuthError::InvalidBaseUrl { .. })
    ));
}

#[test]
fn seeding_is_deterministic_for_a_given_clock() {
    let cfg = SdkConfig {
        claude: provider("", "sk-claude"),
        codex: provider("", "codex-token"),
        ..SdkConfig::default()
    };
    let now = Utc::now();

    assert_eq!(
        build_runtime_upstreams_at(&cfg, now).expect("valid"),
        build_runtime_upstreams_at(&cfg, now).expect("valid"),
        "reloads must not churn credential ids or timestamps"
    );
}

// ---------------------------------------------------------------------------
// RuntimeAuthStore
// ---------------------------------------------------------------------------

#[derive(Default)]
struct MemoryStore {
    records: Mutex<Vec<AuthRecord>>,
    saved: Mutex<Vec<String>>,
    deleted: Mutex<Vec<String>>,
}

impl MemoryStore {
    fn with(records: Vec<AuthRecord>) -> Arc<Self> {
        Arc::new(Self {
            records: Mutex::new(records),
            ..Self::default()
        })
    }
}

#[async_trait::async_trait]
impl AuthStore for MemoryStore {
    async fn list(&self) -> anyhow::Result<Vec<AuthRecord>> {
        Ok(self.records.lock().expect("not poisoned").clone())
    }

    async fn get(&self, id: &str) -> anyhow::Result<Option<AuthRecord>> {
        Ok(self
            .records
            .lock()
            .expect("not poisoned")
            .iter()
            .find(|record| record.id == id)
            .cloned())
    }

    async fn save(&self, record: &AuthRecord) -> anyhow::Result<()> {
        self.saved
            .lock()
            .expect("not poisoned")
            .push(record.id.clone());
        Ok(())
    }

    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        self.deleted
            .lock()
            .expect("not poisoned")
            .push(id.to_owned());
        Ok(())
    }
}

fn record(id: &str, label: &str) -> AuthRecord {
    let mut record = AuthRecord::new(id, "claude", Utc::now());
    record.label = label.to_owned();
    record
}

#[tokio::test]
async fn config_credentials_survive_every_reload() {
    let persisted = MemoryStore::with(vec![record("db-1", "from database")]);
    let store = RuntimeAuthStore::new(persisted, vec![record("cpa-gateway-claude", "from config")]);

    for _ in 0..3 {
        let listed = store.list().await.expect("listing succeeds");

        assert_eq!(listed.len(), 2, "the manager rebuilds its map from list()");
        assert!(listed.iter().any(|r| r.id == "cpa-gateway-claude"));
        assert!(listed.iter().any(|r| r.id == "db-1"));
    }
}

#[tokio::test]
async fn a_persisted_row_wins_over_a_config_credential_with_the_same_id() {
    let persisted = MemoryStore::with(vec![record("cpa-gateway-claude", "from database")]);
    let store = RuntimeAuthStore::new(persisted, vec![record("cpa-gateway-claude", "from config")]);

    let listed = store.list().await.expect("listing succeeds");
    assert_eq!(listed.len(), 1, "the id must not appear twice");
    assert_eq!(listed[0].label, "from database");

    let fetched = store
        .get("cpa-gateway-claude")
        .await
        .expect("loading succeeds")
        .expect("found");
    assert_eq!(fetched.label, "from database");
}

#[tokio::test]
async fn get_falls_back_to_the_config_credentials() {
    let store = RuntimeAuthStore::new(
        MemoryStore::with(Vec::new()),
        vec![record("cpa-gateway-codex", "from config")],
    );

    assert_eq!(
        store
            .get("cpa-gateway-codex")
            .await
            .expect("loading succeeds")
            .map(|r| r.label),
        Some("from config".to_owned())
    );
    assert!(
        store
            .get("missing")
            .await
            .expect("loading succeeds")
            .is_none()
    );
}

#[tokio::test]
async fn writes_pass_straight_through_to_the_persistent_store() {
    let persisted = MemoryStore::with(Vec::new());
    let store = RuntimeAuthStore::new(
        Arc::clone(&persisted) as Arc<dyn AuthStore>,
        vec![record("cpa-gateway-claude", "")],
    );

    store
        .save(&record("db-1", "x"))
        .await
        .expect("saving succeeds");
    store.delete("db-2").await.expect("deleting succeeds");

    assert_eq!(*persisted.saved.lock().expect("not poisoned"), vec!["db-1"]);
    assert_eq!(
        *persisted.deleted.lock().expect("not poisoned"),
        vec!["db-2"]
    );
}

#[test]
fn nothing_to_inject_means_no_decorator() {
    let persisted: Arc<dyn AuthStore> = MemoryStore::with(Vec::new());
    let wrapped = RuntimeAuthStore::wrap(Arc::clone(&persisted), Vec::new());

    assert!(
        Arc::ptr_eq(&persisted, &wrapped),
        "an empty decorator is pure overhead on every list()"
    );
}

#[tokio::test]
async fn a_credential_without_an_id_is_never_injected() {
    let store = RuntimeAuthStore::new(MemoryStore::with(Vec::new()), vec![record("", "nameless")]);

    assert!(store.list().await.expect("listing succeeds").is_empty());
    assert!(store.get("").await.expect("loading succeeds").is_none());
}
