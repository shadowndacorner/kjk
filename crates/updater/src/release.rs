//! The schema is shared with the release producer, not with the game.
use crate::{Channel, Result, Selection, github, require};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub(crate) const MAX_DOWNLOAD: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Asset {
    pub platform: String,
    pub name: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub schema_version: u32,
    pub repository: String,
    pub channel: Channel,
    pub branch: String,
    pub tag: String,
    pub commit: String,
    pub version: String,
    pub assets: Vec<Asset>,
}

pub(crate) fn valid_repo(value: &str) -> bool {
    let parts: Vec<_> = value.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
        })
}

pub(crate) fn valid_tag(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}

fn stable_tag(value: &str) -> bool {
    value.strip_prefix('v').is_some_and(|version| {
        let parts: Vec<_> = version.split('.').collect();
        parts.len() == 3
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|c| c.is_ascii_digit()))
    })
}

pub(crate) fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

pub(crate) fn suffix(platform: &str) -> Result<&'static str> {
    match platform {
        "windows-x86_64" => Ok(".zip"),
        "linux-x86_64" | "macos-aarch64" => Ok(".tar.gz"),
        _ => Err("unknown manifest platform".into()),
    }
}

impl Manifest {
    pub(crate) fn parse(bytes: &[u8], repo: &str) -> Result<Self> {
        let manifest: Self = serde_json::from_value(github::json(bytes)?)?;
        require(manifest.schema_version == 1, "unsupported manifest schema")?;
        require(
            valid_repo(&manifest.repository) && manifest.repository.to_ascii_lowercase() == repo,
            "manifest repository mismatch",
        )?;
        require(
            (1..=255).contains(&manifest.branch.chars().count()),
            "invalid manifest branch",
        )?;
        require(
            valid_tag(&manifest.tag)
                && manifest.version == manifest.tag
                && hex(&manifest.commit, 40),
            "invalid manifest version or commit",
        )?;
        require(
            manifest.channel != Channel::Stable
                || (manifest.branch == "main" && stable_tag(&manifest.tag)),
            "invalid stable release",
        )?;
        require(
            (2..=3).contains(&manifest.assets.len()),
            "missing required platform assets",
        )?;
        let mut platforms = HashSet::new();
        for asset in &manifest.assets {
            require(
                platforms.insert(asset.platform.as_str()),
                "duplicate manifest platform",
            )?;
            require(
                asset.name
                    == format!(
                        "kjk-{}-{}{}",
                        manifest.version,
                        asset.platform,
                        suffix(&asset.platform)?
                    )
                    && hex(&asset.sha256, 64)
                    && (1..=MAX_DOWNLOAD).contains(&asset.size),
                "invalid asset identity or size",
            )?;
        }
        require(
            platforms.contains("windows-x86_64") && platforms.contains("linux-x86_64"),
            "Windows and Linux assets are required",
        )?;
        Ok(manifest)
    }

    pub(crate) fn asset(&self, system: &str) -> Result<&Asset> {
        self.assets
            .iter()
            .find(|asset| asset.platform == system)
            .ok_or_else(|| "manifest lacks this platform".into())
    }

    pub(crate) fn identifier(&self, system: &str) -> Result<String> {
        Ok(format!(
            "{}-{}",
            &self.commit[..12],
            &self.asset(system)?.sha256[..16]
        ))
    }
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("invalid release {key}").into())
}

// GitHub's publication timestamp has a fixed UTC form, whose lexical order is
// chronological. Validate the calendar too so corrupt metadata fails closed.
fn valid_published(value: &str) -> bool {
    let b = value.as_bytes();
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
        || b.iter()
            .enumerate()
            .any(|(i, c)| ![4, 7, 10, 13, 16, 19].contains(&i) && !c.is_ascii_digit())
    {
        return false;
    }
    let number = |range: std::ops::Range<usize>| {
        b[range]
            .iter()
            .fold(0_u32, |value, c| value * 10 + u32::from(c - b'0'))
    };
    let year = number(0..4);
    let month = number(5..7);
    let days = match month {
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return false,
    };
    year > 0
        && (1..=days).contains(&number(8..10))
        && number(11..13) < 24
        && number(14..16) < 60
        && number(17..19) < 60
}

