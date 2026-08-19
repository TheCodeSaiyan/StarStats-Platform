//! Synchronous opt-in probes for user-entered configuration.
//!
//! `check_api_url` and `check_rsi_cookie` are *not* polled — they
//! fire on user click from the Settings pane. Each performs a single
//! HTTPS request with a tight timeout and returns a structured
//! result the UI can render inline next to the input that produced
//! it. Neither persists state; they're pure probes.

use serde::Serialize;
use std::net::IpAddr;
use std::str::FromStr;
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct ApiUrlCheck {
    pub ok: bool,
    pub status: Option<u16>,
    pub server_version: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CookieCheck {
    pub ok: bool,
    pub handle: Option<String>,
    pub error: Option<String>,
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Best-effort string-based SSRF guard. Returns `true` if `host` is a
/// literal private/loopback/link-local IP, or the literal name
/// `localhost`. This is intentionally NOT a DNS-resolution defense —
/// DNS rebinding remains possible but is well outside the threat
/// model. The case we're closing is a user pasting a hostile or typo'd
/// URL that points at an internal service.
///
/// Covered ranges:
/// - IPv4 loopback `127.0.0.0/8`
/// - IPv4 RFC1918 `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`
/// - IPv4 link-local `169.254.0.0/16`
/// - IPv6 loopback `::1`
/// - IPv6 link-local `fe80::/10` and ULA `fc00::/7`
fn host_is_private_or_loopback(host: &str) -> bool {
    // Strip an optional bracket pair from IPv6 literals so callers can
    // pass either the raw URL host or a bare address.
    let trimmed = host.trim_start_matches('[').trim_end_matches(']');
    if trimmed.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let Ok(ip) = IpAddr::from_str(trimmed) else {
        // Not an IP literal — caller has already filtered the
        // `localhost` name above; let DNS-based hosts through.
        return false;
    };
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, _, _] = v4.octets();
            // Loopback, RFC1918, link-local.
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                // Belt-and-braces: is_private covers 10/8, 172.16/12,
                // and 192.168/16, but `is_link_local` covers 169.254/16
                // separately. Spell out 169.254 anyway for clarity.
                || (a == 169 && b == 254)
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return true;
            }
            let seg = v6.segments()[0];
            // fe80::/10 → top 10 bits 1111 1110 10
            let is_link_local = (seg & 0xffc0) == 0xfe80;
            // fc00::/7 → top 7 bits 1111 110
            let is_ula = (seg & 0xfe00) == 0xfc00;
            is_link_local || is_ula
        }
    }
}

/// Probe the configured StarStats server. `/healthz` is exposed by
/// `crates/starstats-server/src/main.rs:371`; we GET it with a 5s
/// timeout. Success means the URL resolves AND a StarStats server is
/// listening (HTTP 200). Server version is read from the optional
/// `X-Server-Version` header if present.
pub async fn check_api_url(url: String) -> ApiUrlCheck {
    let url = url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return ApiUrlCheck {
            ok: false,
            status: None,
            server_version: None,
            error: Some("Invalid URL — must start with http:// or https://".into()),
        };
    }
    // SSRF guard: refuse to probe URLs that name a literal private,
    // loopback, or link-local host. A failed `Url::parse` is also a
    // hard reject — we cannot extract a host to vet.
    match reqwest::Url::parse(&url) {
        Ok(parsed) => match parsed.host_str() {
            None => {
                return ApiUrlCheck {
                    ok: false,
                    status: None,
                    server_version: None,
                    error: Some(
                        "URL targets a private/loopback host — not allowed for the public API probe"
                            .into(),
                    ),
                };
            }
            Some(host) => {
                if host_is_private_or_loopback(host) {
                    return ApiUrlCheck {
                        ok: false,
                        status: None,
                        server_version: None,
                        error: Some(
                            "URL targets a private/loopback host — not allowed for the public API probe"
                                .into(),
                        ),
                    };
                }
            }
        },
        Err(e) => {
            return ApiUrlCheck {
                ok: false,
                status: None,
                server_version: None,
                error: Some(format!("Couldn't parse URL: {e}")),
            };
        }
    }
    let probe_url = format!("{}/healthz", url.trim_end_matches('/'));
    let client = match reqwest::Client::builder().timeout(PROBE_TIMEOUT).build() {
        Ok(c) => c,
        Err(e) => {
            return ApiUrlCheck {
                ok: false,
                status: None,
                server_version: None,
                error: Some(format!("Couldn't build HTTP client: {e}")),
            };
        }
    };
    match client.get(&probe_url).send().await {
        Ok(r) => {
            let status = r.status().as_u16();
            let server_version = r
                .headers()
                .get("X-Server-Version")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            if r.status().is_success() {
                ApiUrlCheck {
                    ok: true,
                    status: Some(status),
                    server_version,
                    error: None,
                }
            } else {
                ApiUrlCheck {
                    ok: false,
                    status: Some(status),
                    server_version,
                    error: Some(format!("Server returned HTTP {status}")),
                }
            }
        }
        Err(e) => {
            let kind = if e.is_timeout() {
                "Timeout — server didn't respond in 5s"
            } else if e.is_connect() {
                "Couldn't connect — check the URL and your network"
            } else {
                "Network error"
            };
            ApiUrlCheck {
                ok: false,
                status: None,
                server_version: None,
                error: Some(format!("{kind}: {e}")),
            }
        }
    }
}

