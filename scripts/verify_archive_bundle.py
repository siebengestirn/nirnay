#!/usr/bin/env python3
"""Verify that a full archive's declared source commit is recoverable.

A source snapshot and its Git bundle can silently diverge: the archive can
carry the latest working tree while the bundle still points at an older
`main` that does not contain that commit. This verifier makes the two agree.

For an archive it checks:

* `ARCHIVE-MANIFEST.json` declares a valid `checked_out_commit`;
* the bundled `git/nirnay-all-refs.bundle` digest matches the manifest;
* `refs/heads/main` is present in the bundle and resolves to that commit;
* the commit is an object in the bundled history and an ancestor of HEAD.

No network, no credentials, no writes outside a temporary directory.

    python3 scripts/verify_archive_bundle.py /path/to/archive.zip
    python3 scripts/verify_archive_bundle.py --self-test
"""
from __future__ import annotations

import json
import re
import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

BUNDLE_MEMBER = "git/nirnay-all-refs.bundle"
MANIFEST_MEMBER = "ARCHIVE-MANIFEST.json"
SHA_RE = re.compile(r"^[0-9a-f]{7,40}$")


def read_manifest(archive: Path) -> dict:
    with zipfile.ZipFile(archive) as bundle:
        return json.loads(bundle.read(MANIFEST_MEMBER))


def extract_bundle(archive: Path, destination: Path) -> Path:
    with zipfile.ZipFile(archive) as bundle:
        payload = bundle.read(BUNDLE_MEMBER)
    destination.write_bytes(payload)
    return destination


def bundle_heads(bundle: Path) -> dict[str, str]:
    result = subprocess.run(
        ["git", "bundle", "list-heads", str(bundle)],
        capture_output=True,
        text=True,
        check=True,
    )
    heads: dict[str, str] = {}
    for line in result.stdout.splitlines():
        oid, _, ref = line.partition(" ")
        heads[ref.strip()] = oid.strip()
    return heads


def sha256(path: Path) -> str:
    import hashlib

    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def check_source_tree(
    archive: Path, manifest: dict, clone: Path, declared: str
) -> list[str]:
    """Require the archived source tree to match the declared commit's tree.

    A clean checkout of ``declared`` is the authority for every tracked path.
    Extra files are allowed only when the manifest declares them as untracked
    working-tree inclusions (for example STYLES.md and references/).
    """
    errors: list[str] = []
    tracked = set(
        subprocess.check_output(
            ["git", "-C", str(clone), "ls-tree", "-r", "--name-only", declared],
            text=True,
        ).splitlines()
    )
    untracked = set(manifest.get("untracked_source_files", []))
    with zipfile.ZipFile(archive) as bundle:
        archived = {
            name[len("nirnay/"):]: name
            for name in bundle.namelist()
            if name.startswith("nirnay/")
        }
    for relative, member in archived.items():
        if relative in tracked:
            blob = subprocess.check_output(
                ["git", "-C", str(clone), "rev-parse", f"{declared}:{relative}"],
                text=True,
            ).strip()
            actual = subprocess.run(
                ["git", "-C", str(clone), "hash-object", "--stdin"],
                input=zipfile.ZipFile(archive).read(member),
                capture_output=True,
            ).stdout.decode().strip()
            if actual != blob:
                errors.append(
                    f"source tree file {member} differs from declared commit {declared}"
                )
        elif relative not in untracked:
            errors.append(f"source tree file {member} is not in the declared commit or manifest")
    for relative in sorted(tracked - set(archived)):
        errors.append(f"tracked file {relative} is missing from the archive source tree")
    return errors


