/// Normalizes what a user types into a base URL, like the macOS client:
/// `http://` is added when no scheme is given, surrounding whitespace and
/// trailing slashes are trimmed, and a reverse-proxy path is kept
/// (`https://host/maine` stays as typed). Returns `None` for anything that is
/// not an `http` or `https` URL with a host. Query and fragment are dropped:
/// the protocol never uses them.
pub fn normalize_server_url(raw: &str) -> Option<String> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    let with_scheme = if text.contains("://") {
        text.to_string()
    } else {
        format!("http://{text}")
    };
    let (scheme, rest) = with_scheme.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if !valid_authority(authority) {
        return None;
    }
    let path_end = tail.find(['?', '#']).unwrap_or(tail.len());
    let path = tail[..path_end].trim_matches('/');
    Some(if path.is_empty() {
        format!("{scheme}://{authority}")
    } else {
        format!("{scheme}://{authority}/{path}")
    })
}

fn valid_authority(authority: &str) -> bool {
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, rest)| rest);
    if host_port.chars().any(|c| c.is_whitespace()) {
        return false;
    }
    let (host, port) = if let Some(inner) = host_port.strip_prefix('[') {
        let Some((host, after)) = inner.split_once(']') else {
            return false;
        };
        (host, after.strip_prefix(':'))
    } else {
        match host_port.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (host_port, None),
        }
    };
    if host.is_empty() {
        return false;
    }
    match port {
        None => true,
        Some(port) => !port.is_empty() && port.parse::<u16>().is_ok(),
    }
}

/// The host of a normalized URL (no scheme, userinfo, port or IPv6 brackets),
/// used as the default profile name.
pub fn host_of(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, r)| r);
    let host = if let Some(inner) = host_port.strip_prefix('[') {
        inner.split_once(']')?.0
    } else {
        host_port.rsplit_once(':').map_or(host_port, |(h, _)| h)
    };
    (!host.is_empty()).then(|| host.to_string())
}

/// `base` plus an endpoint path such as `v1/vault`.
pub(crate) fn endpoint(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_a_scheme_and_trims_slashes() {
        assert_eq!(
            normalize_server_url("127.0.0.1:8484").as_deref(),
            Some("http://127.0.0.1:8484")
        );
        assert_eq!(
            normalize_server_url("  https://sync.example.com///  ").as_deref(),
            Some("https://sync.example.com")
        );
    }

    #[test]
    fn keeps_a_reverse_proxy_path() {
        assert_eq!(
            normalize_server_url("https://host/maine/").as_deref(),
            Some("https://host/maine")
        );
        assert_eq!(
            normalize_server_url("HTTPS://host//a/b//").as_deref(),
            Some("https://host/a/b")
        );
    }

    #[test]
    fn handles_ipv6_and_rejects_bad_input() {
        assert_eq!(
            normalize_server_url("[::1]:8484").as_deref(),
            Some("http://[::1]:8484")
        );
        assert_eq!(normalize_server_url(""), None);
        assert_eq!(normalize_server_url("   "), None);
        assert_eq!(normalize_server_url("http://"), None);
        assert_eq!(normalize_server_url("ftp://host"), None);
        assert_eq!(normalize_server_url("host:notaport"), None);
        assert_eq!(normalize_server_url("host:99999"), None);
        assert_eq!(normalize_server_url("http:///path"), None);
        assert_eq!(normalize_server_url("two words"), None);
    }

    #[test]
    fn drops_query_and_fragment() {
        assert_eq!(
            normalize_server_url("http://host:1/p?x=1#f").as_deref(),
            Some("http://host:1/p")
        );
    }

    #[test]
    fn host_of_strips_scheme_port_and_brackets() {
        assert_eq!(
            host_of("http://127.0.0.1:8484").as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(
            host_of("https://sync.example.com/maine").as_deref(),
            Some("sync.example.com")
        );
        assert_eq!(host_of("http://[::1]:8484").as_deref(), Some("::1"));
        assert_eq!(host_of("http://user@h:1").as_deref(), Some("h"));
        assert_eq!(host_of("nonsense"), None);
    }

    #[test]
    fn endpoint_joins_cleanly() {
        assert_eq!(endpoint("http://h:1", "v1/vault"), "http://h:1/v1/vault");
        assert_eq!(
            endpoint("http://h:1/maine/", "/v1/vault"),
            "http://h:1/maine/v1/vault"
        );
    }
}
