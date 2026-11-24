//! Immutable packages, atomic selection and isolated per-source game data.
use crate::release::{self, Manifest};
use crate::{Args, Command, Result, Scope, Selection, archive, github, require};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct State {
    #[serde(deserialize_with = "Option::deserialize")]
    pub current: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub previous: Option<String>,
}

fn no_link(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            require(
                !metadata.file_type().is_symlink(),
                &format!("managed path is a symlink: {}", path.display()),
            )?;
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                require(
                    metadata.file_attributes() & 0x400 == 0,
                    "managed path is a reparse point",
                )?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn read_bytes(path: &Path) -> Result<Vec<u8>> {
    no_link(path)?;
    let mut file = File::open(path)?.take(65537);
    let metadata = file.get_ref().metadata()?;
    require(
        metadata.is_file() && metadata.len() <= 65536,
        "invalid managed JSON file",
    )?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    require(bytes.len() <= 65536, "managed JSON file is too large")?;
    Ok(bytes)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_value(github::json(&read_bytes(path)?)?)?)
}

pub(crate) fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    no_link(path)?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().ok_or("invalid managed path")?)?;
    serde_json::to_writer(&mut temporary, value)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    // tempfile replaces the destination atomically on both Windows and Unix.
    // A failed persist keeps the old selection and removes the temporary file.
    temporary.persist(path)?;
    Ok(())
}

// Preserve json.dumps(scope, sort_keys=True)'s spaces and ASCII escapes so an
// existing Python-updater profile is selected by the native executable too.
fn ascii_json(value: &str) -> Result<String> {
    let mut result = String::new();
    for c in serde_json::to_string(value)?.chars() {
        if c >= '\u{7f}' {
            for word in c.encode_utf16(&mut [0; 2]) {
                use std::fmt::Write;
                write!(result, "\\u{word:04x}")?;
            }
        } else {
            result.push(c);
        }
    }
    Ok(result)
}

pub(crate) fn scope_id(scope: &Scope) -> Result<String> {
    let json = format!(
        "{{\"branch\": {}, \"channel\": \"{}\", \"repository\": {}}}",
        ascii_json(&scope.branch)?,
        scope.channel.name(),
        ascii_json(&scope.repository)?
    );
    Ok(Sha256::digest(json.as_bytes())
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn home() -> Result<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "home directory is unavailable; supply --root".into())
}

fn data_parent(root: Option<&Path>) -> Result<PathBuf> {
    if let Some(root) = root {
        if let Ok(relative) = root.strip_prefix("~") {
            return Ok(home()?.join(relative));
        }
        return Ok(root.to_path_buf());
    }
    let variable = if cfg!(windows) {
        "LOCALAPPDATA"
    } else {
        "XDG_DATA_HOME"
    };
    let parent = match std::env::var_os(variable).filter(|value| !value.is_empty()) {
        Some(parent) => PathBuf::from(parent),
        None => home()?.join(if cfg!(windows) {
            "AppData/Local"
        } else {
            ".local/share"
        }),
    };
    Ok(parent.join("kjk-updates"))
}

