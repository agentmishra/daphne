// Copyright (c) 2024 Cloudflare, Inc. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use daphne::{auth::BearerToken, testing::MockLeaderMemory, DapError};
use daphne_service_utils::{config::DaphneServiceConfig, metrics::DaphneServiceMetrics};
use futures::lock::Mutex;
use serde::{Deserialize, Serialize};
use storage_proxy_connection::{kv, Do, Kv};
use tokio::sync::RwLock;
use url::Url;

mod roles;
pub mod router;
mod storage_proxy_connection;

/// Entrypoint to the server implementation. This struct implements
/// [`DapLeader`](daphne::roles::DapLeader) and [`DapHelper`](daphne::roles::DapHelper) and can be
/// passed to the router.
///
/// It depends on a cloudflare worker to do it's storage using durable objects.
///
/// It can be constructed from:
/// - a `url` that points to a cloudflare worker which serves as proxy for the storage
/// implementation.
/// - an implementation of [`DaphneServiceMetrics`].
/// - a [`DaphneServiceConfig`].
///
/// # Examples
/// ```
/// use url::Url;
/// use daphne::{DapGlobalConfig, hpke::HpkeKemId, DapVersion};
/// use daphne_server::{App, router, StorageProxyConfig};
/// use daphne_service_utils::{config::DaphneServiceConfig, DapRole, metrics::DaphnePromServiceMetrics};
///
/// let storage_proxy_settings = StorageProxyConfig {
///     url: Url::parse("http://example.com").unwrap(),
///     auth_token: "some-token".into(),
/// };
/// let registry = prometheus::Registry::new();
/// let daphne_service_metrics = DaphnePromServiceMetrics::register(&registry).unwrap();
/// let global = DapGlobalConfig {
///     max_batch_duration: 360_00,
///     min_batch_interval_start: 259_200,
///     max_batch_interval_end: 259_200,
///     supported_hpke_kems: vec![HpkeKemId::X25519HkdfSha256],
///     allow_taskprov: true,
/// };
/// let service_config = DaphneServiceConfig {
///     env: "some-machine-identifier".into(),
///     role: DapRole::Helper,
///     global,
///     report_shard_key: [1; 32],
///     report_shard_count: 4,
///     base_url: None,
///     taskprov: None,
///     default_version: DapVersion::DraftLatest,
///     report_storage_epoch_duration: 300,
///     report_storage_max_future_time_skew: 300,
/// };
/// let app = App::new(storage_proxy_settings, daphne_service_metrics, service_config)?;
///
/// let router = router::new(DapRole::Helper, app);
///
/// # // this is so I don't have to annotate the types of `router::new`
/// # let router: axum::Router<(), axum::body::Body> = router;
/// # Ok::<(), daphne::DapError>(())
/// ```
pub struct App {
    storage_proxy_config: StorageProxyConfig,
    http: reqwest::Client,
    cache: RwLock<kv::Cache>,
    metrics: Box<dyn DaphneServiceMetrics>,
    service_config: DaphneServiceConfig,

    /// Volatile memory for the Leader, including the work queue, pending reports, and pending
    /// colleciton requests. Note that in a production Leader, it is necessary to store this state
    /// across requsets.
    test_leader_state: Arc<Mutex<MockLeaderMemory>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StorageProxyConfig {
    pub url: Url,
    #[serde(with = "transparent_auth_token")]
    pub auth_token: BearerToken,
}

impl router::DaphneService for App {
    fn server_metrics(&self) -> &dyn DaphneServiceMetrics {
        &*self.metrics
    }
}

impl App {
    /// Create a new configured app. See [`App`] for details.
    pub fn new<M>(
        storage_proxy_config: StorageProxyConfig,
        daphne_service_metrics: M,
        service_config: DaphneServiceConfig,
    ) -> Result<Self, DapError>
    where
        M: DaphneServiceMetrics + 'static,
    {
        Ok(Self {
            storage_proxy_config,
            http: reqwest::Client::new(),
            cache: Default::default(),
            metrics: Box::new(daphne_service_metrics),
            service_config,
            test_leader_state: Default::default(),
        })
    }

    pub(crate) fn durable(&self) -> Do<'_> {
        Do::new(&self.storage_proxy_config, &self.http)
    }

    pub(crate) fn kv(&self) -> Kv<'_> {
        Kv::new(&self.storage_proxy_config, &self.http, &self.cache)
    }
}

mod transparent_auth_token {
    //! For backwards compatibility reasons we can't add `#[serde(transparent)]` to
    //! [`BearerToken`], as such we have to have a custom serializer for this field in order to
    //! make the config file less verbose.
    //!
    //! # Example
    //! Without the serializer
    //! ```yaml
    //! auth_token:
    //!     raw: 'the-token'
    //! ```
    //!
    //! With the serializer
    //! ```yaml
    //! auth_token: 'the-token'
    //! ```
    //!
    //! TODO(mendes): Once the `dap_prototype` is removed we can make the change to remove this.

    use daphne::auth::BearerToken;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &BearerToken, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(value.as_ref())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<BearerToken, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(BearerToken::from(s))
    }
}
