//! The application's HTTP client.
//!
//! `gpui` defaults to [`gpui::http_client::NullHttpClient`], which fails every
//! request, so anything that reaches the network — the startup update check,
//! and images a markdown preview points at — silently does nothing until a
//! real client is installed. This is that client.
//!
//! Requests are issued with a blocking client on the background thread pool
//! rather than an async HTTP stack, because a Git GUI makes very few of them
//! and `smol::unblock` already integrates with the executor the rest of the
//! app runs on.

use futures::future::BoxFuture;
use gpui::http_client::{HttpClient, HttpResponse};
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

/// How long a single request may take before it is abandoned.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Ceiling on a response body.
///
/// Requests are driven by repository content — a markdown file names the URLs —
/// so a hostile or broken server must not be able to stream unbounded data into
/// memory.
const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;

/// Redirects to follow before giving up.
const MAX_REDIRECTS: u32 = 5;

pub(crate) struct GitCometHttpClient {
    agent: ureq::Agent,
    user_agent: String,
}

impl GitCometHttpClient {
    pub(crate) fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            .max_redirects(MAX_REDIRECTS)
            .build();

        Self {
            agent: config.into(),
            user_agent: format!(
                "GitComet/{} ({}; {})",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
        }
    }
}

impl HttpClient for GitCometHttpClient {
    fn get(
        &self,
        url: &str,
        follow_redirects: bool,
    ) -> BoxFuture<'static, anyhow::Result<HttpResponse>> {
        let agent = self.agent.clone();
        let user_agent = self.user_agent.clone();
        let url = url.to_owned();

        Box::pin(async move {
            smol::unblock(move || fetch(&agent, &user_agent, &url, follow_redirects)).await
        })
    }
}

fn fetch(
    agent: &ureq::Agent,
    user_agent: &str,
    url: &str,
    follow_redirects: bool,
) -> anyhow::Result<HttpResponse> {
    // A redirect chain is only followed when the caller asked for it; the agent
    // is shared, so the limit is applied per request rather than on the agent.
    let mut request = agent.get(url).header("User-Agent", user_agent);
    if !follow_redirects {
        request = request.config().max_redirects(0).build();
    }

    let response = request.call()?;
    let status = response.status();
    let mut body = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(MAX_RESPONSE_BYTES)
        .read_to_end(&mut body)?;

    Ok(HttpResponse { status, body })
}

/// The client to install on the application.
pub(crate) fn client() -> Arc<dyn HttpClient> {
    Arc::new(GitCometHttpClient::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agent_identifies_the_app_and_platform() {
        let client = GitCometHttpClient::new();

        assert!(
            client.user_agent.starts_with("GitComet/"),
            "servers should be able to attribute the request: {}",
            client.user_agent
        );
        assert!(client.user_agent.contains(std::env::consts::OS));
        assert!(client.user_agent.contains(std::env::consts::ARCH));
    }

    #[test]
    fn a_bad_url_fails_instead_of_panicking() {
        // Image sources come from repository content, so the client is handed
        // whatever a document happens to contain.
        let client = GitCometHttpClient::new();
        let result = smol::block_on(client.get("not a url", true));

        assert!(result.is_err(), "a malformed URL must surface as an error");
    }
}
