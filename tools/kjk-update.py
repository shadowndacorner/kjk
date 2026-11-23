#!/usr/bin/env python3
"""Install published KJK test builds; Python 3.10+, standard library only."""

import argparse
import contextlib
import datetime
import fnmatch
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import re
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.parse
import urllib.request
import zipfile
import zlib

MAX_DOWNLOAD, MAX_EXPANDED, MAX_FILES = 1024**3, 4 * 1024**3, 10000
SUFFIXES = {"windows-x86_64": ".zip", "linux-x86_64": ".tar.gz", "macos-aarch64": ".tar.gz"}
TAG = r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}"
VERSION_ID = r"[0-9a-f]{12}-[0-9a-f]{16}"


class Refused(Exception):
    pass


def require(condition, message):
    if not condition:
        raise Refused(message)


def matches(pattern, value):
    return isinstance(value, str) and re.fullmatch(pattern, value) is not None


def json_bytes(data):
    def pairs(items):
        result = dict(items)
        require(len(result) == len(items), "duplicate JSON key")
        return result
    return json.loads(data.decode("utf-8"), object_pairs_hook=pairs,
                      parse_constant=lambda value: require(False, "non-finite JSON number"))


class GitHubRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, url):
        parsed = urllib.parse.urlsplit(url)
        require(parsed.scheme == "https" and parsed.hostname in {
            "api.github.com", "github.com", "release-assets.githubusercontent.com",
            "objects.githubusercontent.com", "github-releases.githubusercontent.com"
        } and not parsed.username and not parsed.password and parsed.port in (None, 443),
                "unexpected download redirect")
        return super().redirect_request(request, fp, code, message, headers, url)


def transfer(url, limit, size=None, output=None):
    target, digest, count = output or io.BytesIO(), hashlib.sha256(), 0
    deadline = time.monotonic() + 600
    request = urllib.request.Request(url, headers={"User-Agent": "kjk-update/1", "Accept-Encoding": "identity"})
    with urllib.request.build_opener(GitHubRedirect()).open(request, timeout=30) as response:
        length = response.headers.get("Content-Length")
        require(length is None or (length.isdecimal() and int(length) <= limit
                and (size is None or int(length) == size)), "unexpected download size")
        while block := response.read(min(65536, limit - count + 1)):
            count += len(block)
            require(count <= limit and time.monotonic() < deadline, "download exceeded its limit")
            target.write(block)
            digest.update(block)
    require(size is None or count == size, "truncated download")
    return target.getvalue() if output is None else digest.hexdigest()


def api(repo, path):
    return json_bytes(transfer(f"https://api.github.com/repos/{repo}/{path}", 4 * 1024**2))


def asset_url(repo, tag, name):
    return f"https://github.com/{repo}/releases/download/{urllib.parse.quote(tag, safe='')}/{urllib.parse.quote(name, safe='')}"


