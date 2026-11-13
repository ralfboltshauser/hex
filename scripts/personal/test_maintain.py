import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import maintain


class WorkflowTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.repo = self.directory / 'hex'
        maintain.run('git', 'init', '-b', 'personal', str(self.repo))
        self.git('config', 'user.name', 'Test')
        self.git('config', 'user.email', 'test@example.invalid')
        self.git('config', 'commit.gpgsign', 'false')
        (self.repo / '.gitignore').write_text('.hex-fork/\n')
        (self.repo / 'policy').write_text('personal policy\n')
        self.commit('base')
        self.git('remote', 'add', 'origin', str(self.repo))
        self.git('remote', 'add', 'upstream', str(self.repo))
        self.fork = maintain.Fork(self.repo)

    def git(self, *args):
        return maintain.run('git', *args, cwd=self.repo)

    def commit(self, message):
        self.git('add', '.')
        self.git('commit', '-m', message)
        return self.git('rev-parse', 'HEAD')

    def release(self, conflict=False):
        self.git('switch', '-c', 'upstream-release')
        (self.repo / ('policy' if conflict else 'upstream-feature')).write_text('new upstream\n')
        revision = self.commit('upstream')
        self.git('tag', 'v2.0.0')
        self.git('switch', 'personal')
        if conflict:
            (self.repo / 'policy').write_text('changed personal policy\n')
            self.commit('personal change')
        return revision

    def prepare(self):
        original = maintain.run

        def command(*args, **kwargs):
            if args[:2] == ('gh', 'api'):
                return json.dumps(dict(tag_name='v2.0.0', draft=False, prerelease=False))
            return original(*args, **kwargs)

        with patch.object(maintain, 'run', command):
            self.fork.prepare()
        return json.loads((self.fork.state / 'latest.json').read_text())

    def test_candidate_preserves_both_inputs_and_reuses_resolved_candidate(self):
        upstream = self.release()
        personal = self.git('rev-parse', 'HEAD')
        report = self.prepare()
        candidate = Path(report['candidate'])
        head = self.fork.git('rev-parse', 'HEAD', cwd=candidate)
        self.assertTrue(self.fork.ancestor(personal, head))
        self.assertTrue(self.fork.ancestor(upstream, head))
        self.assertEqual((candidate / 'policy').read_text(), 'personal policy\n')
        self.assertEqual(self.prepare()['candidate'], str(candidate))
        (candidate / 'unfinished').write_text('preserve me')
        with self.assertRaisesRegex(RuntimeError, 'unfinished'):
            self.prepare()
        self.assertEqual((candidate / 'unfinished').read_text(), 'preserve me')

    def test_conflict_preserves_candidate_and_personal(self):
        self.release(conflict=True)
        personal = self.git('rev-parse', 'HEAD')
        with self.assertRaises(subprocess.CalledProcessError):
            self.prepare()
        report = json.loads((self.fork.state / 'latest.json').read_text())
        self.assertEqual(report['phase'], 'merge-blocked')
        self.assertIn('<<<<<<<', (Path(report['candidate']) / 'policy').read_text())
        self.assertEqual(self.git('rev-parse', 'HEAD'), personal)

    def test_failed_recheck_revokes_success_and_changed_source_cannot_build(self):
        script = self.repo / 'scripts'
        (script / 'personal').mkdir(parents=True)
        (script / 'personal/release-check.sh').write_text('bash scripts/check && bash scripts/personal/check.sh\n')
        (script / 'check').write_text('exit 0\n')
        (script / 'personal/check.sh').write_text('test ! -e .hex-fork/fail\n')
        head = self.commit('checks')
        self.fork.verify()
        self.assertEqual(self.fork.verified(), head)
        (self.fork.state / 'fail').touch()
        with self.assertRaises(subprocess.CalledProcessError):
            self.fork.verify()
        self.assertFalse(self.fork.receipt(head).exists())
        with self.assertRaises(FileNotFoundError):
            self.fork.build()
        (self.fork.state / 'fail').unlink()
        self.fork.verify()
        (self.repo / 'policy').write_text('unverified\n')
        with self.assertRaisesRegex(RuntimeError, 'unfinished'):
            self.fork.build()
        self.commit('changed source')
        with self.assertRaises(FileNotFoundError):
            self.fork.build()

    def test_worktrees_share_state_and_lock(self):
        other = self.directory / 'feature'
        self.git('worktree', 'add', '-b', 'feature', str(other))
        fork = maintain.Fork(other)
        self.assertEqual(fork.state, self.fork.state)
        with self.fork.lock():
            with self.assertRaisesRegex(RuntimeError, 'running'):
                with fork.lock():
                    self.fail('second writer entered')

    def test_install_archives_previous_app_outside_applications(self):
        home = self.directory / 'home'
        destination = home / 'Applications/Hex Ralf.app'
        destination.mkdir(parents=True)
        (destination / 'version').write_text('old')
        app = self.directory / 'candidate.app'
        app.mkdir()
        (app / 'version').write_text('new')
        original_run = maintain.run
        original_rename = Path.rename

        def fail_replacement(path, target):
            if path.name.startswith('.Hex-Ralf-') and path.suffix == '.app':
                raise OSError('replacement failed')
            return original_rename(path, target)

        with patch.object(Path, 'home', return_value=home), \
                patch.object(self.fork, 'verified', return_value='commit'), \
                patch.object(self.fork, 'artifact', return_value=app), \
                patch.object(self.fork, 'validate_app'), \
                patch.object(maintain, 'run', wraps=maintain.run) as command:
            command.side_effect = lambda *args, **kwargs: '' if args[0] == 'ps' else original_run(*args, **kwargs)
            with patch.object(Path, 'rename', fail_replacement):
                with self.assertRaisesRegex(OSError, 'replacement failed'):
                    self.fork.install()
            self.assertEqual((destination / 'version').read_text(), 'old')
            # Remove only the staged test candidate left by the failed replacement.
            for staged in destination.parent.glob('.Hex-Ralf-*.app'):
                import shutil
                shutil.rmtree(staged)
            self.fork.install()
        receipt = json.loads((self.fork.state / 'installed.json').read_text())
        archive = Path(receipt['previous'])
        self.assertEqual(archive.parent, self.fork.state / 'backups.noindex')
        restored = self.directory / 'restored'
        maintain.run('ditto', '-x', '-k', str(archive), str(restored))
        self.assertEqual((restored / 'Hex Ralf.app/version').read_text(), 'old')
        self.assertEqual((destination / 'version').read_text(), 'new')
        self.assertEqual(list(destination.parent.iterdir()), [destination])
        self.assertFalse(list(archive.parent.glob('*.backup')))

    def test_artifact_digest_detects_bytes_paths_and_symlink_changes(self):
        app = self.directory / 'Test.app'
        app.mkdir()
        (app / 'executable').write_bytes(b'original')
        original = maintain.digest(app)
        (app / 'executable').write_bytes(b'changed')
        self.assertNotEqual(original, maintain.digest(app))
        original = maintain.digest(app)
        os.symlink('executable', app / 'helper')
        self.assertNotEqual(original, maintain.digest(app))
        original = maintain.digest(app)
        (app / 'helper').unlink()
        os.symlink('other', app / 'helper')
        self.assertNotEqual(original, maintain.digest(app))


if __name__ == '__main__':
    unittest.main()
