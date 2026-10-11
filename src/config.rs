//! Configuration is read from a TOML file (default `config.toml`, override with `--config <path>`).
//! Unknown keys are rejected so typos fail loudly, and anything security-relevant fails closed.

use serde::Deserialize;
use std::net::SocketAddr;
use std::path::Path;
use url::Url;

#[derive(Debug, thiserror::Error)]
#[error("configuration error: {0}")]
pub struct ConfigError(String);

fn err<T>(msg: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError(msg.into()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Stdio,
    Http,
}

/// Core (Node) REST API, called in-cluster with the internal scopes header (no credentials).
#[derive(Debug)]
pub struct CoreConfig {
    pub api_base: Url,
}

/// OAuth resource-server settings (core is the authorization server).
#[derive(Debug)]
pub struct OAuthConfig {
    pub issuer: String,
    pub jwks_url: Url,
    pub resource_uri: Url,
}

#[derive(Debug)]
pub struct Config {
    pub transport: Transport,
    pub bind: SocketAddr,
    pub log_level: String,
    pub core: CoreConfig,
    pub oauth: Option<OAuthConfig>,
}

pub const REQUIRED_SCOPE: &str = "portfolio:admin";
pub const REQUIRED_ROLE: &str = "admin";
pub const DEFAULT_CONFIG_PATH: &str = "config.toml";

// ---- file layout ------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    server: RawServer,
    oauth: Option<RawOAuth>,
    core: RawCore,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, default)]
struct RawServer {
    transport: Transport,
    bind: String,
    allow_remote: bool,
    behind_tls_proxy: bool,
    allow_internal_http: bool,
    resource_uri: Option<String>,
    log_level: String,
}