def manifest(value, repo):
    require(isinstance(value, dict) and set(value) == {
        "schema_version", "repository", "channel", "branch", "tag", "commit", "version", "assets"
    }, "invalid manifest fields")
    require(type(value["schema_version"]) is int and value["schema_version"] == 1, "unsupported manifest schema")
    require(matches(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", value["repository"])
            and value["repository"].lower() == repo, "manifest repository mismatch")
    require(value["channel"] in ("stable", "dev") and isinstance(value["branch"], str)
            and 1 <= len(value["branch"]) <= 255, "invalid manifest channel or branch")
    require(matches(TAG, value["tag"]) and value["version"] == value["tag"]
            and matches(r"[0-9a-f]{40}", value["commit"]), "invalid manifest version or commit")
    require(value["channel"] != "stable" or (value["branch"] == "main"
            and matches(r"v[0-9]+\.[0-9]+\.[0-9]+", value["tag"])), "invalid stable release")
    assets, platforms = value["assets"], set()
    require(isinstance(assets, list) and 2 <= len(assets) <= 3, "missing required platform assets")
    for entry in assets:
        require(isinstance(entry, dict) and set(entry) == {"platform", "name", "sha256", "size"}, "invalid asset fields")
        name = entry["platform"]
        require(isinstance(name, str) and name in SUFFIXES and name not in platforms, "duplicate or unknown platform")
        platforms.add(name)
        require(entry["name"] == f"kjk-{value['version']}-{name}{SUFFIXES[name]}"
                and matches(r"[0-9a-f]{64}", entry["sha256"])
                and type(entry["size"]) is int and 0 < entry["size"] <= MAX_DOWNLOAD, "invalid asset identity or size")
    require({"windows-x86_64", "linux-x86_64"} <= platforms, "Windows and Linux assets are required")
    return value


def select(args):
    if args.tag:
        releases = [api(args.repo, "releases/tags/" + urllib.parse.quote(args.tag, safe=""))]
    else:
        releases = []
        for page in range(1, 11):
            batch = api(args.repo, f"releases?per_page=100&page={page}")
            require(isinstance(batch, list) and len(batch) <= 100, "invalid releases response")
            releases.extend(batch)
            if len(batch) < 100:
                break
        else:
            raise Refused("release scan exceeds 1,000 entries; select --tag")
    candidates = []
    for release in releases:
        require(isinstance(release, dict) and type(release.get("draft")) is bool
                and type(release.get("prerelease")) is bool, "invalid release metadata")
        tag = release.get("tag_name")
        if release["draft"] or release["prerelease"] != (args.channel == "dev"):
            continue
        if not matches(TAG, tag) or not (tag == args.tag if args.tag else fnmatch.fnmatchcase(tag, args.tag_pattern)):
            continue
        if args.channel == "stable" and not matches(r"v[0-9]+\.[0-9]+\.[0-9]+", tag):
            continue
        published = release.get("published_at")
        require(matches(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ", published), "invalid publication time")
        datetime.datetime.strptime(published, "%Y-%m-%dT%H:%M:%SZ")
        candidates.append(release)
    for release in sorted(candidates, key=lambda item: item["published_at"], reverse=True):
        assets = release.get("assets")
        require(isinstance(assets, list) and len(assets) <= 32, "invalid release assets")
        by_name = {}
        for item in assets:
            require(isinstance(item, dict) and isinstance(item.get("name"), str)
                    and item["name"] not in by_name and item.get("state") == "uploaded"
                    and type(item.get("size")) is int and item["size"] > 0, "invalid or duplicate release asset")
            by_name[item["name"]] = item
        require("kjk-release.json" in by_name, "release lacks kjk-release.json; no build installed")
        size = by_name["kjk-release.json"]["size"]
        require(size <= 65536, "manifest is too large")
        value = manifest(json_bytes(transfer(asset_url(args.repo, release["tag_name"], "kjk-release.json"), size, size)), args.repo)
        require(value["tag"] == release["tag_name"] and value["channel"] == args.channel, "manifest/release mismatch")
        for item in value["assets"]:
            require(item["name"] in by_name and by_name[item["name"]]["size"] == item["size"], "manifest/asset mismatch")
        if value["branch"] != args.branch:
            continue
        commit = api(args.repo, "commits/" + urllib.parse.quote(value["tag"], safe=""))
        require(isinstance(commit, dict) and commit.get("sha") == value["commit"], "release tag moved or commit mismatched")
        return value
    raise Refused("no eligible published release; check repository, channel, branch and tag")


class TarReader:
    def __init__(self, source):
        self.source, self.position, self.last = source, 0, b""

    def tell(self):
        return self.position

    def read(self, size):
        require(0 <= size <= 1024**2, "oversized tar metadata")
        data = self.source.read(size)
        self.last = data
        self.position += len(data)
        require(self.position <= MAX_EXPANDED, "expanded archive exceeded its limit")
        return data

    def seek(self, offset):
        require(self.position <= offset <= MAX_EXPANDED, "invalid tar offset")
        while self.position < offset:
            require(self.read(min(65536, offset - self.position)), "truncated tar")
        return self.position


def unpack(archive, destination, asset):
    suffix = SUFFIXES[asset["platform"]]
    expected_root, paths, seen, total = asset["name"][:-len(suffix)], {}, set(), 0

    def extract(name, directory, size, mode, source):
        nonlocal total
        parts = (name[:-1] if directory and name.endswith("/") else name).split("/")
        require(parts[0] == expected_root and len(parts) <= 32 and len(name) <= 240, "invalid archive root or path length")
        for part in parts:
            require(part and part not in (".", "..") and part == part.strip() and part[-1] != "."
                    and all(32 <= ord(c) < 127 and c not in '\\<>:"|?*~' for c in part)
                    and not re.fullmatch(r"CON(?:IN\$|OUT\$)?|CLOCK\$|PRN|AUX|NUL|COM[0-9]|LPT[0-9]",
                                         part.split(".")[0].rstrip(), re.I), "unsafe archive path")
        require(len(parts) > 1 or directory, "archive root must be a directory")
        key = "/".join(parts).casefold()
        require(key not in seen and len(seen) < MAX_FILES, "duplicate archive path or too many entries")
        seen.add(key)
        for end in range(1, len(parts) + 1):
            name = "/".join(parts[:end])
            identity = (name, end < len(parts) or directory)
            require(paths.setdefault(name.casefold(), identity) == identity, "archive case or file/directory collision")
        total += size
        require(0 <= size <= MAX_DOWNLOAD and total <= MAX_EXPANDED, "expanded files exceeded their limit")
        target = destination.joinpath(*parts)
        if directory:
            require(size == 0, "directory contains data")
            target.mkdir(parents=True, exist_ok=True)
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            with target.open("xb") as output:
                remaining = size
                while block := source.read(min(65536, remaining + 1)):
                    remaining -= len(block)
                    require(remaining >= 0, "oversized archive member")
                    output.write(block)
                require(remaining == 0, "truncated archive member")
            target.chmod(0o755 if mode & 0o111 else 0o644)

    if suffix == ".zip":
        with archive.open("rb") as source:
            source.seek(max(0, archive.stat().st_size - 65557))
            tail = source.read()
        offset = tail.rfind(b"PK\x05\x06")
        require(offset >= 0 and len(tail) - offset >= 22, "invalid zip directory")
        _, disk, start, local, count, length, position, comment = struct.unpack_from("<4s4H2LH", tail, offset)
        require(disk == start == 0 and local == count <= MAX_FILES and length <= 4 * 1024**2
                and position + length == archive.stat().st_size - len(tail) + offset
                and tail[max(0, offset - 20):offset - 16] != b"PK\x06\x07"
                and len(tail) - offset == 22 + comment, "oversized or unsupported zip directory")
        with zipfile.ZipFile(archive) as packed:
            require(len(packed.infolist()) == count, "zip directory count mismatch")
            for item in packed.infolist():
                kind = stat.S_IFMT(item.external_attr >> 16)
                require(item.orig_filename == item.filename and not item.flag_bits & 1
                        and kind in (0, stat.S_IFDIR if item.is_dir() else stat.S_IFREG)
                        and item.compress_type in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED)
                        and not item.external_attr & 0x400, "unsupported zip entry type, compression or encryption")
                with packed.open(item) as source:
                    extract(item.filename, item.is_dir(), item.file_size, item.external_attr >> 16, source)
    else:
        with gzip.open(archive, "rb") as compressed:
            reader = TarReader(compressed)
            with tarfile.open(fileobj=reader, mode="r:") as packed:
                for item in packed:
                    require((item.isreg() or item.isdir()) and item.sparse is None, "tar links, devices and sparse files are unsupported")
                    with contextlib.closing(packed.extractfile(item) if item.isreg() else io.BytesIO()) as source:
                        extract(item.name, item.isdir(), item.size, item.mode, source)
            require(reader.last == bytes(512), "missing or invalid tar end marker")
            padding = 0
            while block := reader.read(65536):
                require(not any(block), "nonzero data after tar end")
                padding += len(block)
            require(padding >= 512 and reader.position % 512 == 0, "truncated tar end")
    return destination / expected_root


def read_json(path):
    require(path.is_file() and not path.is_symlink() and path.stat().st_size <= 65536, f"invalid managed file: {path}")
    return json_bytes(path.read_bytes())


def atomic_json(path, value):
    with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=path.parent, delete=False) as stream:
        temporary = Path(stream.name)
        try:
            json.dump(value, stream, sort_keys=True)
            stream.flush()
            os.fsync(stream.fileno())
        except BaseException:
            stream.close()
            temporary.unlink(missing_ok=True)
            raise
    try:
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


@contextlib.contextmanager
def locked(root):
    require(not root.is_symlink(), "managed root is a symlink")
    root.mkdir(parents=True, exist_ok=True)
    require(not (root / ".lock").is_symlink(), "lock is a symlink")
    with (root / ".lock").open("a+b") as stream:
        if stream.seek(0, 2) == 0:
            stream.write(b"0")
            stream.flush()
        stream.seek(0)
        try:
            if os.name == "nt":
                import msvcrt
                msvcrt.locking(stream.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(stream.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as error:
            raise Refused("this profile is busy in another updater or game launch") from error
        try:
            yield
        finally:
            if os.name == "nt":
                stream.seek(0)
                msvcrt.locking(stream.fileno(), msvcrt.LK_UNLCK, 1)


def platform_name():
    require(platform.machine().lower() in ("x86_64", "amd64"), "only x86-64 devices are supported")
    require(os.name == "nt" or sys.platform.startswith("linux"), "this updater supports Windows and Linux")
    return "windows-x86_64" if os.name == "nt" else "linux-x86_64"


def selected_asset(value, system):
    return next(item for item in value["assets"] if item["platform"] == system)


def identifier(value, system):
    return value["commit"][:12] + "-" + selected_asset(value, system)["sha256"][:16]


def installed(root, version, args, system):
    require(matches(VERSION_ID, version), "invalid installed version pointer")
    folder = root / "versions" / version
    require(not folder.is_symlink(), "installed version is a symlink")
    value = manifest(read_json(folder / ".kjk-release.json"), args.repo)
    require(value["channel"] == args.channel and value["branch"] == args.branch
            and identifier(value, system) == version, "installed version identity mismatch")
    executable = folder / ("kjk.exe" if system.startswith("windows") else "kjk")
    require(executable.is_file() and not executable.is_symlink(), "installed executable is missing")
    return value, executable


def update(root, state, args, system):
    value = select(args)
    if state["current"] is not None and not args.tag:
        previous = installed(root, state["current"], args, system)[0]["commit"]
        if previous != value["commit"]:
            comparison = api(args.repo, f"compare/{previous}...{value['commit']}?per_page=1")
            require(isinstance(comparison, dict) and comparison.get("status") == "ahead"
                    and isinstance(comparison.get("merge_base_commit"), dict)
                    and comparison["merge_base_commit"].get("sha") == previous,
                    "automatic update is older or divergent; use --tag only for an intentional change")
    version, asset = identifier(value, system), selected_asset(value, system)
    destination = root / "versions" / version
    if destination.exists():
        require(installed(root, version, args, system)[0] == value, "immutable version metadata changed")
    else:
        with tempfile.TemporaryDirectory(prefix=".install-", dir=root) as temporary:
            staging = Path(temporary)
            with (staging / "download").open("xb") as output:
                digest = transfer(asset_url(args.repo, value["tag"], asset["name"]), asset["size"], asset["size"], output)
            require(digest == asset["sha256"], "archive SHA-256 mismatch")
            folder = unpack(staging / "download", staging / "unpacked", asset)
            extension = ".exe" if system.startswith("windows") else ""
            required = ["kjk" + extension, "kjk-save-editor" + extension, "kjk-update.py", "README.txt",
                        "LICENSE.txt", "NOTICE.txt", "THIRD-PARTY-LICENSES.html", "THIRD-PARTY-LICENSES-save-editor.html"]
            require(all((folder / name).is_file() and (folder / name).stat().st_size > 0 for name in required), "incomplete package")
            require(extension or all((folder / name).stat().st_mode & 0o111 for name in required[:2]), "programs are not executable")
            with (folder / ".kjk-release.json").open("x", encoding="utf-8") as marker:
                json.dump(value, marker)
            os.rename(folder, destination)
    if state["current"] != version:
        atomic_json(root / "current.json", {"current": version, "previous": state["current"]})
    print(f"Installed {value['tag']} ({value['commit'][:12]})")


def main():
    require(sys.version_info >= (3, 10), "Python 3.10 or later is required")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", default="shadowndacorner/kjk", help="trusted public GitHub owner/repository")
    parser.add_argument("--channel", choices=("stable", "dev"), default="stable")
    parser.add_argument("--branch", help="exact source branch (main for stable, develop for dev)")
    tags = parser.add_mutually_exclusive_group()
    tags.add_argument("--tag", help="exact published release tag")
    tags.add_argument("--tag-pattern", help="tag glob (v* for stable, dev-* for dev)")
    parser.add_argument("--root", type=Path, help="managed data parent, separate from existing game data")
    commands = parser.add_subparsers(dest="command", required=True)
    for command in ("check", "update", "rollback"):
        commands.add_parser(command)
    launch = commands.add_parser("launch", help="run the installed version; offline unless --update")
    launch.add_argument("--update", action="store_true", help="require a successful update before launching")
    launch.add_argument("game_args", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    args.repo = args.repo.lower()
    require(matches(r"[a-z0-9_-][a-z0-9_.-]*/[a-z0-9_-][a-z0-9_.-]*", args.repo), "invalid repository")
    if args.branch is None:
        args.branch = "main" if args.channel == "stable" else "develop"
    if args.tag_pattern is None:
        args.tag_pattern = "v*" if args.channel == "stable" else "dev-*"
    require(1 <= len(args.branch) <= 255 and all(ord(c) >= 32 for c in args.branch)
            and (args.channel != "stable" or args.branch == "main"), "invalid branch for channel")
    require(args.tag is None or matches(TAG, args.tag), "invalid tag")
    system = platform_name()
    if args.command == "check":
        value = select(args)
        print(f"Available {value['tag']} ({value['commit']}) for {args.repo} {args.channel}/{args.branch}")
        return 0
    base = (Path(os.environ.get("LOCALAPPDATA") or Path.home() / "AppData/Local") if os.name == "nt"
            else Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share")) / "kjk-updates"
    scope = {"repository": args.repo, "channel": args.channel, "branch": args.branch}
    root = (args.root or base).expanduser().resolve() / hashlib.sha256(json.dumps(scope, sort_keys=True).encode()).hexdigest()[:24]
    with locked(root):
        if (root / "scope.json").exists():
            require(read_json(root / "scope.json") == scope, "managed profile scope mismatch")
        else:
            require(set(root.iterdir()) == {root / ".lock"}, "unrecognized managed directory")
            atomic_json(root / "scope.json", scope)
        for name in ("versions", "profile"):
            require(not (root / name).is_symlink(), "managed directory is a symlink")
            (root / name).mkdir(exist_ok=True)
        state = read_json(root / "current.json") if (root / "current.json").exists() else {"current": None, "previous": None}
        require(isinstance(state, dict) and set(state) == {"current", "previous"}, "invalid current pointer")
        for version in state.values():
            if version is not None:
                installed(root, version, args, system)
        if args.command == "update" or (args.command == "launch" and args.update):
            update(root, state, args, system)
            state = read_json(root / "current.json")
        elif args.command == "rollback":
            require(state["previous"] is not None, "no previous version to roll back to")
            state = {"current": state["previous"], "previous": state["current"]}
            atomic_json(root / "current.json", state)
            print("Rolled back binaries; saves and settings are unchanged")
        print(f"Profile: {root / 'profile'}")
        if args.command == "launch":
            require(state["current"] is not None, "no installed version; run update first")
            value, executable = installed(root, state["current"], args, system)
            environment = dict(os.environ, KJK_SETTINGS=str(root / "profile/settings.json"),
                               KJK_SAVES=str(root / "profile/saves"), KJK_STORE=str(root / "profile/store"))
            game_args = args.game_args[1:] if args.game_args[:1] == ["--"] else args.game_args
            print(f"Launching installed {value['tag']}", flush=True)
            return subprocess.call([str(executable), *game_args], cwd=root / "profile", env=environment)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
    except (Refused, OSError, ValueError, KeyError, TypeError, RuntimeError, EOFError,
            tarfile.TarError, zipfile.BadZipFile, zlib.error) as error:
        print(f"kjk-update: {error}", file=sys.stderr)
        sys.exit(1)