/// Probe an RSI session cookie by issuing one authenticated request
/// against `robertsspaceindustries.com`. Returns `ok: true` if RSI
/// accepts the cookie (HTTP 200 on the pledges page). Does NOT
/// persist the cookie — the user must explicitly hit Save.
///
/// The `handle` field is left `None` in this first pass — extracting
/// it reliably from the pledges page would require additional
/// scraping work; the validation signal alone ("the cookie is
/// accepted") is the load-bearing UX win.
pub async fn check_rsi_cookie(cookie: String) -> CookieCheck {
    let cookie = cookie.trim().to_string();
    if cookie.is_empty() {
        return CookieCheck {
            ok: false,
            handle: None,
            error: Some("Paste your Rsi-Token cookie first".into()),
        };
    }
    match crate::hangar::probe_with_cookie(&cookie).await {
        Ok(()) => CookieCheck {
            ok: true,
            handle: None,
            error: None,
        },
        Err(e) => CookieCheck {
            ok: false,
            handle: None,
            error: Some(format!("{e}")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn check_api_url_rejects_non_http() {
        let r = check_api_url("ftp://example.com".into()).await;
        assert!(!r.ok);
        assert!(r.error.unwrap().contains("Invalid URL"));
    }

    #[tokio::test]
    async fn check_api_url_rejects_garbage() {
        let r = check_api_url("not even a url".into()).await;
        assert!(!r.ok);
    }

    #[tokio::test]
    async fn check_rsi_cookie_rejects_empty() {
        let r = check_rsi_cookie("".into()).await;
        assert!(!r.ok);
        assert!(r.error.unwrap().contains("Paste"));
    }

    #[tokio::test]
    async fn check_api_url_rejects_loopback_v4() {
        let r = check_api_url("http://127.0.0.1:8080".into()).await;
        assert!(!r.ok);
        assert!(r.error.unwrap().contains("private/loopback"));
    }

    #[tokio::test]
    async fn check_api_url_rejects_localhost_name() {
        let r = check_api_url("http://localhost:8080".into()).await;
        assert!(!r.ok);
    }

    #[tokio::test]
    async fn check_api_url_rejects_rfc1918_v4() {
        for host in ["http://10.0.0.1", "http://192.168.1.1", "http://172.16.0.5"] {
            let r = check_api_url(host.into()).await;
            assert!(!r.ok, "expected reject for {host}");
        }
    }

    #[tokio::test]
    async fn check_api_url_rejects_link_local_v4() {
        let r = check_api_url("http://169.254.169.254".into()).await;
        assert!(!r.ok);
    }

    #[tokio::test]
    async fn check_api_url_rejects_loopback_v6() {
        let r = check_api_url("http://[::1]/".into()).await;
        assert!(!r.ok);
    }
}