def verify(archive: Path) -> list[str]:
    """Return one message per failed invariant; empty means the archive is sound."""
    errors: list[str] = []
    if not archive.is_file():
        return [f"archive not found: {archive}"]
    try:
        manifest = read_manifest(archive)
    except Exception as error:
        return [f"cannot read {MANIFEST_MEMBER}: {type(error).__name__}: {error}"]

    declared = str(manifest.get("checked_out_commit", ""))
    if not SHA_RE.fullmatch(declared):
        errors.append(f"declared checked_out_commit {declared!r} is not a hex SHA")
    declared_main = str(manifest.get("main_commit", ""))
    if declared_main and declared_main != declared:
        errors.append(
            f"manifest main_commit {declared_main} != checked_out_commit {declared}"
        )

    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        bundle = root / "archive.bundle"
        try:
            extract_bundle(archive, bundle)
        except Exception as error:
            return errors + [f"cannot extract {BUNDLE_MEMBER}: {error}"]

        declared_bundle = (manifest.get("git_bundle") or {}).get("sha256", "")
        actual_bundle = sha256(bundle)
        if declared_bundle and declared_bundle != actual_bundle:
            errors.append(
                f"bundle digest {actual_bundle} != manifest {declared_bundle}"
            )

        try:
            heads = bundle_heads(bundle)
        except subprocess.CalledProcessError as error:
            return errors + [f"git bundle list-heads failed: {error}"]

        main_ref = heads.get("refs/heads/main", "")
        if not main_ref:
            errors.append("bundle has no refs/heads/main")
        elif declared and main_ref != declared and main_ref.startswith(declared):
            # a longer SHA with the same prefix is acceptable
            pass
        elif declared and declared.startswith(main_ref) and len(main_ref) >= 7:
            pass
        elif declared and main_ref != declared:
            errors.append(
                f"bundle refs/heads/main {main_ref} != declared source commit {declared}"
            )

        clone = root / "clone"
        try:
            subprocess.run(
                ["git", "clone", "-q", str(bundle), str(clone)],
                capture_output=True,
                text=True,
                check=True,
            )
            if declared:
                exists = subprocess.run(
                    ["git", "-C", str(clone), "cat-file", "-e", f"{declared}^{{commit}}"],
                    capture_output=True,
                    text=True,
                )
                if exists.returncode != 0:
                    errors.append(f"declared commit {declared} is absent from bundled history")
                else:
                    ancestor = subprocess.run(
                        ["git", "-C", str(clone), "merge-base", "--is-ancestor", declared, "HEAD"],
                        capture_output=True,
                        text=True,
                    )
                    if ancestor.returncode != 0:
                        errors.append(
                            f"declared commit {declared} is not an ancestor of bundle HEAD"
                        )
                    else:
                        errors.extend(check_source_tree(archive, manifest, clone, declared))
        except subprocess.CalledProcessError as error:
            errors.append(f"cannot clone bundle: {error}")
    return errors


def make_archive(
    directory: Path, bundle_members: dict[str, str], declared: str, source_repo: Path
) -> Path:
    """Create a synthetic archive for self-tests from a real git repo/bundle."""
    root = Path(directory)
    bundle = root / "b.bundle"
    bundle.write_bytes(Path(bundle_members["bundle_source"]).read_bytes())
    manifest = {
        "checked_out_commit": declared,
        "main_commit": declared,
        "git_bundle": {"sha256": sha256(bundle)},
        "untracked_source_files": [],
    }
    tree = subprocess.check_output(
        ["git", "-C", str(source_repo), "ls-tree", "-r", "--name-only", declared],
        text=True,
    ).splitlines()
    archive = root / "archive.zip"
    with zipfile.ZipFile(archive, "w") as bundle_zip:
        bundle_zip.writestr(MANIFEST_MEMBER, json.dumps(manifest))
        bundle_zip.writestr(BUNDLE_MEMBER, bundle.read_bytes())
        for relative in tree:
            payload = subprocess.check_output(
                ["git", "-C", str(source_repo), "show", f"{declared}:{relative}"]
            )
            bundle_zip.writestr(f"nirnay/{relative}", payload)
    return archive