pub(crate) fn select(selection: &Selection) -> Result<Manifest> {
    let scope = &selection.scope;
    let mut releases = Vec::new();
    if let Some(tag) = &selection.tag {
        releases.push(github::api(
            &scope.repository,
            &format!("releases/tags/{}", github::segment(tag)),
        )?);
    } else {
        for page in 1..=10 {
            let batch = github::api(
                &scope.repository,
                &format!("releases?per_page=100&page={page}"),
            )?;
            let batch = batch.as_array().ok_or("invalid releases response")?;
            require(batch.len() <= 100, "invalid releases response")?;
            releases.extend_from_slice(batch);
            if batch.len() < 100 {
                break;
            }
            require(
                page < 10,
                "release scan exceeds 1,000 entries; select --tag",
            )?;
        }
    }
    let mut candidates = Vec::new();
    for release in &releases {
        let draft = release
            .get("draft")
            .and_then(Value::as_bool)
            .ok_or("invalid release draft flag")?;
        let prerelease = release
            .get("prerelease")
            .and_then(Value::as_bool)
            .ok_or("invalid release prerelease flag")?;
        if draft || prerelease != (scope.channel == Channel::Dev) {
            continue;
        }
        let Some(tag) = release
            .get("tag_name")
            .and_then(Value::as_str)
            .filter(|tag| valid_tag(tag))
        else {
            continue;
        };
        if !selection
            .tag
            .as_ref()
            .map_or_else(|| selection.pattern.matches(tag), |exact| tag == exact)
            || (scope.channel == Channel::Stable && !stable_tag(tag))
        {
            continue;
        }
        let published = string(release, "published_at")?;
        require(valid_published(published), "invalid publication time")?;
        candidates.push((published, tag, release));
    }
    candidates.sort_by(|a, b| b.0.cmp(a.0));
    for (_, tag, release) in candidates {
        let assets = release
            .get("assets")
            .and_then(Value::as_array)
            .ok_or("invalid release assets")?;
        require(assets.len() <= 32, "too many release assets")?;
        let mut by_name = HashMap::new();
        for asset in assets {
            let name = string(asset, "name")?;
            let size = asset
                .get("size")
                .and_then(Value::as_u64)
                .filter(|size| *size > 0)
                .ok_or("invalid release asset size")?;
            require(
                string(asset, "state")? == "uploaded" && by_name.insert(name, size).is_none(),
                "invalid or duplicate release asset",
            )?;
        }
        let size = *by_name
            .get("kjk-release.json")
            .ok_or("release lacks kjk-release.json; no build installed")?;
        require(size <= 65536, "manifest is too large")?;
        let manifest = Manifest::parse(
            &github::fetch(
                &github::asset_url(&scope.repository, tag, "kjk-release.json"),
                size,
                Some(size),
            )?,
            &scope.repository,
        )?;
        require(
            manifest.tag == tag && manifest.channel == scope.channel,
            "manifest/release mismatch",
        )?;
        for asset in &manifest.assets {
            require(
                by_name.get(asset.name.as_str()) == Some(&asset.size),
                "manifest/asset mismatch",
            )?;
        }
        if manifest.branch != scope.branch {
            continue;
        }
        let commit = github::api(
            &scope.repository,
            &format!("commits/{}", github::segment(tag)),
        )?;
        require(
            commit.get("sha").and_then(Value::as_str) == Some(&manifest.commit),
            "release tag moved or commit mismatched",
        )?;
        return Ok(manifest);
    }
    Err("no eligible published release; check repository, channel, branch and tag".into())
}
