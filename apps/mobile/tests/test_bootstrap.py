"""Verify the remote bootstrap's trust boundary without downloading or starting services."""
import base64
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[1] / "plugins/remote/shared/bootstrap.py"
spec = importlib.util.spec_from_file_location("bootstrap", SOURCE)
bootstrap = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bootstrap)


class BootstrapTests(unittest.TestCase):
    def test_artifact_origin_is_restricted(self):
        for url in ["http://dl.velaterm.com/file", "https://dl.velaterm.com.evil.test/file", "file:///tmp/file"]:
            with self.assertRaises(ValueError):
                bootstrap.fetch(url)

    def test_hash_mismatch_is_rejected_before_signature_execution(self):
        with patch.object(bootstrap.subprocess, "run") as run:
            with self.assertRaisesRegex(ValueError, "SHA-256"):
                bootstrap.verify(b"modified", {"sha256": "0" * 64})
            run.assert_not_called()

    def test_real_ed25519_signature_and_tampering(self):
        openssl = "/opt/homebrew/opt/openssl@3/bin/openssl"
        if not Path(openssl).exists():
            openssl = shutil.which("openssl")
        self.assertTrue(openssl, "OpenSSL is required for signature verification tests")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def command(*args):
                subprocess.run([openssl, *args], check=True, capture_output=True)
            command("genpkey", "-algorithm", "ED25519", "-out", str(root / "key.pem"))
            command("pkey", "-in", str(root / "key.pem"), "-pubout", "-outform", "DER", "-out", str(root / "public.der"))
            data = b"a signed VelaTerm artifact fixture"
            (root / "message").write_bytes(hashlib.blake2b(data).digest())
            command("pkeyutl", "-sign", "-inkey", str(root / "key.pem"), "-rawin", "-in", str(root / "message"), "-out", str(root / "signature"))
            key_id = b"12345678"
            public = base64.b64encode(b"Ed" + key_id + (root / "public.der").read_bytes()[-32:]).decode()
            key_file = base64.b64encode(("untrusted comment: test key\n" + public + "\n").encode()).decode()
            signature = b"ED" + key_id + (root / "signature").read_bytes()
            entry = {"sha256": hashlib.sha256(data).hexdigest(), "signature": base64.b64encode(b"untrusted comment: fixture\n" + base64.b64encode(signature) + b"\n").decode()}
            with patch.object(bootstrap, "PUBLIC_KEY_FILE", key_file), patch.object(bootstrap.shutil, "which", return_value=openssl):
                bootstrap.verify(data, entry)
                corrupt = signature[:-1] + bytes([signature[-1] ^ 1])
                entry["signature"] = base64.b64encode(b"untrusted comment: fixture\n" + base64.b64encode(corrupt) + b"\n").decode()
                with self.assertRaisesRegex(ValueError, "signature verification"):
                    bootstrap.verify(data, entry)

    def test_shared_key_and_version_match_desktop(self):
        root = SOURCE.parents[5]
        self.assertEqual(bootstrap.VERSION, json.loads((root / "package.json").read_text())["version"])
        self.assertIn(bootstrap.PUBLIC_KEY_FILE, (root / "src-tauri/src/server_supply.rs").read_text())


if __name__ == "__main__":
    unittest.main()
