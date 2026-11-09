//! Module manifest download: the hostname is resolved up front and every address is checked against internal ranges
//! (SSRF); the request has a timeout, the response is at most `MAX_DOWNLOAD_BYTES`, redirects are not followed.

use std::net::IpAddr;
use std::time::Duration;

use agentd_core::{FetchError, MAX_DOWNLOAD_BYTES, ModuleFetcher, PortFuture, is_internal_ip};

use crate::HttpClient;

pub struct HttpModuleFetcher {
    http: HttpClient,
}

impl HttpModuleFetcher {
    #[must_use]
    pub fn new(http: HttpClient) -> HttpModuleFetcher {
        HttpModuleFetcher { http }
    }
}

/// `host` and `port` from the address `http(s)://host[:port]/…`.
fn host_port(url: &str) -> Option<(String, u16)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let default = if scheme == "https" { 443 } else { 80 };
    if let Some(v6) = authority.strip_prefix('[') {
        let (h, tail) = v6.split_once(']')?;
        let port = tail
            .strip_prefix(':')
            .map_or(Some(default), |p| p.parse().ok())?;
        return Some((h.to_string(), port));
    }
    match authority.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => {
            Some((h.to_string(), p.parse().ok()?))
        }
        _ => Some((authority.to_string(), default)),
    }
}

impl ModuleFetcher for HttpModuleFetcher {
    fn fetch<'a>(
        &'a self,
        url: &'a str,
        allowed: Option<&'a str>,
    ) -> PortFuture<'a, Result<Vec<u8>, FetchError>> {
        Box::pin(async move {
            let (host, port) = host_port(url).ok_or(FetchError::Failed)?;
            let authority = url
                .split_once("://")
                .and_then(|(_, r)| r.split(['/', '?', '#']).next())
                .unwrap_or_default();
            if allowed != Some(authority) {
                let addrs: Vec<IpAddr> = tokio::time::timeout(
                    Duration::from_secs(5),
                    tokio::net::lookup_host((host.as_str(), port)),
                )
                .await
                .map_err(|_| FetchError::Failed)?
                .map_err(|_| FetchError::Failed)?
                .map(|a| a.ip())
                .collect();
                if addrs.is_empty() || addrs.iter().any(|ip| is_internal_ip(*ip)) {
                    return Err(FetchError::Failed);
                }
            }
            let (status, body) = tokio::time::timeout(
                Duration::from_secs(10),
                self.http.call_json("GET", url, &[], Vec::new()),
            )
            .await
            .map_err(|_| FetchError::Failed)?
            .map_err(|_| FetchError::Failed)?;
            if status != 200 {
                return Err(FetchError::Status(status));
            }
            if body.len() > MAX_DOWNLOAD_BYTES {
                return Err(FetchError::TooLarge);
            }
            Ok(body)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_and_port() {
        assert_eq!(host_port("https://a.test/x"), Some(("a.test".into(), 443)));
        assert_eq!(
            host_port("http://a.test:8080/x"),
            Some(("a.test".into(), 8080))
        );
        assert_eq!(host_port("http://[::1]:9/x"), Some(("::1".into(), 9)));
    }
}
