//! Bounded HTTPS reads. Release metadata never chooses a host or a command.
use crate::{Result, require};
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};
use ureq::tls::{Certificate, RootCerts, TlsConfig};
use url::Url;

pub(crate) fn allowed_url(url: &Url) -> bool {
    url.scheme() == "https"
        && matches!(
            url.host_str(),
            Some(
                "api.github.com"
                    | "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
                    | "github-releases.githubusercontent.com"
            )
        )
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
}

pub(crate) fn transfer(
    url: &str,
    limit: u64,
    size: Option<u64>,
    output: &mut dyn Write,
) -> Result<String> {
    let deadline = Instant::now() + Duration::from_secs(600);
    let certificates = rustls_native_certs::load_native_certs();
    require(
        !certificates.certs.is_empty(),
        "no system TLS certificates available",
    )?;
    let roots: Vec<_> = certificates
        .certs
        .iter()
        .map(|certificate| Certificate::from_der(certificate.as_ref()).to_owned())
        .collect();
    let tls = TlsConfig::builder()
        .root_certs(RootCerts::new_with_certs(&roots))
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    let mut url = Url::parse(url)?;
    for redirects in 0..=10 {
        require(allowed_url(&url), "unexpected download URL or redirect")?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("download exceeded its deadline")?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .tls_config(tls.clone())
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(remaining))
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_per_read(Some(Duration::from_secs(30)))
            .timeout_per_write(Some(Duration::from_secs(30)))
            .build()
            .into();
        let response = agent
            .get(url.as_str())
            .header("User-Agent", "kjk-update/1")
            .header("Accept-Encoding", "identity")
            .call()?;
        if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            require(redirects < 10, "too many download redirects")?;
            let location = response
                .headers()
                .get("location")
                .ok_or("redirect lacks a location")?
                .to_str()?;
            url = url.join(location)?;
            continue;
        }
        require(
            response.status().as_u16() == 200,
            &format!("GitHub returned HTTP {}", response.status()),
        )?;
        let lengths: Vec<_> = response
            .headers()
            .get_all("content-length")
            .iter()
            .collect();
        require(lengths.len() <= 1, "duplicate download size header")?;
        if let Some(length) = lengths.first() {
            let value = length.to_str()?;
            require(
                !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit()),
                "invalid download size",
            )?;
            let length: u64 = value.parse()?;
            require(
                length <= limit && size.is_none_or(|size| length == size),
                "unexpected download size",
            )?;
        }
        if let Some(encoding) = response.headers().get("content-encoding") {
            require(encoding == "identity", "unexpected HTTP content encoding")?;
        }
        let mut source = response.into_body().into_reader();
        let mut count = 0_u64;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 65536];
        loop {
            let capacity = buffer.len().min((limit - count + 1) as usize);
            let read = source.read(&mut buffer[..capacity])?;
            count += read as u64;
            require(
                count <= limit && Instant::now() < deadline,
                "download exceeded its limit",
            )?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read])?;
            digest.update(&buffer[..read]);
        }
        require(size.is_none_or(|size| count == size), "truncated download")?;
        return Ok(digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect());
    }
    Err("too many download redirects".into())
}

pub(crate) fn fetch(url: &str, limit: u64, size: Option<u64>) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    transfer(url, limit, size, &mut bytes)?;
    Ok(bytes)
}

pub(crate) fn api(repo: &str, path: &str) -> Result<Value> {
    json(&fetch(
        &format!("https://api.github.com/repos/{repo}/{path}"),
        4 * 1024 * 1024,
        None,
    )?)
}

pub(crate) fn asset_url(repo: &str, tag: &str, name: &str) -> String {
    // Repository and tag are validated before reaching here; escaping also
    // makes this construction unambiguous for every manifest asset name.
    format!(
        "https://github.com/{repo}/releases/download/{}/{}",
        segment(tag),
        segment(name)
    )
}

pub(crate) fn segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

// serde_json::Value normally overwrites duplicate object keys. Reject them
// recursively in API responses as well as in our typed manifest/state files.
struct Unique(Value);

impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Json;
        impl<'de> Visitor<'de> for Json {
            type Value = Unique;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("JSON without duplicate object keys")
            }
            fn visit_bool<E: serde::de::Error>(
                self,
                value: bool,
            ) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Bool(value)))
            }
            fn visit_i64<E: serde::de::Error>(self, value: i64) -> std::result::Result<Unique, E> {
                Ok(Unique(value.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, value: u64) -> std::result::Result<Unique, E> {
                Ok(Unique(value.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, value: f64) -> std::result::Result<Unique, E> {
                serde_json::Number::from_f64(value)
                    .map(|v| Unique(Value::Number(v)))
                    .ok_or_else(|| E::custom("non-finite JSON number"))
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::String(value.to_string())))
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut values = Vec::new();
                while let Some(Unique(value)) = seq.next_element()? {
                    values.push(value);
                }
                Ok(Unique(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some((key, Unique(value))) = map.next_entry::<String, Unique>()? {
                    if values.insert(key, value).is_some() {
                        return Err(serde::de::Error::custom("duplicate JSON key"));
                    }
                }
                Ok(Unique(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(Json)
    }
}

pub(crate) fn json(bytes: &[u8]) -> Result<Value> {
    Ok(serde_json::from_slice::<Unique>(bytes)?.0)
}
