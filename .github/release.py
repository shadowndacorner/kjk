#!/usr/bin/env python3
"""Validate delivery provenance, packages, and the published update manifest."""

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import tarfile
import tempfile
import zipfile


TAG = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}\Z")
STABLE = re.compile(r"v[0-9]+\.[0-9]+\.[0-9]+\Z")
CANDIDATE = re.compile(r"v[0-9]+\.[0-9]+\.[0-9]+-[A-Za-z0-9][A-Za-z0-9._-]*\Z")
DEV = re.compile(r"dev-[A-Za-z0-9][A-Za-z0-9._-]*\Z")
PLATFORMS = {
    "windows-x86_64": ("windows-x86_64", ".zip"),
    "linux-x86_64": ("linux-x86_64", ".tar.gz"),
    "macos-aarch64": ("macos-arm64", ".tar.gz"),
}
MANIFEST_FIELDS = (
    "schema_version", "repository", "channel", "branch", "tag", "commit", "version"
)


def git(*args):
    return subprocess.check_output(
        ["git", *args], text=True, stderr=subprocess.PIPE
    ).strip()


def on_branch(commit, branch):
    if not 1 <= len(branch) <= 255:
        raise ValueError("source branch must have 1 to 255 characters")
    git("check-ref-format", f"refs/heads/{branch}")
    git("fetch", "--no-tags", "origin", f"+refs/heads/{branch}:refs/remotes/origin/{branch}")
    result = subprocess.run(
        ["git", "merge-base", "--is-ancestor", commit, f"refs/remotes/origin/{branch}"],
        check=False,
    )
    if result.returncode not in (0, 1):
        raise ValueError(f"could not verify the history of {branch}")
    return result.returncode == 0


def verify_tag(tag, commit):
    ref = f"refs/tags/{tag}"
    if git("rev-parse", f"{ref}^{{commit}}") != commit:
        raise ValueError("the checked-out commit differs from the release tag")
    refs = dict(
        (name, sha) for sha, name in (
            line.split() for line in git("ls-remote", "--tags", "origin", ref, ref + "^{}").splitlines()
        )
    )
    if refs.get(ref + "^{}", refs.get(ref)) != commit:
        raise ValueError("the remote release tag is missing or no longer identifies this build")


def context():
    repository = os.environ["GITHUB_REPOSITORY"]
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("invalid repository")
    event_sha = os.environ["GITHUB_SHA"]
    if not re.fullmatch(r"[0-9a-f]{40}", event_sha):
        raise ValueError("invalid event commit")
    commit = git("rev-parse", "HEAD^{commit}")
    if git("rev-parse", event_sha + "^{commit}") != commit:
        raise ValueError("checkout does not match the event commit")
    event = os.environ["GITHUB_EVENT_NAME"]
    ref_type = os.environ["GITHUB_REF_TYPE"]
    ref_name = os.environ["GITHUB_REF_NAME"]
    if event not in ("push", "workflow_dispatch"):
        raise ValueError("only tag pushes and manual builds can release packages")
    tagged = event == "push" and ref_type == "tag"
    publication, prerelease, channel = "none", True, "dev"
    if tagged:
        tag = ref_name
        if not TAG.fullmatch(tag):
            raise ValueError("invalid release tag")
        verify_tag(tag, commit)
        if STABLE.fullmatch(tag):
            branch, channel, publication, prerelease = "main", "stable", "draft", False
            if git("ls-files", "--", "bumps/*.toml"):
                raise ValueError("stable releases require the pending bumps to be pinned first")
        elif DEV.fullmatch(tag):
            branch, publication = "develop", "prerelease"
        elif CANDIDATE.fullmatch(tag):
            branch = "main" if on_branch(commit, "main") else "develop"
            publication = "draft"
        else:
            raise ValueError("expected vMAJOR.MINOR.PATCH, a version candidate, or dev-<identifier>")
    else:
        branch = ref_name if ref_type == "branch" else (
            "main" if on_branch(commit, "main") else "develop"
        )
        described = git("describe", "--tags", "--match", "v*", "--always")
        slug = re.sub(r"[^A-Za-z0-9._-]", "-", branch)
        tag = f"pre-{slug[:max(1, 122 - len(described))]}-{described}"
        if event == "workflow_dispatch" and os.environ.get("DRAFT_PRERELEASE") == "true":
            publication = "draft"
    if not TAG.fullmatch(tag):
        raise ValueError("release version must be a safe tag of at most 128 ASCII characters")
    if not on_branch(commit, branch):
        raise ValueError(f"{commit} is not reachable from origin/{branch}")
    return dict(
        schema_version=1, repository=repository, channel=channel, branch=branch,
        tag=tag, commit=commit, version=tag, publication=publication,
        prerelease=prerelease, tagged=tagged,
    )


