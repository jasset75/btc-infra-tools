import contextlib
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch

from mining_health import NoRedirect, core_query, healthy, main, rpc_query


class MiningHealthTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        credentials = Path(self.temp.name) / "rpc.env"
        credentials.write_text("export RPC_USER='health'\nRPC_PASSWORD='a secret # literal'\n")
        self.rpc = dict(endpoint="http://127.0.0.1:8332/", credentials_file=str(credentials),
                        user_key="RPC_USER", password_key="RPC_PASSWORD")

    def test_all_protected_checks_are_required(self):
        block = dict(blocks=12, headers=12, initialblockdownload=False)
        net = dict(networkactive=True, connections=5)
        miner = dict(isUsingFallbackStratum=0, stratumURL="primary", stratumPort=3333, hashRate=1000)
        config = dict(stratum_host="primary", stratum_port=3333, tcp_peer="primary:3333->miner:")
        tcp = "primary:3333->miner:6000 (ESTABLISHED)"
        self.assertTrue(healthy(block, net, miner, tcp, config))
        for obj, key, value in [(block, "headers", 13), (block, "initialblockdownload", True),
                                (net, "networkactive", False), (net, "connections", 0),
                                (miner, "isUsingFallbackStratum", 1), (miner, "stratumURL", "fallback"),
                                (miner, "stratumPort", 3334), (miner, "hashRate", 0)]:
            old = obj[key]
            obj[key] = value
            self.assertFalse(healthy(block, net, miner, tcp, config))
            obj[key] = old
        self.assertFalse(healthy(block, net, miner, "", config))

    def test_rpc_mode_does_not_spawn_cli_or_read_its_config(self):
        with patch("mining_health.rpc_query", return_value={"blocks": 12}) as query, \
                patch("mining_health.run") as cli:
            self.assertEqual(core_query({"rpc": self.rpc, "bitcoin_cli": ["forbidden-cli"]},
                                        "getblockchaininfo"), {"blocks": 12})
        query.assert_called_once_with(self.rpc, "getblockchaininfo")
        cli.assert_not_called()

    def test_rpc_authentication_and_request_are_bounded_and_read_only(self):
        response = io.StringIO(json.dumps({"result": {"blocks": 12}, "error": None}))
        opener = Mock()
        opener.open.return_value = response
        with patch("mining_health.urllib.request.build_opener", return_value=opener) as build:
            self.assertEqual(rpc_query(self.rpc, "getblockchaininfo"), {"blocks": 12})
        request = opener.open.call_args.args[0]
        self.assertEqual(opener.open.call_args.kwargs, {"timeout": 5})
        self.assertEqual(json.loads(request.data)["method"], "getblockchaininfo")
        self.assertEqual(json.loads(request.data)["params"], [])
        self.assertTrue(request.get_header("Authorization").startswith("Basic "))
        self.assertEqual(build.call_args.args[0].proxies, {})
        self.assertIsInstance(build.call_args.args[1], NoRedirect)
        self.assertIsNone(NoRedirect().redirect_request(None, None, 302, "", {}, "https://elsewhere/"))

    def test_rpc_rejects_remote_targets_and_mutating_methods_before_reading_credentials(self):
        for endpoint, method in [("http://example.com/", "getnetworkinfo"),
                ("https://example.com/", "getnetworkinfo"),
                ("http://user:secret@127.0.0.1/", "getnetworkinfo"),
                ("http://127.0.0.1/", "stop")]:
            with self.subTest(endpoint=endpoint, method=method), \
                    patch("mining_health.Path.read_text") as read:
                with self.assertRaises(ValueError):
                    rpc_query(dict(self.rpc, endpoint=endpoint), method)
                read.assert_not_called()

    def test_rpc_errors_never_report_healthy(self):
        for response in [{"result": None, "error": {"code": -1}}, {"result": "invalid", "error": None}]:
            opener = Mock()
            opener.open.return_value = io.StringIO(json.dumps(response))
            with patch("mining_health.urllib.request.build_opener", return_value=opener):
                with self.assertRaises(ValueError):
                    rpc_query(self.rpc, "getnetworkinfo")

    def test_legacy_cli_mode_remains_available(self):
        with patch("mining_health.run", return_value=Mock(stdout='{"blocks":12}')) as run:
            self.assertEqual(core_query({"bitcoin_cli": ["bitcoin-cli"]}, "getblockchaininfo"), {"blocks": 12})
        run.assert_called_once_with(["bitcoin-cli", "getblockchaininfo"], timeout=5)

    def test_failed_probe_reports_stage_without_exception_details(self):
        config = Path(self.temp.name) / "health.toml"
        config.write_text('[mining]\nbitcoin_cli = ["bitcoin-cli"]\n')
        output = io.StringIO()
        with patch("sys.argv", ["mining_health.py", "--config", str(config)]), \
                patch("mining_health.core_query", side_effect=subprocess.TimeoutExpired("secret-command", 5)), \
                contextlib.redirect_stdout(output):
            self.assertEqual(main(), 1)
        self.assertEqual(json.loads(output.getvalue()), {"mining_healthy": False,
            "failed_check": "core_blockchain", "error_type": "TimeoutExpired"})


if __name__ == "__main__":
    unittest.main()
