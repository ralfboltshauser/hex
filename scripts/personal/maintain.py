#!/usr/bin/env python3
"""Single-Mac personal fork: prepare, check, build, publish, and explicitly install."""
import argparse
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import uuid


PERSONAL_TEAM_ID = '76625SQ67N'
PERSONAL_REQUIREMENT = (
    'identifier "anomaly.hex.test" and anchor apple generic '
    f'and certificate leaf[subject.OU] = "{PERSONAL_TEAM_ID}" '
    'and certificate leaf[field.1.2.840.113635.100.6.1.13] exists'
)


def run(*args, cwd=None):
    return subprocess.check_output(args, cwd=cwd, text=True).strip()


def save(path, value):
    temporary = path.with_suffix('.tmp')
    temporary.write_text(json.dumps(value, indent=2) + '\n')
    temporary.replace(path)


def digest(directory):
    result = hashlib.sha256()
    for path in sorted(directory.rglob('*')):
        relative = str(path.relative_to(directory))
        result.update(relative.encode() + b'\0')
        if path.is_symlink():
            result.update(b'link\0' + os.readlink(path).encode())
        elif path.is_file():
            result.update(path.read_bytes())
    return result.hexdigest()


class Fork:
    def __init__(self, source):
        self.source = Path(run('git', 'rev-parse', '--show-toplevel', cwd=source))
        common = Path(run('git', 'rev-parse', '--path-format=absolute', '--git-common-dir', cwd=source))
        self.root = common.parent
        self.state = self.root / '.hex-fork'
        self.state.mkdir(exist_ok=True)

    def git(self, *args, cwd=None):
        return run('git', *args, cwd=cwd or self.source)

    @contextlib.contextmanager
    def lock(self):
        with (self.state / 'lock').open('w') as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError as error:
                raise RuntimeError('Another personal maintenance operation is running') from error
            yield

    def clean_head(self, cwd=None):
        if self.git('status', '--porcelain', '--untracked-files=all', cwd=cwd):
            raise RuntimeError('Commit or preserve unfinished changes before verification')
        return self.git('rev-parse', 'HEAD', cwd=cwd)

    def ancestor(self, base, head):
        return subprocess.run(['git', 'merge-base', '--is-ancestor', base, head],
                              cwd=self.source, check=False).returncode == 0

    def logged(self, command, *, env=None):
        log = self.state / f'{uuid.uuid4().hex}.log'
        print(f'Running {command}; log: {log}', flush=True)
        with log.open('w') as output:
            subprocess.run(command, cwd=self.source, env=env, stdout=output,
                           stderr=subprocess.STDOUT, check=True)
        return str(log)

    def prepare(self):
        release = json.loads(run('gh', 'api', 'repos/anomalyco/hex/releases/latest'))
        tag = release['tag_name']
        if release['draft'] or release['prerelease'] or not tag.startswith('v'):
            raise RuntimeError('Expected a published stable release')
        self.git('check-ref-format', f'refs/tags/{tag}')
        self.git('fetch', '--no-tags', 'origin', 'personal')
        self.git('fetch', '--no-tags', 'upstream',
                 f'refs/tags/{tag}:refs/hex-personal/upstream/{tag}')
        personal = self.git('rev-parse', 'origin/personal')
        upstream = self.git('rev-parse', f'refs/hex-personal/upstream/{tag}^{{commit}}')
        report = dict(tag=tag, personal=personal, upstream=upstream, phase='current')
        if self.ancestor(upstream, personal):
            save(self.state / 'latest.json', report)
            print('personal already contains the latest stable release')
            return
        branch = f'update/{tag}-{personal[:12]}'
        candidate = self.root.parent / f'hex-{tag}-{personal[:12]}'
        report.update(candidate=str(candidate), branch=branch, phase='preparing')
        save(self.state / 'latest.json', report)
        if not candidate.exists():
            self.git('worktree', 'add', '-b', branch, str(candidate), personal)
        else:
            if self.git('branch', '--show-current', cwd=candidate) != branch:
                raise RuntimeError('Existing candidate has a different branch; preserve and inspect it')
            self.clean_head(candidate)
        if not self.ancestor(personal, self.git('rev-parse', 'HEAD', cwd=candidate)):
            raise RuntimeError('Candidate no longer contains its pinned personal input')
        try:
            self.git('merge', '--no-edit', upstream, cwd=candidate)
            report['phase'] = 'needs-review'
        except subprocess.CalledProcessError:
            report['phase'] = 'merge-blocked'
            save(self.state / 'latest.json', report)
            raise
        save(self.state / 'latest.json', report)
        print(json.dumps(report, indent=2))

    def receipt(self, commit):
        return self.state / f'verified-{commit}.json'

    def verify(self):
        commit = self.clean_head()
        receipt = self.receipt(commit)
        receipt.unlink(missing_ok=True)
        logs = []
        env = dict(os.environ, NEXTEST_TEST_THREADS='4')
        for command in (['bash', 'scripts/check'], ['bash', 'scripts/personal/check.sh']):
            logs.append(self.logged(command, env=env))
            if self.clean_head() != commit:
                raise RuntimeError('Source changed during checks; verify again')
        save(receipt, dict(commit=commit, logs=logs))
        print(f'Checks completed for {commit}')

    def verified(self):
        commit = self.clean_head()
        receipt = json.loads(self.receipt(commit).read_text())
        if receipt['commit'] != commit:
            raise RuntimeError('Verification does not match this source')
        return commit

    def build(self):
        commit = self.verified()
        destination = self.state / 'builds' / commit
        if destination.exists():
            self.artifact(commit)
            print(destination / 'Hex Ralf.app')
            return
        env = dict(os.environ, HEX_TEST_BUILD='1', HEX_LOCAL_BUILD='1', HEX_PERSONAL_BUILD='1',
                   HEX_TEAM_ID=PERSONAL_TEAM_ID, HEX_COMMIT=commit,
                   CARGO_TARGET_DIR=str(self.source / 'target'))
        env.pop('HEX_RELEASE_UNSIGNED', None)
        self.logged(['bash', 'scripts/build-app.sh'], env=env)
        if self.verified() != commit:
            raise RuntimeError('Source changed during build')
        app = self.source / 'target/app/Hex Ralf.app'
        self.validate_app(app)
        destination.parent.mkdir(exist_ok=True)
        temporary = destination.parent / f'.{commit}-{uuid.uuid4().hex}'
        temporary.mkdir()
        shutil.copytree(app, temporary / app.name, symlinks=True)
        save(temporary / 'build.json', dict(commit=commit, sha256=digest(temporary / app.name)))
        temporary.rename(destination)
        print(destination / app.name)

    def validate_app(self, app):
        identifier = run('/usr/libexec/PlistBuddy', '-c', 'Print :CFBundleIdentifier',
                         str(app / 'Contents/Info.plist'))
        if identifier != 'anomaly.hex.test':
            raise RuntimeError('Only isolated Hex Ralf bundles may be installed')
        for key in ('CFBundleName', 'CFBundleDisplayName'):
            name = run('/usr/libexec/PlistBuddy', '-c', f'Print :{key}',
                       str(app / 'Contents/Info.plist'))
            if name != 'Hex Ralf':
                raise RuntimeError(f'Expected Hex Ralf for {key}, got {name}')
        run('codesign', '--verify', '--deep', '--strict', str(app))
        run('codesign', '--verify', '--strict', '-R', '=' + PERSONAL_REQUIREMENT, str(app))

    def artifact(self, commit):
        directory = self.state / 'builds' / commit
        receipt = json.loads((directory / 'build.json').read_text())
        app = directory / 'Hex Ralf.app'
        if receipt['commit'] != commit or receipt['sha256'] != digest(app):
            raise RuntimeError('Build artifact changed; preserve it and rebuild')
        self.validate_app(app)
        return app

    def promote(self):
        commit = self.verified()
        self.artifact(commit)
        self.git('fetch', '--no-tags', 'origin', 'personal')
        if not self.ancestor('origin/personal', commit):
            raise RuntimeError('personal advanced; merge it and reverify')
        self.git('push', 'origin', f'{commit}:refs/heads/personal')
        print(f'Published {commit}; installation is separate')

    def install(self):
        commit = self.verified()
        app = self.artifact(commit)
        destination = Path.home() / 'Applications/Hex Ralf.app'
        processes = run('ps', '-axo', 'command=')
        if str(destination / 'Contents/MacOS/hex') in processes:
            raise RuntimeError('Quit Hex Ralf before installing; no process was stopped')
        destination.parent.mkdir(exist_ok=True)
        temporary = destination.parent / f'.Hex-Ralf-{uuid.uuid4().hex}.app'
        shutil.copytree(app, temporary, symlinks=True)
        self.validate_app(temporary)
        previous = None
        if destination.exists():
            previous = destination.parent / f'Hex Ralf Backup-{uuid.uuid4().hex}.app'
            destination.rename(previous)
        try:
            temporary.rename(destination)
        except BaseException:
            if previous:
                previous.rename(destination)
            raise
        save(self.state / 'installed.json', dict(commit=commit, app=str(destination),
                                               previous=str(previous) if previous else None))
        print(f'Installed {destination}. Launch explicitly and check recording/transcription/paste.')

    def status(self):
        for name in ('latest.json', 'installed.json'):
            path = self.state / name
            print(f'{name}: {path.read_text() if path.exists() else "none"}')
        commit = self.git('rev-parse', 'HEAD')
        print(f'Current checkout: {self.source}\nCommit: {commit}')
        print(f'Completed checks: {self.receipt(commit).exists()}')
        print(f'Build record: {self.state / "builds" / commit / "build.json"}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=['prepare', 'verify', 'build', 'promote', 'install', 'status'])
    args = parser.parse_args()
    fork = Fork(Path.cwd())
    with fork.lock():
        getattr(fork, args.operation)()


if __name__ == '__main__':
    try:
        main()
    except (RuntimeError, OSError, subprocess.CalledProcessError, ValueError) as error:
        sys.exit(str(error))