def verified_context(path):
    saved = json.loads(path.read_text(encoding="utf-8"))
    if saved != context():
        raise ValueError("release context changed since the build was started")
    return saved


def checksum(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def package_name(tag, platform):
    if not TAG.fullmatch(tag):
        raise ValueError("invalid package version")
    return f"kjk-{tag}-{platform}{PLATFORMS[platform][1]}"


def package_files(platform, dlss):
    files = {
        "README.txt", "LICENSE.txt", "NOTICE.txt", "THIRD-PARTY-LICENSES.html",
        "THIRD-PARTY-LICENSES-save-editor.html", "kjk-update.py",
    }
    programs = ("kjk", "kjk-save-editor")
    if platform == "macos-aarch64":
        for app, binary in zip(("kjk.app", "kjk Save Editor.app"), programs):
            files.update(f"{app}/Contents/{part}" for part in (
                f"MacOS/{binary}", "Info.plist", "Resources/kjk.icns", "_CodeSignature/CodeResources"
            ))
    else:
        files.update(name + (".exe" if platform == "windows-x86_64" else "") for name in programs)
    library = None
    if dlss:
        if platform == "macos-aarch64":
            raise ValueError("macOS packages do not have DLSS")
        sdk_platform = "Windows_x86_64" if platform == "windows-x86_64" else "Linux_x86_64"
        pattern = "nvngx_dlss.dll" if platform == "windows-x86_64" else "libnvidia-ngx-dlss.so.*"
        libraries = list((Path(os.environ["DLSS_SDK"]) / "lib" / sdk_platform / "rel").glob(pattern))
        if len(libraries) != 1:
            raise ValueError("expected exactly one SDK release DLSS library")
        library = libraries[0]
        files.update((library.name, "NVIDIA-RTX-SDK-LICENSE.txt", "NVIDIA-DLSS-NOTICES.txt"))
    return files, library


def unpack_checked(archive, tag, platform, destination, dlss=False):
    expected, library = package_files(platform, dlss)
    root = f"kjk-{tag}-{platform}"
    files = {f"{root}/{name}" for name in expected}
    directories = {root}
    for name in files:
        directories.update(str(parent) for parent in PurePosixPath(name).parents if str(parent) != ".")
    seen, found = set(), set()
    zipped = platform == "windows-x86_64"
    opener = zipfile.ZipFile if zipped else tarfile.open
    with opener(archive, "r") as packed:
        members = packed.infolist() if zipped else packed.getmembers()
        for member in members:
            name = member.filename if zipped else member.name
            path = PurePosixPath(name)
            key = str(path)
            if (
                not name or name not in (key, key + "/") or "\\" in name
                or path.is_absolute() or ".." in path.parts
                or key not in files | directories or key.casefold() in seen
            ):
                raise ValueError(f"unexpected or duplicate archive path: {name!r}")
            seen.add(key.casefold())
            mode = (member.external_attr >> 16) if zipped else member.mode
            directory = member.is_dir() if zipped else member.isdir()
            regular = stat.S_IFMT(mode) in (0, stat.S_IFREG) if zipped else member.isfile()
            directory_type = not zipped or stat.S_IFMT(mode) in (0, stat.S_IFDIR)
            if (directory and (not directory_type or key not in directories)) or (not directory and (not regular or key not in files)):
                raise ValueError(f"unexpected archive entry type: {name!r}")
            target = destination.joinpath(*path.parts)
            if directory:
                target.mkdir(parents=True, exist_ok=True)
                continue
            size = member.file_size if zipped else member.size
            if size <= 0:
                raise ValueError(f"empty package file: {name}")
            target.parent.mkdir(parents=True, exist_ok=True)
            source = packed.open(member) if zipped else packed.extractfile(member)
            with source, target.open("xb") as output:
                shutil.copyfileobj(source, output)
            if not zipped:
                target.chmod(mode & 0o777)
            found.add(key)
    if found != files:
        raise ValueError(f"missing package files: {sorted(files - found)}")
    if library and checksum(destination / root / library.name) != checksum(library):
        raise ValueError("packaged DLSS library differs from the SDK release library")
    return destination / root


def check_package(args):
    archive = args.packages / package_name(args.tag, args.platform)
    with tempfile.TemporaryDirectory(prefix="kjk-package-") as temporary:
        stage = unpack_checked(archive, args.tag, args.platform, Path(temporary), args.dlss)
        programs = []
        for app, binary in (("kjk.app", "kjk"), ("kjk Save Editor.app", "kjk-save-editor")):
            if args.platform == "macos-aarch64":
                bundle = stage / app
                subprocess.run(["plutil", "-lint", str(bundle / "Contents/Info.plist")], check=True)
                subprocess.run(["codesign", "--verify", "--strict", "-vv", str(bundle)], check=True)
                program = bundle / "Contents/MacOS" / binary
            else:
                program = stage / (binary + (".exe" if args.platform == "windows-x86_64" else ""))
            if args.platform != "windows-x86_64" and not program.stat().st_mode & 0o111:
                raise ValueError(f"program is not executable: {program.name}")
            subprocess.run([str(program), "--help"], check=True)
            programs.append(program)
        version = subprocess.check_output([str(programs[0]), "--version"], text=True).strip()
        if version != f"kjk {args.tag} ({args.commit})":
            raise ValueError(f"package version does not identify this build: {version}")
        print(version)


def assemble(saved, artifacts, output):
    output.mkdir()
    manifest = {key: saved[key] for key in MANIFEST_FIELDS}
    manifest["assets"] = []
    for platform, (artifact, _) in PLATFORMS.items():
        folder = artifacts / artifact
        if platform == "macos-aarch64":
            marker = artifacts / "sim-macos-arm64" / "sim-passed.txt"
            if not folder.is_dir() or not marker.is_file() or marker.read_text().strip() != "passed":
                print("::warning::macOS package unavailable or its determinism check did not pass")
                continue
        name = package_name(saved["tag"], platform)
        if not folder.is_dir() or sorted(path.name for path in folder.iterdir()) != [name]:
            raise ValueError(f"expected exactly the current {platform} archive: {name}")
        target = output / name
        shutil.copyfile(folder / name, target)
        size = target.stat().st_size
        if size <= 0:
            raise ValueError(f"empty archive: {name}")
        manifest["assets"].append(dict(platform=platform, name=name, sha256=checksum(target), size=size))
    (output / "kjk-release.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    sums = "".join(f"{checksum(path)}  {path.name}\n" for path in sorted(output.iterdir()))
    (output / "SHA256SUMS").write_text(sums, encoding="utf-8")
    platforms = ", ".join(asset["platform"] for asset in manifest["assets"])
    notes = (
        f"{'Stable build' if saved['channel'] == 'stable' else 'Development prerelease'} "
        f"from `{saved['branch']}` at `{saved['commit']}`.\n\n"
        f"Unsigned packages: {platforms}. Read README.txt before starting. "
        "kjk-release.json identifies this build for the updater; SHA256SUMS covers the archives and manifest. "
        f"Verify archive provenance with `gh attestation verify <archive> --repo {saved['repository']}`.\n"
    )
    if saved["prerelease"]:
        notes += "\nFor testing; saves have not been checked against earlier releases' save corpus.\n"
    if len(manifest["assets"]) == 2:
        notes += "\nThe macOS build is not available in this release.\n"
    output.with_suffix(".md").write_text(notes, encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    prepare = commands.add_parser("prepare")
    prepare.add_argument("--output", type=Path, required=True)
    verify = commands.add_parser("verify")
    verify.add_argument("--context", type=Path, required=True)
    check = commands.add_parser("check")
    check.add_argument("--packages", type=Path, required=True)
    check.add_argument("--tag", required=True)
    check.add_argument("--commit", required=True)
    check.add_argument("--platform", choices=PLATFORMS, required=True)
    check.add_argument("--dlss", action="store_true")
    collect = commands.add_parser("assemble")
    collect.add_argument("--context", type=Path, required=True)
    collect.add_argument("--artifacts", type=Path, required=True)
    collect.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "prepare":
        saved = context()
        args.output.write_text(json.dumps(saved, indent=2) + "\n", encoding="utf-8")
        if "GITHUB_OUTPUT" in os.environ:
            with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as outputs:
                for key in ("tag", "commit", "publication", "prerelease", "tagged"):
                    value = saved[key]
                    outputs.write(f"{key}={str(value).lower() if isinstance(value, bool) else value}\n")
    elif args.command == "verify":
        verified_context(args.context)
    elif args.command == "check":
        check_package(args)
    else:
        assemble(verified_context(args.context), args.artifacts, args.output)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError, tarfile.TarError, zipfile.BadZipFile) as error:
        raise SystemExit(f"release: {getattr(error, 'stderr', None) or error}") from error
