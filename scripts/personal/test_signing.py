"""Native signing checks; never launches the fixture executables or an app."""
import hashlib
import os
from pathlib import Path
import plistlib
import re
import subprocess
import sys
import tempfile
import unittest

import maintain


@unittest.skipUnless(sys.platform == 'darwin', 'macOS code signing only')
class SigningTests(unittest.TestCase):
    def test_personal_packaging_rejects_missing_team_and_adhoc_identity(self):
        script = Path(__file__).resolve().parents[1] / 'build-app.sh'
        env = dict(os.environ, HEX_TEST_BUILD='1', HEX_PERSONAL_BUILD='1')
        for key in ('HEX_TEAM_ID', 'HEX_CODESIGN_IDENTITY', 'HEX_RELEASE_UNSIGNED'):
            env.pop(key, None)
        result = subprocess.run(['bash', str(script)], env=env, text=True, capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Set HEX_TEAM_ID', result.stderr)
        env.update(HEX_TEAM_ID=maintain.PERSONAL_TEAM_ID, HEX_CODESIGN_IDENTITY='-')
        result = subprocess.run(['bash', str(script)], env=env, text=True, capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('not a Developer ID Application identity', result.stderr)

    def test_updated_code_keeps_identity_and_adhoc_code_is_rejected(self):
        identities = maintain.run('security', 'find-identity', '-v', '-p', 'codesigning')
        match = re.search(
            rf'"(Developer ID Application:.*\({maintain.PERSONAL_TEAM_ID}\))"', identities)
        self.assertIsNotNone(match, 'Personal builds require the configured Developer ID certificate')
        identity = match.group(1)
        validator = object.__new__(maintain.Fork)
        with tempfile.TemporaryDirectory(prefix='hex-signing-') as temporary:
            apps = []
            for version, signer in enumerate((identity, identity, '-'), start=1):
                app = Path(temporary) / f'{version}.app'
                executable = app / 'Contents/MacOS/fixture'
                executable.parent.mkdir(parents=True)
                (app / 'Contents/Info.plist').write_bytes(plistlib.dumps(dict(
                    CFBundleIdentifier='anomaly.hex.test', CFBundleExecutable='fixture',
                    CFBundleName='Hex Ralf', CFBundleDisplayName='Hex Ralf')))
                subprocess.run(['clang', '-x', 'c', '-', '-o', str(executable)],
                               input=f'int main(void) {{ return {version}; }}',
                               text=True, check=True, capture_output=True)
                subprocess.run(['codesign', '--force', '--timestamp=none', '--options',
                                'runtime', '--sign', signer, str(app)],
                               check=True, capture_output=True)
                apps.append(app)
            hashes = [hashlib.sha256((app / 'Contents/MacOS/fixture').read_bytes()).hexdigest()
                      for app in apps]
            self.assertNotEqual(hashes[0], hashes[1])
            requirements = []
            for app in apps[:2]:
                validator.validate_app(app)
                output = subprocess.check_output(['codesign', '-d', '-r-', str(app)],
                                                 stderr=subprocess.STDOUT, text=True)
                requirement = next(line.split('=> ', 1)[1] for line in output.splitlines()
                                   if 'designated => ' in line)
                self.assertNotIn('cdhash', requirement)
                requirements.append(requirement)
            self.assertEqual(requirements[0], requirements[1])
            for app, requirement in zip(reversed(apps[:2]), requirements):
                subprocess.run(['codesign', '--verify', '--strict', '-R', '=' + requirement, str(app)],
                               check=True, capture_output=True)
            for requirement in (requirements[0], 'identifier "anomaly.hex.other"'):
                app = apps[2] if requirement == requirements[0] else apps[0]
                result = subprocess.run(['codesign', '--verify', '--strict', '-R', '=' + requirement,
                                         str(app)], capture_output=True)
                self.assertNotEqual(result.returncode, 0)
            with self.assertRaises(subprocess.CalledProcessError):
                validator.validate_app(apps[2])


if __name__ == '__main__':
    unittest.main()