impl Default for RawServer {
    fn default() -> Self {
        Self {
            transport: Transport::Http,
            bind: "127.0.0.1:8765".into(),
            allow_remote: false,
            behind_tls_proxy: false,
            allow_internal_http: false,
            resource_uri: None,
            log_level: "info".into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOAuth {
    issuer: String,
    jwks_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCore {
    base_url: String,
    #[serde(default = "default_service_name")]
    service_name: String,
}

fn default_service_name() -> String {
    "core".into()
}

// ---- validation -------------------------------------------------------------------

fn is_loopback_url(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(d)) => d == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// `allow_http` permits plain http for in-cluster service URLs (explicit opt-in only).
fn parse_url(name: &str, raw: &str, allow_http: bool) -> Result<Url, ConfigError> {
    let url = Url::parse(raw).or_else(|_| err(format!("{name} is not a valid URL")))?;
    let http_ok = url.scheme() == "http" && (allow_http || is_loopback_url(&url));
    if url.scheme() != "https" && !http_ok {
        return err(format!("{name} must be https (http only for loopback)"));
    }
    Ok(url)
}

/// serde's "invalid type/value" messages quote the offending value, which may be a secret:
/// report only the line and what was expected.
fn describe_parse_error(e: &toml::de::Error, text: &str) -> String {
    let msg = e.message();
    let msg = if ["invalid type", "invalid value", "invalid length"]
        .iter()
        .any(|p| msg.starts_with(p))
    {
        msg.split_once(", expected").map_or_else(
            || "invalid value".to_owned(),
            |(_, exp)| format!("invalid value (expected{exp})"),
        )
    } else {
        msg.to_owned()
    };
    match e.span() {
        Some(span) => {
            let line = text[..span.start.min(text.len())].matches('\n').count() + 1;
            format!("line {line}: {msg}")
        }
        None => msg,
    }
}

fn non_empty<'a>(name: &str, value: &'a str) -> Result<&'a str, ConfigError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return err(format!("{name} is required"));
    }
    Ok(trimmed)
}

impl Config {
    /// Path from `--config <path>`, else `config.toml` in the working directory.
    pub fn path_from_args() -> Result<String, ConfigError> {
        let mut args = std::env::args().skip(1);
        match (args.next().as_deref(), args.next()) {
            (None, _) => Ok(DEFAULT_CONFIG_PATH.into()),
            (Some("--config"), Some(path)) => Ok(path),
            _ => err("usage: portfolio-mcp [--config <path/to/config.toml>]"),
        }
    }

    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .or_else(|e| err(format!("cannot read {}: {e}", path.display())))?;
        Self::from_toml_str(&text)
    }

    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        let raw: RawConfig =
            toml::from_str(text).or_else(|e| err(describe_parse_error(&e, text)))?;
        Self::validate(raw)
    }

    fn validate(raw: RawConfig) -> Result<Self, ConfigError> {
        let s = raw.server;

        let bind: SocketAddr = s
            .bind
            .parse()
            .or_else(|_| err("server.bind must be host:port"))?;
        if s.transport == Transport::Http
            && !bind.ip().is_loopback()
            && !(s.allow_remote && s.behind_tls_proxy)
        {
            return err(
                "non-loopback server.bind requires server.allow_remote = true and server.behind_tls_proxy = true",
            );
        }

        // Only the server-to-server URLs (core API, JWKS) may use http on a private network.
        let internal_http = s.allow_internal_http;
        let c = raw.core;
        let base = parse_url(
            "core.base_url",
            non_empty("core.base_url", &c.base_url)?,
            internal_http,
        )?;
        if c.service_name.is_empty()
            || !c
                .service_name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        {
            return err("core.service_name contains invalid characters");
        }
        let api_base = base
            .join(&format!("apis/{}/", c.service_name))
            .or_else(|_| err("core.base_url is invalid"))?;
        let core = CoreConfig { api_base };

        // OAuth is mandatory for HTTP; stdio's trust boundary is the spawning process.
        let oauth = if s.transport == Transport::Http {
            let Some(o) = raw.oauth else {
                return err("[oauth] section is required for the http transport");
            };
            let issuer = non_empty("oauth.issuer", &o.issuer)?.to_owned();
            let issuer_url = parse_url("oauth.issuer", &issuer, false)?;
            let resource = s.resource_uri.as_deref().map_or_else(
                || err("server.resource_uri is required for the http transport"),
                Ok,
            )?;
            let resource_uri = parse_url("server.resource_uri", resource, false)?;
            let jwks_url = match o.jwks_url.as_deref() {
                Some(u) => parse_url("oauth.jwks_url", u, internal_http)?,
                None => issuer_url
                    .join("/oauth/jwks")
                    .or_else(|_| err("oauth.issuer is invalid"))?,
            };
            Some(OAuthConfig {
                issuer,
                jwks_url,
                resource_uri,
            })
        } else {
            None
        };

        Ok(Config {
            transport: s.transport,
            bind,
            log_level: s.log_level,
            core,
            oauth,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"
        [server]
        resource_uri = "http://127.0.0.1:8765/mcp"

        [oauth]
        issuer = "http://127.0.0.1:3000"

        [core]
        base_url = "http://127.0.0.1:3000"
    "#;

    /// Applies `key = value` overrides (or removes the key when value is None) to BASE.
    fn with(overrides: &[(&str, &str, Option<&str>)]) -> Result<Config, ConfigError> {
        let mut doc: toml::Table = toml::from_str(BASE).unwrap();
        for (section, key, value) in overrides {
            let table = doc
                .entry(section.to_string())
                .or_insert_with(|| toml::Value::Table(toml::Table::new()))
                .as_table_mut()
                .unwrap();
            match value {
                Some(v) => {
                    let parsed: toml::Value = toml::from_str::<toml::Table>(&format!("v = {v}"))
                        .unwrap()
                        .remove("v")
                        .unwrap();
                    table.insert(key.to_string(), parsed);
                }
                None => {
                    table.remove(*key);
                }
            }
        }
        Config::from_toml_str(&toml::to_string(&doc).unwrap())
    }

    #[test]
    fn valid_loopback_config_loads_with_defaults() {
        let c = Config::from_toml_str(BASE).unwrap();
        assert_eq!(c.transport, Transport::Http);
        assert_eq!(c.bind.to_string(), "127.0.0.1:8765");
        assert_eq!(c.core.api_base.as_str(), "http://127.0.0.1:3000/apis/core/");
        let o = c.oauth.unwrap();
        assert_eq!(o.jwks_url.as_str(), "http://127.0.0.1:3000/oauth/jwks");
    }

    #[test]
    fn missing_security_settings_fail_closed() {
        for (section, key) in [
            ("core", "base_url"),
            ("oauth", "issuer"),
            ("server", "resource_uri"),
        ] {
            assert!(
                with(&[(section, key, None)]).is_err(),
                "{section}.{key} should be required"
            );
        }
    }

    #[test]
    fn unknown_keys_and_bad_types_are_rejected() {
        assert!(
            with(&[("core", "base_urll", Some("\"x\""))]).is_err(),
            "typo"
        );
        assert!(
            with(&[("server", "allow_remote", Some("\"yes\""))]).is_err(),
            "type"
        );
        assert!(
            with(&[("server", "transport", Some("\"sse\""))]).is_err(),
            "enum"
        );
        let e = Config::from_toml_str("not toml [").unwrap_err();
        assert!(e.to_string().starts_with("configuration error"));
    }

    #[test]
    fn parse_errors_do_not_echo_values() {
        let text = BASE.replace(
            "resource_uri = \"http://127.0.0.1:8765/mcp\"",
            "resource_uri = 12345678",
        );
        let e = Config::from_toml_str(&text).unwrap_err().to_string();
        assert!(!e.contains("12345678"), "{e}");
        assert!(e.contains("expected a string"), "{e}");
        assert!(e.contains("line "), "{e}");
    }

    #[test]
    fn remote_bind_needs_explicit_opt_in() {
        let bind = ("server", "bind", Some("\"0.0.0.0:8765\""));
        assert!(with(&[bind]).is_err());
        assert!(with(&[bind, ("server", "allow_remote", Some("true"))]).is_err());
        assert!(with(&[
            bind,
            ("server", "allow_remote", Some("true")),
            ("server", "behind_tls_proxy", Some("true")),
        ])
        .is_ok());
    }

    #[test]
    fn plain_http_rejected_for_non_loopback_urls() {
        assert!(with(&[("oauth", "issuer", Some("\"http://auth.example.com\""))]).is_err());
    }

    #[test]
    fn internal_http_opt_in_never_relaxes_public_urls() {
        let internal = [
            (
                "core",
                "base_url",
                Some("\"http://core.portfolio.svc.cluster.local:8080\""),
            ),
            (
                "oauth",
                "jwks_url",
                Some("\"http://core.portfolio.svc.cluster.local:8080/oauth/jwks\""),
            ),
        ];
        assert!(with(&internal).is_err());
        let mut allowed = internal.to_vec();
        allowed.push(("server", "allow_internal_http", Some("true")));
        assert!(with(&allowed).is_ok());

        let mut bad_issuer = allowed.clone();
        bad_issuer.push(("oauth", "issuer", Some("\"http://auth.example.com\"")));
        assert!(with(&bad_issuer).is_err(), "issuer must stay https");

        let mut bad_resource = allowed;
        bad_resource.push((
            "server",
            "resource_uri",
            Some("\"http://mcp.example.com/mcp\""),
        ));
        assert!(with(&bad_resource).is_err(), "resource must stay https");
    }

    #[test]
    fn stdio_does_not_require_oauth() {
        let c = with(&[
            ("server", "transport", Some("\"stdio\"")),
            ("oauth", "issuer", None),
            ("server", "resource_uri", None),
        ]);
        // oauth table still present but empty -> issuer missing; drop the section entirely instead
        assert!(c.is_err());
        let text = r#"
            [server]
            transport = "stdio"
            [core]
            base_url = "http://127.0.0.1:3000"
        "#;
        assert!(Config::from_toml_str(text).unwrap().oauth.is_none());
    }

    #[test]
    fn shipped_example_config_is_valid() {
        let c = Config::from_toml_str(include_str!("../config.example.toml")).unwrap();
        assert_eq!(c.transport, Transport::Http);
        assert!(c.oauth.is_some());
    }

    #[test]
    fn deployed_config_is_valid() {
        let c = Config::from_toml_str(include_str!("../config.toml")).unwrap();
        assert_eq!(c.transport, Transport::Http);
        assert_eq!(
            c.oauth.unwrap().resource_uri.as_str(),
            "https://mcp.jeetul.com/mcp"
        );
    }
}