class ArchiveBundleTests(unittest.TestCase):
    def git(self, directory: Path, *args: str) -> str:
        return subprocess.check_output(
            ["git", "-C", str(directory), *args], text=True
        ).strip()

    def make_repo(self, directory: Path, commits: int) -> tuple[Path, str]:
        repo = directory / "repo"
        repo.mkdir()
        self.git(repo, "init", "-q")
        self.git(repo, "config", "user.name", "Test")
        self.git(repo, "config", "user.email", "test@example.invalid")
        head = ""
        for index in range(commits):
            (repo / "f.txt").write_text(f"{index}\n")
            self.git(repo, "add", "f.txt")
            self.git(repo, "commit", "-q", "-m", f"c{index}")
            head = self.git(repo, "rev-parse", "HEAD")
        return repo, head

    def make_bundle(self, directory: Path, repo: Path, name: str) -> Path:
        bundle = directory / name
        subprocess.check_call(
            ["git", "-C", str(repo), "bundle", "create", str(bundle), "refs/heads/main"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        return bundle

    def test_matching_archive_passes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo, head = self.make_repo(root, 2)
            bundle = self.make_bundle(root, repo, "good.bundle")
            archive = make_archive(root, {"bundle_source": str(bundle)}, head, repo)
            self.assertEqual(verify(archive), [])

    def test_stale_bundle_is_caught(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo, head = self.make_repo(root, 2)
            self.git(repo, "reset", "-q", "--hard", "HEAD~1")
            stale = self.make_bundle(root, repo, "stale.bundle")
            archive = make_archive(root, {"bundle_source": str(stale)}, head, repo)
            messages = verify(archive)
            self.assertTrue(
                any("refs/heads/main" in m or "absent" in m for m in messages),
                f"stale bundle must fail: {messages}",
            )

    def test_missing_main_ref_is_caught(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo, head = self.make_repo(root, 1)
            self.git(repo, "branch", "other")
            bundle = root / "bare.bundle"
            subprocess.check_call(
                ["git", "-C", str(repo), "bundle", "create", str(bundle), "refs/heads/other"],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            archive = make_archive(root, {"bundle_source": str(bundle)}, head, repo)
            messages = verify(archive)
            self.assertTrue(any("refs/heads/main" in m for m in messages), str(messages))

    def test_source_tree_tampering_is_caught(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo, head = self.make_repo(root, 2)
            bundle = self.make_bundle(root, repo, "good.bundle")
            archive = make_archive(root, {"bundle_source": str(bundle)}, head, repo)
            # rewrite the archived source with different content
            with zipfile.ZipFile(archive) as original:
                names = original.namelist()
                payloads = {name: original.read(name) for name in names}
            payloads["nirnay/f.txt"] = b"tampered\n"
            with zipfile.ZipFile(archive, "w") as rewritten:
                for name, payload in payloads.items():
                    rewritten.writestr(name, payload)
            messages = verify(archive)
            self.assertTrue(
                any("differs from declared commit" in m for m in messages),
                f"source tree tampering must fail: {messages}",
            )

    def test_short_source_commit_is_accepted(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo, head = self.make_repo(root, 1)
            bundle = self.make_bundle(root, repo, "good.bundle")
            archive = make_archive(root, {"bundle_source": str(bundle)}, head[:7], repo)
            self.assertEqual(verify(archive), [])


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(ArchiveBundleTests)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1
    if len(sys.argv) != 2:
        print("usage: verify_archive_bundle.py <archive.zip> | --self-test", file=sys.stderr)
        return 2
    errors = verify(Path(sys.argv[1]))
    if errors:
        for error in errors:
            print(f"FAIL: {error}", file=sys.stderr)
        print(f"{len(errors)} archive/bundle invariant(s) failed", file=sys.stderr)
        return 1
    print("PASS: archive source commit is recoverable from the bundled history")
    return 0


if __name__ == "__main__":
    sys.exit(main())