fn locked(root: &Path) -> Result<File> {
    no_link(root)?;
    fs::create_dir_all(root)?;
    let path = root.join(".lock");
    no_link(&path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.try_lock()
        .map_err(|error| format!("this profile is busy or cannot be locked: {error}"))?;
    Ok(file)
}

fn installed(
    root: &Path,
    version: &str,
    selection: &Selection,
    system: &str,
) -> Result<(Manifest, PathBuf)> {
    let pieces: Vec<_> = version.split('-').collect();
    require(
        pieces.len() == 2 && release::hex(pieces[0], 12) && release::hex(pieces[1], 16),
        "invalid installed version pointer",
    )?;
    let folder = root.join("versions").join(version);
    no_link(&folder)?;
    let manifest = Manifest::parse(
        &read_bytes(&folder.join(".kjk-release.json"))?,
        &selection.scope.repository,
    )?;
    require(
        manifest.channel == selection.scope.channel
            && manifest.branch == selection.scope.branch
            && manifest.identifier(system)? == version,
        "installed version identity mismatch",
    )?;
    let executable = folder.join(if system == "windows-x86_64" {
        "kjk.exe"
    } else {
        "kjk"
    });
    no_link(&executable)?;
    require(executable.is_file(), "installed executable is missing")?;
    Ok((manifest, executable))
}

fn complete_package(folder: &Path, system: &str) -> Result<()> {
    let extension = if system == "windows-x86_64" {
        ".exe"
    } else {
        ""
    };
    let mut required = vec![
        format!("kjk{extension}"),
        format!("kjk-save-editor{extension}"),
        format!("kjk-update{extension}"),
    ];
    required.extend(
        [
            "README.txt",
            "LICENSE.txt",
            "NOTICE.txt",
            "THIRD-PARTY-LICENSES.html",
            "THIRD-PARTY-LICENSES-save-editor.html",
            "THIRD-PARTY-LICENSES-updater.html",
        ]
        .map(str::to_string),
    );
    for (index, name) in required.iter().enumerate() {
        let path = folder.join(name);
        no_link(&path)?;
        let metadata = fs::metadata(path)?;
        require(
            metadata.is_file() && metadata.len() > 0,
            "incomplete package",
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            require(
                !extension.is_empty() || index >= 3 || metadata.permissions().mode() & 0o111 != 0,
                "programs are not executable",
            )?;
        }
        #[cfg(not(unix))]
        let _ = index;
    }
    Ok(())
}

pub(crate) fn update(
    root: &Path,
    state: &State,
    selection: &Selection,
    system: &str,
) -> Result<()> {
    let manifest = release::select(selection)?;
    if let Some(current) = state.current.as_ref().filter(|_| selection.tag.is_none()) {
        let previous = installed(root, current, selection, system)?.0.commit;
        if previous != manifest.commit {
            let comparison = github::api(
                &selection.scope.repository,
                &format!("compare/{previous}...{}?per_page=1", manifest.commit),
            )?;
            require(
                comparison.get("status").and_then(serde_json::Value::as_str) == Some("ahead")
                    && comparison
                        .get("merge_base_commit")
                        .and_then(|value| value.get("sha"))
                        .and_then(serde_json::Value::as_str)
                        == Some(&previous),
                "automatic update is older or divergent; use --tag only for an intentional change",
            )?;
        }
    }
    let version = manifest.identifier(system)?;
    let asset = manifest.asset(system)?;
    let destination = root.join("versions").join(&version);
    no_link(&destination)?;
    if destination.try_exists()? {
        require(
            installed(root, &version, selection, system)?.0 == manifest,
            "immutable version metadata changed",
        )?;
    } else {
        let staging = tempfile::Builder::new()
            .prefix(".install-")
            .tempdir_in(root)?;
        let download = staging.path().join("download");
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&download)?;
        let digest = github::transfer(
            &github::asset_url(&selection.scope.repository, &manifest.tag, &asset.name),
            asset.size,
            Some(asset.size),
            &mut output,
        )?;
        output.sync_all()?;
        drop(output);
        require(digest == asset.sha256, "archive SHA-256 mismatch")?;
        let folder = archive::unpack(&download, &staging.path().join("unpacked"), asset)?;
        complete_package(&folder, system)?;
        let mut marker = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(folder.join(".kjk-release.json"))?;
        serde_json::to_writer(&mut marker, &manifest)?;
        marker.sync_all()?;
        drop(marker);
        fs::rename(&folder, &destination)?;
    }
    if state.current.as_deref() != Some(&version) {
        atomic_json(
            &root.join("current.json"),
            &State {
                current: Some(version),
                previous: state.current.clone(),
            },
        )?;
    }
    println!("Installed {} ({})", manifest.tag, &manifest.commit[..12]);
    Ok(())
}

pub(crate) fn run(args: &Args, selection: &Selection, system: &str) -> Result<u8> {
    let parent = data_parent(args.root.as_deref())?;
    fs::create_dir_all(&parent)?;
    let root = parent.canonicalize()?.join(scope_id(&selection.scope)?);
    let _lock = locked(&root)?;
    let scope_path = root.join("scope.json");
    no_link(&scope_path)?;
    if scope_path.try_exists()? {
        require(
            read_json::<Scope>(&scope_path)? == selection.scope,
            "managed profile scope mismatch",
        )?;
    } else {
        let entries = fs::read_dir(&root)?.collect::<std::io::Result<Vec<_>>>()?;
        require(
            entries.len() == 1 && entries[0].file_name() == ".lock",
            "unrecognized managed directory",
        )?;
        atomic_json(&scope_path, &selection.scope)?;
    }
    for name in ["versions", "profile"] {
        let path = root.join(name);
        no_link(&path)?;
        fs::create_dir_all(path)?;
    }
    let pointer = root.join("current.json");
    no_link(&pointer)?;
    let mut state: State = if pointer.try_exists()? {
        read_json(&pointer)?
    } else {
        State {
            current: None,
            previous: None,
        }
    };
    for version in [state.current.as_ref(), state.previous.as_ref()]
        .into_iter()
        .flatten()
    {
        installed(&root, version, selection, system)?;
    }
    match &args.command {
        Command::Update | Command::Launch { update: true, .. } => {
            update(&root, &state, selection, system)?;
            state = read_json(&pointer)?;
        }
        Command::Rollback => {
            require(
                state.previous.is_some(),
                "no previous version to roll back to",
            )?;
            std::mem::swap(&mut state.current, &mut state.previous);
            atomic_json(&pointer, &state)?;
            println!("Rolled back binaries; saves and settings are unchanged");
        }
        _ => {}
    }
    let profile = root.join("profile");
    println!("Profile: {}", profile.display());
    if let Command::Launch { game_args, .. } = &args.command {
        let current = state
            .current
            .as_deref()
            .ok_or("no installed version; run update first")?;
        let (manifest, executable) = installed(&root, current, selection, system)?;
        println!("Launching installed {}", manifest.tag);
        std::io::stdout().flush()?;
        let status = std::process::Command::new(executable)
            .args(game_args)
            .current_dir(&profile)
            .env("KJK_SETTINGS", profile.join("settings.json"))
            .env("KJK_SAVES", profile.join("saves"))
            .env("KJK_STORE", profile.join("store"))
            .status()?;
        return Ok(status.code().map_or(1, |code| code as u8));
    }
    Ok(0)
}
