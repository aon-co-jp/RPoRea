//! Tomcat の RemoteIpValve / コンテキストパスに相当する層: Apache・nginx・open-web-server の背後で動かすときの
//! 「本当のクライアントIP・スキーム・ホスト・URL接頭辞」の解決。信頼したプロキシ(CIDR)からの転送ヘッダだけを信じる。
//! Equivalent of Tomcat's RemoteIpValve + context path: resolves the real client IP, scheme, host and URL prefix behind
//! Apache, nginx or open-web-server. Forwarding headers are honoured only when the direct peer is a trusted proxy.
use std::net::IpAddr;

/// A trusted proxy network, e.g. `10.0.0.0/8` or `127.0.0.1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cidr {
    net: IpAddr,
    bits: u8,
}

impl Cidr {
    pub fn parse(s: &str) -> Option<Cidr> {
        let (a, b) = match s.split_once('/') {
            Some((a, b)) => (a, Some(b)),
            None => (s, None),
        };
        let net: IpAddr = a.trim().parse().ok()?;
        let max = if net.is_ipv4() { 32 } else { 128 };
        let bits = match b {
            Some(b) => b.trim().parse::<u8>().ok().filter(|v| *v <= max)?,
            None => max,
        };
        Some(Cidr { net, bits })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        fn bits_of(ip: IpAddr) -> u128 {
            match ip {
                IpAddr::V4(v) => u32::from(v) as u128,
                IpAddr::V6(v) => u128::from(v),
            }
        }
        if self.net.is_ipv4() != ip.is_ipv4() {
            return false;
        }
        let width: u32 = if ip.is_ipv4() { 32 } else { 128 };
        let drop = width - self.bits as u32;
        if drop >= 128 {
            return true;
        }
        (bits_of(self.net) >> drop) == (bits_of(ip) >> drop)
    }
}

/// What the application should see as the request origin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    pub client_ip: IpAddr,
    pub https: bool,
    pub host: Option<String>,
    /// URL prefix the proxy mounted this app under (no trailing slash), `""` for the root.
    pub prefix: String,
}

fn clean_prefix(p: &str) -> String {
    let t = p.trim().trim_matches('/');
    if t.is_empty() || t.contains("..") || t.contains('\\') || t.chars().any(|c| c.is_control() || c.is_whitespace()) {
        String::new()
    } else {
        format!("/{t}")
    }
}

/// Resolve the origin. `get` returns a header value by lower-case name. `peer` is the directly connected address.
pub fn resolve(peer: IpAddr, get: impl Fn(&str) -> Option<String>, trusted: &[Cidr]) -> Origin {
    let is_trusted = |ip: IpAddr| trusted.iter().any(|c| c.contains(ip));
    if !is_trusted(peer) {
        return Origin { client_ip: peer, https: false, host: get("host"), prefix: String::new() };
    }
    // X-Forwarded-For: walk from the right, skipping trusted proxies; the first untrusted hop is the client.
    let mut client = peer;
    if let Some(xff) = get("x-forwarded-for") {
        for part in xff.split(',').rev() {
            match part.trim().parse::<IpAddr>() {
                Ok(ip) => {
                    client = ip;
                    if !is_trusted(ip) {
                        break;
                    }
                }
                Err(_) => break, // garbage: stop, keep the last good hop
            }
        }
    }
    let https = match get("x-forwarded-proto") {
        Some(p) => p.split(',').next().map(|v| v.trim().eq_ignore_ascii_case("https")).unwrap_or(false),
        None => false,
    };
    let host = get("x-forwarded-host").and_then(|h| h.split(',').next().map(|v| v.trim().to_string())).filter(|h| !h.is_empty()).or_else(|| get("host"));
    let prefix = get("x-forwarded-prefix").map(|p| clean_prefix(&p)).unwrap_or_default();
    Origin { client_ip: client, https, host, prefix }
}

