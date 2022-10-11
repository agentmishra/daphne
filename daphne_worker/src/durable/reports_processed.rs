// Copyright (c) 2022 Cloudflare, Inc. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause

use crate::{
    durable::{state_set_if_not_exists, BINDING_DAP_REPORTS_PROCESSED},
    int_err,
};
use futures::future::try_join_all;
use worker::*;

pub(crate) const DURABLE_REPORTS_PROCESSED_MARK_AGGREGATED: &str =
    "/internal/do/report_store/mark_aggregated";

/// Durable Object (DO) for tracking which reports have been processed.
///
/// This object defines a single API endpoint, `DURABLE_REPORTS_PROCESSED_MARK_AGGREGATED`, which
/// is used to mark a set of reports as aggregated. It returns the set of reports in that have
/// already been aggregated (and thus need to be rejected by the caller).
///
/// The schema for stored report IDs is as follows:
///
/// ```text
///     processed/<report_id> -> bool
/// ```
///
/// where `<report_id>` is the hex-encoded report ID.
#[durable_object]
pub struct ReportsProcessed {
    #[allow(dead_code)]
    state: State,
    env: Env,
    touched: bool,
}

impl ReportsProcessed {
    /// Check if the report has been processed. If not, return None; otherwise, return the ID.
    async fn to_checked(&self, nonce_hex: String) -> Result<Option<String>> {
        let key = format!("processed/{}", nonce_hex);
        let processed: bool = state_set_if_not_exists(&self.state, &key, &true)
            .await?
            .unwrap_or(false);
        if processed {
            Ok(Some(nonce_hex))
        } else {
            Ok(None)
        }
    }
}

#[durable_object]
impl DurableObject for ReportsProcessed {
    fn new(state: State, env: Env) -> Self {
        Self {
            state,
            env,
            touched: false,
        }
    }

    async fn fetch(&mut self, mut req: Request) -> Result<Response> {
        let id_hex = self.state.id().to_string();
        ensure_garbage_collected!(req, self, id_hex.clone(), BINDING_DAP_REPORTS_PROCESSED);

        match (req.path().as_ref(), req.method()) {
            // Mark a set of reports as aggregated. Return the set of report IDs that already
            // exist.
            //
            // Input: `nonce_hex_set: Vec<String>` (hex-encoded report IDs)
            // Output: `Vec<String>` (subset of the inputs that already exist).
            (DURABLE_REPORTS_PROCESSED_MARK_AGGREGATED, Method::Post) => {
                let nonce_hex_set: Vec<String> = req.json().await?;
                let mut requests = Vec::new();
                for nonce_hex in nonce_hex_set.into_iter() {
                    requests.push(self.to_checked(nonce_hex));
                }

                let responses: Vec<Option<String>> = try_join_all(requests).await?;
                let res: Vec<String> = responses.into_iter().flatten().collect();
                Response::from_json(&res)
            }

            _ => Err(int_err(format!(
                "ReportsProcessed: unexpected request: method={:?}; path={:?}",
                req.method(),
                req.path()
            ))),
        }
    }
}