/// Remove the mount prefix from a request path (the proxy may or may not have stripped it already).
pub fn strip_prefix<'a>(path: &'a str, prefix: &str) -> &'a str {
    if prefix.is_empty() {
        return path;
    }
    match path.strip_prefix(prefix) {
        Some("") => "/",
        Some(rest) if rest.starts_with('/') => rest,
        _ => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn hdr(v: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        move |k| m.get(k).cloned()
    }
    fn t() -> Vec<Cidr> {
        vec![Cidr::parse("127.0.0.1").unwrap(), Cidr::parse("10.0.0.0/8").unwrap()]
    }

    #[test]
    fn cidr_matching() {
        let c = Cidr::parse("10.0.0.0/8").unwrap();
        assert!(c.contains("10.9.8.7".parse().unwrap()));
        assert!(!c.contains("11.0.0.1".parse().unwrap()));
        assert!(!c.contains("::1".parse().unwrap()));
        assert!(Cidr::parse("::1/128").unwrap().contains("::1".parse().unwrap()));
        assert!(Cidr::parse("0.0.0.0/0").unwrap().contains("8.8.8.8".parse().unwrap()));
        assert!(Cidr::parse("10.0.0.0/33").is_none());
    }

    #[test]
    fn untrusted_peer_cannot_spoof() {
        let o = resolve("203.0.113.5".parse().unwrap(), hdr(&[("x-forwarded-for", "1.2.3.4"), ("x-forwarded-proto", "https"), ("x-forwarded-prefix", "/evil"), ("host", "a.example")]), &t());
        assert_eq!(o.client_ip.to_string(), "203.0.113.5");
        assert!(!o.https);
        assert_eq!(o.prefix, "");
        assert_eq!(o.host.as_deref(), Some("a.example"));
    }

    #[test]
    fn trusted_proxy_chain() {
        let o = resolve("127.0.0.1".parse().unwrap(), hdr(&[("x-forwarded-for", "9.9.9.9, 198.51.100.7, 10.1.1.1"), ("x-forwarded-proto", "https"), ("x-forwarded-host", "site.example"), ("x-forwarded-prefix", "/app/")]), &t());
        assert_eq!(o.client_ip.to_string(), "198.51.100.7"); // the spoofed 9.9.9.9 is to the left of the first untrusted hop
        assert!(o.https);
        assert_eq!(o.host.as_deref(), Some("site.example"));
        assert_eq!(o.prefix, "/app");
    }

    #[test]
    fn garbage_headers_are_safe() {
        let o = resolve("127.0.0.1".parse().unwrap(), hdr(&[("x-forwarded-for", "not-an-ip"), ("x-forwarded-prefix", "/../etc")]), &t());
        assert_eq!(o.client_ip.to_string(), "127.0.0.1");
        assert_eq!(o.prefix, "");
    }

    #[test]
    fn prefix_stripping() {
        assert_eq!(strip_prefix("/app/x/y", "/app"), "/x/y");
        assert_eq!(strip_prefix("/app", "/app"), "/");
        assert_eq!(strip_prefix("/apple/x", "/app"), "/apple/x");
        assert_eq!(strip_prefix("/x", ""), "/x");
    }

    #[test]
    fn edge_cases_ipv6_all_trusted_and_proto_list() {
        // IPv6 client behind a trusted IPv4 proxy
        let o = resolve("127.0.0.1".parse().unwrap(), hdr(&[("x-forwarded-for", "2001:db8::1"), ("x-forwarded-proto", "https, http")]), &t());
        assert_eq!(o.client_ip.to_string(), "2001:db8::1");
        assert!(o.https); // the first value of the list wins
        // every hop trusted: the leftmost entry is the client
        let o = resolve("127.0.0.1".parse().unwrap(), hdr(&[("x-forwarded-for", "10.1.1.1, 10.2.2.2")]), &t());
        assert_eq!(o.client_ip.to_string(), "10.1.1.1");
        // plain http, and no forwarded host falls back to Host
        let o = resolve("127.0.0.1".parse().unwrap(), hdr(&[("x-forwarded-proto", "http"), ("host", "h.example")]), &t());
        assert!(!o.https);
        assert_eq!(o.host.as_deref(), Some("h.example"));
        // empty forwarded host is ignored
        let o = resolve("127.0.0.1".parse().unwrap(), hdr(&[("x-forwarded-host", " "), ("host", "h.example")]), &t());
        assert_eq!(o.host.as_deref(), Some("h.example"));
    }

    #[test]
    fn cidr_parse_and_prefix_edge_cases() {
        // IPv6 prefix match and mismatch
        let c = Cidr::parse("2001:db8::/32").unwrap();
        assert!(c.contains("2001:db8:1::5".parse().unwrap()));
        assert!(!c.contains("2001:db9::1".parse().unwrap()));
        // a /0 of one family does not cover the other family
        assert!(!Cidr::parse("0.0.0.0/0").unwrap().contains("::1".parse().unwrap()));
        // garbage and out-of-range lengths are rejected
        for bad in ["", "abc", "10.0.0.0/x", "::1/129", "10.0.0.0/-1"] {
            assert!(Cidr::parse(bad).is_none(), "{bad:?}");
        }
        // surrounding spaces are tolerated
        assert!(Cidr::parse(" 10.0.0.0 / 8 ").is_some());
        // strip_prefix keeps a path that only shares the prefix text, and handles a trailing slash
        assert_eq!(strip_prefix("/app/", "/app"), "/");
        assert_eq!(strip_prefix("/ap", "/app"), "/ap");
    }

    #[test]
    fn unsafe_prefixes_are_dropped() {
        for bad in ["/a b", "/a\\b", "/a/../b", "/a\tb"] {
            let o = resolve("127.0.0.1".parse().unwrap(), hdr(&[("x-forwarded-prefix", bad)]), &t());
            assert_eq!(o.prefix, "", "{bad:?}");
        }
        let o = resolve("127.0.0.1".parse().unwrap(), hdr(&[("x-forwarded-prefix", "/")]), &t());
        assert_eq!(o.prefix, "");
    }

    #[test]
    fn untrusted_peer_ignores_all_forwarded_headers_and_chain_stops_at_first_untrusted() {
        // untrusted peer: https/host/prefix headers are all ignored
        let o = resolve(
            "203.0.113.5".parse().unwrap(),
            hdr(&[("x-forwarded-proto", "https"), ("x-forwarded-host", "evil.example"), ("x-forwarded-prefix", "/x"), ("host", "real.example")]),
            &t(),
        );
        assert_eq!((o.https, o.host.as_deref(), o.prefix.as_str()), (false, Some("real.example"), ""));
        // trusted peer: XFF is walked from the right and stops at the first untrusted hop
        let o = resolve("127.0.0.1".parse().unwrap(), hdr(&[("x-forwarded-for", "9.9.9.9, 198.51.100.7, 10.1.1.1")]), &t());
        assert_eq!(o.client_ip, "198.51.100.7".parse::<IpAddr>().unwrap());
        // proto is case-insensitive, and a nested prefix is kept
        let o = resolve("127.0.0.1".parse().unwrap(), hdr(&[("x-forwarded-proto", "HTTPS"), ("x-forwarded-prefix", "/a/b/")]), &t());
        assert!(o.https);
        assert_eq!(o.prefix, "/a/b");
    }

    #[test]
    fn trusted_peer_without_headers_and_garbage_hop() {
        // no forwarded headers: the peer itself is the client, http, Host kept, no prefix
        let o = resolve("127.0.0.1".parse().unwrap(), hdr(&[("host", "h.example")]), &t());
        assert_eq!((o.client_ip, o.https, o.host.as_deref(), o.prefix.as_str()), ("127.0.0.1".parse::<IpAddr>().unwrap(), false, Some("h.example"), ""));
        // a garbage hop to the left of a good untrusted hop is never reached
        let o = resolve("127.0.0.1".parse().unwrap(), hdr(&[("x-forwarded-for", "not-an-ip, 198.51.100.7")]), &t());
        assert_eq!(o.client_ip, "198.51.100.7".parse::<IpAddr>().unwrap());
        // a garbage hop right next to the proxy: keep the peer (last good hop)
        let o = resolve("127.0.0.1".parse().unwrap(), hdr(&[("x-forwarded-for", "198.51.100.7, garbage")]), &t());
        assert_eq!(o.client_ip, "127.0.0.1".parse::<IpAddr>().unwrap());
        // strip_prefix must not cut inside a path segment
        assert_eq!(strip_prefix("/abc/x", "/a"), "/abc/x");
    }
}
